#!/usr/bin/env python3
"""Isolated Harness v2 lifecycle evidence, using only loopback and a mock model.

The default target is the disposable QA worker on port 3320. The optional restart
checks both its recorded PID/executable and listening port before stopping it.
Never targets the live-model worker on 3319 or an existing custom provider.
"""
from __future__ import annotations

import argparse
import concurrent.futures
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlsplit
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[1]
QA = ROOT / "backend/target/harness-v2-qa"
USERDATA = QA / "worker-3320/userdata"
KEY = "harness-lifecycle-fixture-only"
LABEL = "isolated-harness-lifecycle-fixture"
HEADERS = {"X-JX3-Provider-Settings": "1"}
spec = importlib.util.spec_from_file_location("harness_http", Path(__file__).with_name("harness-smoke.py"))
http = importlib.util.module_from_spec(spec)
spec.loader.exec_module(http)


def require(value, message):
    if not value:
        raise AssertionError(message)


class Laboratory:
    def __init__(self):
        self.lock = threading.Lock()
        self.mode = "hold"
        self.calls = 0
        self.release = threading.Event()

    def mode_as(self, mode):
        with self.lock:
            self.release.set()
            self.release = threading.Event()
            self.mode, self.calls = mode, 0

    def request(self):
        with self.lock:
            self.calls += 1
            mode, count, release = self.mode, self.calls, self.release
        if mode == "hold" or mode == "evaluate_hold" and count > 1:
            release.wait(90)
            return "finish", {"summary": "已停止的离线连接。"}
        if mode in {"evaluate_hold", "evaluate_finish"} and count == 1:
            return "experiment", {"kind": "evaluate", "macro_text": "/cast 盾刀", "hypothesis": "用真实模拟建立可恢复候选证据。"}
        return "finish", {"artifact_id": "evidence-1", "summary": "仅交付已模拟的当前场景候选。"}

    def wait_calls(self, minimum):
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            with self.lock:
                if self.calls >= minimum:
                    return
            time.sleep(.03)
        raise AssertionError("mock provider did not reach the expected lifecycle boundary")


def mock_server(lab):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            data = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            require(self.headers.get("Authorization") == "Bearer " + KEY, "fixture authentication mismatch")
            require(data["model"] == "harness-lifecycle-mock", "unexpected model reached fixture")
            name, arguments = lab.request()
            response = {"choices": [{"finish_reason": "tool_calls", "message": {
                "role": "assistant", "content": "执行可验证的生命周期测试。",
                "reasoning_content": "PRIVATE_LIFECYCLE_REASONING_MUST_NOT_PERSIST",
                "tool_calls": [{"id": "fixture-" + str(time.time_ns()), "type": "function",
                                "function": {"name": name, "arguments": json.dumps(arguments, ensure_ascii=False)}}],
            }}], "usage": {"prompt_tokens": 10, "completion_tokens": 4, "total_tokens": 14}}
            encoded = json.dumps(response, ensure_ascii=False).encode()
            try:
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(encoded)))
                self.end_headers()
                self.wfile.write(encoded)
            except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
                pass  # Expected when cancellation drops an in-flight request.

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.daemon_threads = True
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def request_body():
    sim = copy.deepcopy(http.payload()["simulation"])
    sim.update(sequence=["__macro__"] * 60, macro_text="/cast 盾刀", macro_duration=10)
    return {"goal": "隔离生命周期测试：保留当前场景证据。", "provider_profile": "user-custom",
            "simulation": sim, "version": "AnYingQianJi", "mount": "FenShanJin",
            "constraints": {"duration_seconds": 10, "allowed_skills": ["盾刀"]},
            "budget": {"max_model_calls": 12, "max_simulations": 16, "wall_time_ms": 180000,
                       "max_output_tokens": 1024, "max_total_tokens": 12000}}


def wait_run(client, created):
    deadline, sequence = time.monotonic() + 25, 0
    while time.monotonic() < deadline:
        state = client.ok(created["status_url"])
        require(state["run_id"] == created["run_id"], "run identity changed")
        require(state["sequence"] >= sequence, "status sequence regressed")
        sequence = state["sequence"]
        require(state["usage"]["simulations"] <= state["budget"]["max_simulations"], "simulation budget exceeded")
        if not state["running"]:
            return state
        time.sleep(.025)
    raise AssertionError("run did not reach terminal state")


