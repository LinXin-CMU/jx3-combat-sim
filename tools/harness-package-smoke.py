#!/usr/bin/env python3
"""Extract and exercise the Windows package with disposable data and no API key."""
from __future__ import annotations
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import socket
import stat
import subprocess
import sys
import tempfile
import time
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]
# Imported smoke helpers are read-only inputs, not release artifacts.
sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("autonomous_smoke", Path(__file__).with_name("harness-run-smoke.py"))
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)


def require(condition, message):
    # Safety and acceptance checks must remain active under python -O.
    if not condition:
        raise AssertionError(message)


def validate_archive(archive, extracted):
    entries = archive.infolist()
    require(0 < len(entries) <= 2048, "unexpected archive entry count")
    require(sum(item.file_size for item in entries) <= 512 * 1024 * 1024,
            "archive exceeds the runtime payload limit")
    seen = set()
    for item in entries:
        name = PurePosixPath(item.filename)
        require(not name.is_absolute() and name.as_posix() == item.filename and name.parts,
                "invalid archive path")
        require(all(part not in (".", "..") and not part.endswith((".", " "))
                    and not any(c in part for c in '\\:<>"|?*')
                    and not any(ord(c) < 32 for c in part) for part in name.parts),
                "unsafe Windows archive path")
        require(not item.is_dir() and not stat.S_ISLNK(item.external_attr >> 16)
                and not item.flag_bits & 1, "linked, directory, or encrypted archive entry")
        require(item.filename.casefold() not in seen, "duplicate Windows archive path")
        seen.add(item.filename.casefold())
        require(item.file_size <= 256 * 1024 * 1024, "archive entry too large")
        require((extracted / item.filename).resolve().is_relative_to(extracted),
                "archive path leaves the disposable directory")
    require(archive.testzip() is None, "archive CRC validation failed")
    require("manifest.json" in archive.namelist(), "archive manifest missing")
    require(archive.getinfo("manifest.json").file_size <= 1024 * 1024, "manifest too large")
    manifest = json.loads(archive.read("manifest.json"))
    require(manifest.get("schema") == "harness-local-release/v2"
            and manifest.get("manifest_excludes_itself") is True, "wrong manifest schema")
    rows = manifest.get("files")
    require(isinstance(rows, list), "manifest files must be an array")
    expected = {row["path"] for row in rows} | {"manifest.json"}
    require(len(expected) == len(rows) + 1, "duplicate manifest entry")
    require(set(archive.namelist()) == expected, "manifest entry set differs from ZIP")
    require({"backend/jx3-combat-sim.exe", "frontend/index.html", "start.cmd",
             "backend/data/level130/equip.json", "backend/data/level50/equip.json",
             "config/agent.providers.example.toml"} <= expected, "release layout incomplete")
    for row in rows:
        data = archive.read(row["path"])
        require(len(data) == row["bytes"] and hashlib.sha256(data).hexdigest() == row["sha256"],
                "manifest content hash differs from ZIP")
    return len(expected)


def listeners_for_port(output, port):
    """Parse numeric netstat output without depending on localized state names."""
    listeners = set()
    for line in output.splitlines():
        fields = line.split()
        if (len(fields) == 5 and fields[0].upper() == "TCP"
                and fields[2] in {"0.0.0.0:0", "[::]:0"}
                and fields[1].rsplit(":", 1)[-1] == str(port) and fields[-1].isdigit()):
            listeners.add((fields[1], int(fields[-1])))
    return listeners


def require_owned_listener(child, port):
    # Get-NetTCPConnection/WMI can need privileges. netstat -n/-o is sufficient
    # here and prevents a port race from sending writes to somebody else's worker.
    netstat = Path(os.environ["SystemRoot"]) / "System32/netstat.exe"
    result = subprocess.run([str(netstat), "-ano", "-p", "tcp"], capture_output=True,
                            text=True, errors="replace", timeout=10,
                            creationflags=subprocess.CREATE_NO_WINDOW)
    require(result.returncode == 0, "cannot verify packaged server listener ownership")
    listeners = listeners_for_port(result.stdout, port)
    expected = (f"127.0.0.1:{port}", child.pid)
    require(not listeners or listeners == {expected}, "requested port belongs to another process or host")
    return expected in listeners