def sse_snapshot(client, created, terminal, last_event_id=None):
    headers = {"Accept": "text/event-stream"}
    if last_event_id is not None:
        headers["Last-Event-ID"] = str(last_event_id)
    with urlopen(Request(client.base_url + created["events_url"], headers=headers), timeout=10) as response:
        require("text/event-stream" in response.headers.get("Content-Type", ""), "missing SSE content type")
        frame = {}
        for raw in response:
            line = raw.decode().rstrip("\r\n")
            if not line and "data" in frame:
                state = json.loads(frame["data"])
                require(state["run_id"] == created["run_id"], "SSE crossed run identity")
                require(int(frame["id"]) == state["sequence"], "SSE id differs from snapshot sequence")
                require(frame["event"] == ("completed" if terminal else "progress"), "SSE terminal/progress mismatch")
                require(state["running"] is not terminal, "SSE running flag mismatch")
                return state
            if ":" in line and not line.startswith(":"):
                key, value = line.split(":", 1)
                frame[key] = value.lstrip()
    raise AssertionError("SSE ended without a complete snapshot")


def error(client, path, body, expected, code):
    status, result = client.request(path, body)
    require(status == expected and result.get("error", {}).get("code") == code,
            f"unexpected rejection for {path}: {status}, {result}")


def configure(client, server):
    body = {"label": LABEL, "base_url": f"http://127.0.0.1:{server.server_port}/v1",
            "model": "harness-lifecycle-mock", "protocol": "chat_completions", "api_key": KEY}
    status, view = client.request("/api/agent/providers/custom", body, method="PUT", headers=HEADERS)
    require(status == 200 and view.get("has_key"), "could not configure isolated mock provider")


def restart_qa(client):
    # The known executable is shared with 3319; checking the listening owner is
    # therefore mandatory in addition to checking PID and exact executable path.
    def quote(path):
        return "'" + str(path).replace("'", "''") + "'"
    pid_file = QA / "worker-3320/pid.txt"
    previous_pid = pid_file.read_text(encoding="utf-8-sig").strip()
    script = fr"""$ErrorActionPreference='Stop'
$qaWorkerPid=[int](Get-Content -LiteralPath {quote(QA / 'worker-3320/pid.txt')})
$qaWorker=Get-Process -Id $qaWorkerPid -ErrorAction Stop
if ($qaWorker.Path -ne {quote(QA / 'runtime/jx3-combat-sim.exe')}) {{ throw 'QA executable mismatch' }}
$qaListener=(& netstat.exe -ano -p TCP) | Where-Object {{ $_ -match ('^\s*TCP\s+\S+:3320\s+\S+\s+LISTENING\s+' + $qaWorkerPid + '\s*$') }}
if (-not $qaListener) {{ throw 'QA listening owner mismatch' }}
Stop-Process -Id $qaWorkerPid
Wait-Process -Id $qaWorkerPid -ErrorAction SilentlyContinue
& {quote(QA / 'start.ps1')} -Port 3320
"""
    # Start-Process output redirection can keep its PowerShell supervisor alive.
    # Observe the new worker PID and HTTP readiness, not supervisor termination.
    supervisor = subprocess.Popen(["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", script],
                                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                  creationflags=subprocess.CREATE_NO_WINDOW)
    deadline = time.monotonic() + 50
    while time.monotonic() < deadline:
        require(supervisor.poll() in (None, 0), "QA restart guard failed; worker ownership was not changed blindly")
        try:
            current_pid = pid_file.read_text(encoding="utf-8-sig").strip()
            if current_pid != previous_pid and client.request("/api/harness/capabilities")[0] == 200:
                return
        except AssertionError:
            pass
        time.sleep(.2)
    raise AssertionError("isolated QA worker did not return after restart")


def legacy_payload(sim):
    text = "/cast [rage>30] 盾刀"
    return {"macro_text": text, "base_loop": {"version": 1, "target": sim["target"], "talents": {}, "recipes": {},
            "sequence": [{"type": "macro", "count": 60}], "macro": {"mode": "general", "general": text}},
            "attrs": sim["attributes"], "target": sim["target"], "duration": 600,
            "haste_level": sim["haste_level"], "initial_rage": 50,
            "ga_params": {"pop_size": 16, "generations": 128, "top_n": 1}, "samples_per_eval": 1}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--backend", default="http://127.0.0.1:3320")
    parser.add_argument("--restart", action="store_true", help="restart only the checked disposable QA worker")
    parser.add_argument("--report", type=Path, default=QA / "lifecycle-report.json")
    args = parser.parse_args()
    address = urlsplit(args.backend)
    require(address.hostname in {"localhost", "127.0.0.1"} and address.port == 3320,
            "this evaluator exclusively owns disposable QA port 3320")
    client, lab = http.Client(args.backend), Laboratory()
    original = client.ok("/api/mounts/current")
    require(not any(r["running"] for r in client.ok("/api/harness/runs")["runs"]), "pre-existing run is active")
    require(not any(r["running"] for r in client.ok("/api/harness/jobs")["jobs"]), "pre-existing compiler job is active")
    require(not client.ok("/api/optimizer/status")["running"], "pre-existing optimizer is active")
    status, custom = client.request("/api/agent/providers/custom", headers=HEADERS)
    require(status == 200 and custom["config"] is None, "refusing to overwrite an existing custom provider")
    server = mock_server(lab)
    created_runs, rows, legacy_owned = [], [], False
    before = {p: hashlib.sha256(p.read_bytes()).hexdigest() for p in USERDATA.rglob("*") if p.is_file()}
    try:
        client.ok("/api/mounts/switch", {"version": "AnYingQianJi", "mount": "FenShanJin", "persist": False})
        configure(client, server)
        body = request_body()
        lab.mode_as("evaluate_hold")
        created = client.ok("/api/harness/runs", body, 202)
        created_runs.append(created)
        lab.wait_calls(2)
        initial = client.ok(created["status_url"])
        require(initial["running"] and len(initial["artifacts"]) == 1, "mock hold did not retain completed evidence")
        live = sse_snapshot(client, created, False)
        error(client, "/api/harness/runs", body, 409, "compute_busy")
        error(client, "/api/harness/jobs", http.payload(), 409, "job_conflict")
        legacy_paths = ["/api/optimizer/start", "/api/rl/train/start", "/api/rl/analyze/start", "/api/rl/pretrain/start", "/api/equip/auto_optimize"]
        with concurrent.futures.ThreadPoolExecutor(max_workers=len(legacy_paths)) as pool:
            list(pool.map(lambda path: error(client, path, {}, 409, "compute_busy"), legacy_paths))
        require(client.ok(created["cancel_url"], {})["accepted"], "active cancellation was refused")
        stopped = wait_run(client, created)
        require(stopped["status"] == "cancelled" and stopped["resumable"], "cancelled run cannot resume its unused budget")
        require(stopped["usage"]["simulations"] == 2 and len(stopped["artifacts"]) == 1, "cancel discarded completed evidence")
        require(not client.ok(created["cancel_url"], {})["accepted"], "terminal cancellation is not idempotent")
        terminal = sse_snapshot(client, created, True, live["sequence"])
        require(terminal["sequence"] >= live["sequence"], "reconnected SSE regressed")
        rows.append({"case": "admission_cancel_sse", "blocked_legacy_endpoints": len(legacy_paths), "simulations": 2})
        print("PASS admission, live/terminal SSE, cancellation evidence", flush=True)

        lab.mode_as("finish")
        client.ok(created["resume_url"], {}, 202)
        completed = wait_run(client, created)
        require(completed["status"] == "completed" and completed["usage"]["simulations"] == 2, "resume reran completed experiments")
        error(client, created["resume_url"], {}, 409, "not_resumable")
        bundle = client.ok(created["artifacts_url"])
        artifact = bundle["artifacts"][0]
        require(artifact["result"]["best"]["verified"], "mock's real candidate was not verified")
        selected = {"artifact_id": artifact["id"], "expected_scenario_hash": completed["scenario_hash"]}
        error(client, created["status_url"] + "/apply", {**selected, "expected_scenario_hash": "stale"}, 409, "scene_changed")
        transaction = client.ok(created["status_url"] + "/apply", selected)
        reverse = client.ok(created["status_url"] + "/undo", {"transaction_id": transaction["transaction_id"]})
        require(reverse["after"] == transaction["before"], "undo does not restore the complete original scenario")
        replay = client.ok("/api/simulate", transaction["after"]["simulation"])
        require(str(replay["fingerprint"]) == artifact["result"]["best"]["fingerprint"], "applied artifact replay differs")
        client.ok("/api/mounts/switch", {"version": "AnYingQianJi", "mount": "TieGuYi", "persist": False})
        error(client, created["status_url"] + "/apply", selected, 409, "runtime_mismatch")
        client.ok("/api/mounts/switch", {"version": "AnYingQianJi", "mount": "FenShanJin", "persist": False})
        rows.append({"case": "resume_apply_undo", "simulations_unchanged": 2, "stale_scene_rejected": True, "stale_mount_rejected": True})
        print("PASS resume without replay, stale apply, typed undo and fingerprint", flush=True)

        lab.mode_as("hold")
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
            outcomes = list(pool.map(lambda _: client.request("/api/harness/runs", body), range(2)))
        require(sorted(code for code, _ in outcomes) == [202, 409], "simultaneous starts both acquired computation")
        winner = next(data for code, data in outcomes if code == 202)
        created_runs.append(winner)
        client.ok(winner["cancel_url"], {})
        require(wait_run(client, winner)["status"] == "cancelled", "concurrent winner did not stop")
        rows.append({"case": "simultaneous_new_runs", "statuses": [202, 409]})

        observed_legacy_rejection = False
        for _ in range(3):
            started = client.ok("/api/optimizer/start", legacy_payload(body["simulation"]))
            legacy_owned = True
            require(started.get("run_id"), "legacy optimizer was not actually started")
            status, admission = client.request("/api/harness/runs", body)
            if status == 202:
                # A very fast GA may finish before the following request reaches
                # admission. Own and cancel our newly admitted run before retrying.
                created_runs.append(admission)
                client.ok(admission["cancel_url"], {})
                wait_run(client, admission)
            else:
                require(status == 409 and admission.get("error", {}).get("code") == "compute_busy", "unexpected legacy admission rejection")
                observed_legacy_rejection = True
            client.ok("/api/optimizer/stop", {})
            deadline = time.monotonic() + 25
            while client.ok("/api/optimizer/status")["running"] and time.monotonic() < deadline:
                time.sleep(.03)
            require(not client.ok("/api/optimizer/status")["running"], "owned legacy optimizer did not release admission")
            legacy_owned = False
            if observed_legacy_rejection:
                break
        require(observed_legacy_rejection, "could not observe the bounded legacy-to-new admission window")
        rows.append({"case": "legacy_optimizer_blocks_new_run", "status": 409})
        print("PASS simultaneous new runs and legacy-to-new admission", flush=True)

        if args.restart:
            lab.mode_as("evaluate_hold")
            interrupted = client.ok("/api/harness/runs", body, 202)
            created_runs.append(interrupted)
            lab.wait_calls(2)
            preserved = client.ok(interrupted["artifacts_url"])
            require(len(preserved["artifacts"]) == 1, "restart fixture has no completed evidence")
            restart_qa(client)
            recovered = client.ok(interrupted["status_url"])
            require(recovered["status"] == "interrupted" and recovered["resumable"], "running checkpoint was not recoverable after restart")
            recovered_bundle = client.ok(interrupted["artifacts_url"])
            require(recovered_bundle["artifacts"] == preserved["artifacts"], "restart changed immutable artifact evidence")
            require(client.ok(created["artifacts_url"])["artifacts"] == bundle["artifacts"], "completed run evidence changed after restart")
            _, custom = client.request("/api/agent/providers/custom", headers=HEADERS)
            require(custom["has_key"] is False, "custom credential survived worker restart")
            configure(client, server)
            lab.mode_as("finish")
            client.ok(interrupted["resume_url"], {}, 202)
            resumed = wait_run(client, interrupted)
            require(resumed["status"] == "completed" and resumed["usage"]["simulations"] == preserved["usage"]["simulations"], "restart recovery reran prior evidence")
            reverse_again = client.ok(created["status_url"] + "/undo", {"transaction_id": transaction["transaction_id"]})
            require(reverse_again["after"] == transaction["before"], "durable undo record changed after restart")
            rows.append({"case": "worker_restart_recovery", "artifact_count": 1, "simulations": resumed["usage"]["simulations"], "credential_restored": False})
            print("PASS worker restart, immutable evidence, credential boundary and resume", flush=True)

        for run in created_runs:
            stored = client.ok(run["artifacts_url"])
            serialized = json.dumps(stored)
            require(KEY not in serialized and "PRIVATE_LIFECYCLE_REASONING" not in serialized and "reasoning_content" not in serialized, "private provider state leaked to bundle")
            run_dir = USERDATA / "harness_runs/v2" / run["run_id"]
            require(any(run_dir.glob("checkpoint-*.json")), "run checkpoint not persisted in isolated userdata")
            for path in run_dir.glob("*.json"):
                content = path.read_text(encoding="utf-8")
                require(KEY not in content and "PRIVATE_LIFECYCLE_REASONING" not in content, "private provider state leaked to disk")
    finally:
        lab.release.set()
        for run in created_runs:
            try:
                client.ok(run["cancel_url"], {})
            except Exception:
                pass
        if legacy_owned:
            client.ok("/api/optimizer/stop", {})
        try:
            client.request("/api/agent/providers/custom", method="DELETE", headers=HEADERS)
            client.ok("/api/mounts/switch", {"version": original["version"], "mount": original["mount"], "persist": False})
        finally:
            server.shutdown()
            server.server_close()
    for path, digest in before.items():
        require(path.is_file() and hashlib.sha256(path.read_bytes()).hexdigest() == digest, "a pre-existing QA userdata file was modified")
    report = {"schema": "harness-run-lifecycle/v1", "backend": args.backend, "results": rows,
              "run_ids": [run["run_id"] for run in created_runs], "existing_qa_files_unchanged": len(before), "real_provider_calls": 0}
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"PASS {len(rows)} lifecycle boundaries; no external provider calls; report: {args.report}", flush=True)