def stop_owned_process(child):
    if child.poll() is None:
        child.terminate()
    try:
        child.wait(timeout=10)
    except subprocess.TimeoutExpired:
        child.kill()
        child.wait(timeout=10)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("--port", type=int, default=3331)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    require(__debug__, "run without -O: imported scenario smoke helpers use assertions")
    require(os.name == "nt", "this smoke check runs the Windows release executable")
    require(1024 <= args.port <= 65535, "test port must be between 1024 and 65535")
    require(args.report is None or args.report.resolve() != args.archive.resolve(),
            "report path must not overwrite the release archive")
    # Local QA never uses an inherited HTTP proxy, including through the imported
    # HTTP helpers. No remote request is part of this package check.
    urllib.request.install_opener(urllib.request.build_opener(urllib.request.ProxyHandler({})))
    # Fail before launching if another service owns the requested port.
    with socket.socket() as probe:
        probe.setsockopt(socket.SOL_SOCKET, socket.SO_EXCLUSIVEADDRUSE, 1)
        probe.bind(("127.0.0.1", args.port))
    target = (ROOT / "backend/target").resolve()
    with args.archive.open("rb") as source:
        archive_hash = hashlib.file_digest(source, "sha256").hexdigest()
    report = {"schema": "harness-package-smoke/v1", "archive_sha256": archive_hash,
              "checks": [], "api_key_supplied": False}
    with tempfile.TemporaryDirectory(prefix="harness-package-check-", dir=target) as temporary:
        extracted = Path(temporary).resolve()
        require(extracted.is_relative_to(target), "temporary directory must be under backend/target")
        with zipfile.ZipFile(args.archive) as archive:
            file_count = validate_archive(archive, extracted)
            archive.extractall(extracted)
        report["checks"].append({"check": "archive_and_manifest", "files": file_count, "passed": True})
        environment = {key: value for key, value in os.environ.items() if not key.upper().startswith("JX3_")}
        environment.update(JX3_BIND="127.0.0.1", JX3_PORT=str(args.port), JX3_NO_BROWSER="1",
                           JX3_AGENT_CONFIG=str(extracted / "config/agent.providers.example.toml"),
                           JX3_USERDATA_DIR=str(extracted / "userdata"),
                           JX3_ICON_CACHE_DIR=str(extracted / "icon-cache"))
        base = f"http://127.0.0.1:{args.port}"
        client = smoke.Client(base)
        child = subprocess.Popen([str(extracted / "backend/jx3-combat-sim.exe")],
                                 cwd=extracted / "backend", env=environment, stdin=subprocess.DEVNULL,
                                 stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                 creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        try:
            deadline = time.monotonic() + 30
            while True:
                require(child.poll() is None, "packaged server exited during startup")
                if not require_owned_listener(child, args.port):
                    require(time.monotonic() < deadline, "packaged server readiness timeout")
                    time.sleep(0.15)
                    continue
                try:
                    caps = client.ok("/api/harness/capabilities")
                    break
                except (OSError, AssertionError):
                    if time.monotonic() >= deadline:
                        raise AssertionError("packaged server readiness timeout") from None
                    time.sleep(0.15)
            require(child.poll() is None, "packaged server exited after readiness")
            require(caps["workflow"] == "model_selected_experiments", "wrong runtime capabilities")
            profiles = client.ok("/api/agent/providers")["profiles"]
            require(any(p["id"] == "offline" and p["available"] for p in profiles), "offline fixture unavailable")
            require(all(not p["available"] for p in profiles if p["id"] != "offline"),
                    "a live provider unexpectedly inherited credentials")
            for name in ("index.html", "assistant-shell.js", "harness-run.js", "harness-workspace.js", "assistant-shell.css"):
                with urllib.request.urlopen(base + "/" + name, timeout=10) as response:
                    require(response.read() == (extracted / "frontend" / name).read_bytes(), "server used non-package frontend")
            report["checks"].append({"check": "standalone_frontend_and_runtime", "passed": True})
            combinations = {(v, m) for v in ("ShanHaiYuanLiu", "AnYingQianJi", "CangShengZhuShiTest")
                            for m in ("FenShanJin", "TieGuYi")}
            catalog = client.ok("/api/mounts")
            require({(row["version"], row["mount"]) for row in catalog} == combinations
                    and len(catalog) == 6, "package does not expose the complete 3 x 2 matrix")
            matrix = []
            for version, mount in sorted(combinations):
                switched = client.ok("/api/mounts/switch", {"version": version, "mount": mount, "persist": False})
                require(switched.get("ok") is True, f"package could not load {version}/{mount}")
                current = client.ok("/api/mounts/current")
                require((current["version"], current["mount"]) == (version, mount), "version/mount switch was not applied")
                level = 50 if version == "CangShengZhuShiTest" else 130
                require(current["mount_constants"]["level"] == level, "incorrect version level boundary")
                skills, talents = client.ok("/api/skills"), client.ok("/api/talents")
                require(skills and talents, "packaged skill/talent catalog missing")
                defaults = client.ok("/api/mounts/defaults")
                require(isinstance(defaults, dict) and defaults.get("attributes"), "packaged mount defaults missing")
                equipment = client.ok("/api/equip/meta")
                require(equipment["total_items"] > 0, "processed equipment catalog missing")
                matrix.append({"version": version, "mount": mount, "level": level,
                               "skills": len(skills), "equipment_items": equipment["total_items"]})
            report["checks"].append({"check": "three_versions_two_mounts", "matrix": matrix, "passed": True})
            restored = client.ok("/api/mounts/switch", {"version": "AnYingQianJi", "mount": "FenShanJin", "persist": False})
            require(restored.get("ok") is True, "could not restore the scenario fixture version")
            for kind in ("macro", "rotation", "equipment"):
                row = smoke.run_case(client, smoke.payload(kind, "offline"), kind, False)
                report["checks"].append({"check": kind, "passed": True, "simulations": row["simulations"]})
            require((extracted / "userdata/harness_runs/v2").is_dir(), "package did not use its isolated userdata")
        finally:
            stop_owned_process(child)
    report["temporary_process_and_userdata_removed"] = True
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False))


if __name__ == "__main__":
    main()