def recovery_only():
    """Exercise durable recovery without repeating already-recorded admission tests."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--restart-only", action="store_true")
    parser.add_argument("--backend", default="http://127.0.0.1:3320")
    parser.add_argument("--report", type=Path, default=QA / "lifecycle-recovery-report.json")
    args = parser.parse_args()
    address = urlsplit(args.backend)
    require(address.hostname in {"localhost", "127.0.0.1"} and address.port == 3320,
            "this evaluator exclusively owns disposable QA port 3320")
    client, lab = http.Client(args.backend), Laboratory()
    original = client.ok("/api/mounts/current")
    require(not any(r["running"] for r in client.ok("/api/harness/runs")["runs"]), "pre-existing run is active")
    require(not any(r["running"] for r in client.ok("/api/harness/jobs")["jobs"]), "pre-existing compiler job is active")
    require(not client.ok("/api/optimizer/status")["running"], "pre-existing optimizer is active")
    status, custom = client.request("/api/agent/providers/custom", headers=HEADERS)
    require(status == 200 and custom["config"] is None, "refusing to overwrite an existing custom provider")
    before = {p: hashlib.sha256(p.read_bytes()).hexdigest() for p in USERDATA.rglob("*") if p.is_file()}
    created_runs, rows = [], []
    server = mock_server(lab)
    try:
        client.ok("/api/mounts/switch", {"version": "AnYingQianJi", "mount": "FenShanJin", "persist": False})
        configure(client, server)
        body = request_body()
        lab.mode_as("evaluate_finish")
        completed_run = client.ok("/api/harness/runs", body, 202)
        created_runs.append(completed_run)
        completed = wait_run(client, completed_run)
        require(completed["status"] == "completed", "durable completed fixture failed")
        completed_bundle = client.ok(completed_run["artifacts_url"])
        artifact = completed_bundle["artifacts"][0]
        require(artifact["result"]["best"]["verified"], "completed fixture has no verified candidate")
        transaction = client.ok(completed_run["status_url"] + "/apply", {
            "artifact_id": artifact["id"], "expected_scenario_hash": completed["scenario_hash"]})
        lab.mode_as("evaluate_hold")
        interrupted = client.ok("/api/harness/runs", body, 202)
        created_runs.append(interrupted)
        lab.wait_calls(2)
        preserved = client.ok(interrupted["artifacts_url"])
        require(len(preserved["artifacts"]) == 1 and preserved["usage"]["simulations"] == 2,
                "running fixture must have a completed real experiment")
        print("READY completed and running evidence recorded; restarting checked QA worker", flush=True)
        restart_qa(client)
        recovered = client.ok(interrupted["status_url"])
        require(recovered["status"] == "interrupted" and recovered["resumable"], "running checkpoint did not recover as resumable")
        require(client.ok(interrupted["artifacts_url"])["artifacts"] == preserved["artifacts"], "restart changed immutable experiment evidence")
        require(client.ok(completed_run["status_url"])["status"] == "completed", "completed run lost terminal state")
        require(client.ok(completed_run["artifacts_url"])["artifacts"] == completed_bundle["artifacts"], "completed evidence changed after restart")
        _, custom = client.request("/api/agent/providers/custom", headers=HEADERS)
        require(custom["has_key"] is False, "custom credential survived restart")
        configure(client, server)
        lab.mode_as("finish")
        client.ok(interrupted["resume_url"], {}, 202)
        resumed = wait_run(client, interrupted)
        require(resumed["status"] == "completed", "recovered run could not finish from retained evidence")
        require(resumed["usage"]["simulations"] == preserved["usage"]["simulations"], "resume charged retained experiment again")
        reverse = client.ok(completed_run["status_url"] + "/undo", {"transaction_id": transaction["transaction_id"]})
        require(reverse["after"] == transaction["before"], "durable undo did not restore complete original scene")
        for run in created_runs:
            serialized = json.dumps(client.ok(run["artifacts_url"]))
            require(KEY not in serialized and "PRIVATE_LIFECYCLE_REASONING" not in serialized and "reasoning_content" not in serialized,
                    "private provider state leaked to bundle")
            run_dir = USERDATA / "harness_runs/v2" / run["run_id"]
            require(any(run_dir.glob("checkpoint-*.json")), "isolated run checkpoint missing")
            for path in run_dir.glob("*.json"):
                content = path.read_text(encoding="utf-8")
                require(KEY not in content and "PRIVATE_LIFECYCLE_REASONING" not in content, "private provider state leaked to disk")
        rows.append({"case": "worker_restart_recovery", "artifact_count": 1, "simulations": resumed["usage"]["simulations"],
                     "completed_evidence_unchanged": True, "credential_restored": False, "durable_undo": True})
    finally:
        lab.release.set()
        for run in created_runs:
            try:
                client.ok(run["cancel_url"], {})
            except Exception:
                pass
        try:
            status, _ = client.request("/api/agent/providers/custom", method="DELETE", headers=HEADERS)
            require(status == 200, "fixture provider cleanup failed")
            client.ok("/api/mounts/switch", {"version": original["version"], "mount": original["mount"], "persist": False})
        finally:
            server.shutdown()
            server.server_close()
    for path, digest in before.items():
        require(path.is_file() and hashlib.sha256(path.read_bytes()).hexdigest() == digest, "a pre-existing QA userdata file was modified")
    _, custom = client.request("/api/agent/providers/custom", headers=HEADERS)
    require(custom["config"] is None, "temporary custom provider remains installed")
    require(not any(r["running"] for r in client.ok("/api/harness/runs")["runs"]), "fixture left an active run")
    report = {"schema": "harness-run-lifecycle/v1", "backend": args.backend, "results": rows,
              "run_ids": [run["run_id"] for run in created_runs], "existing_qa_files_unchanged": len(before), "real_provider_calls": 0}
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"PASS worker restart, immutable evidence, credential boundary, resume and durable undo; report: {args.report}", flush=True)


if __name__ == "__main__":
    if "--restart-only" in sys.argv:
        recovery_only()
    else:
        main()
