#!/usr/bin/env python3
"""HTTP-only harness smoke checks for an isolated local simulator worker.

Uses only the standard library. Writes no files and never creates Agent sessions.
Runtime switches always use persist=false and are restored in finally. Legacy
start probes deliberately contain invalid JSON schemas, so a missed busy window
cannot accidentally start an optimizer/training job or write an archive.
"""
from __future__ import annotations

import argparse
import concurrent.futures
import copy
import http.client
import json
import math
from pathlib import Path
import sys
import time
import urllib.error
import urllib.request


ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "backend/tests/agent_diagnostic_eval/scenario.json"


def require(condition, message):
    if not condition:
        raise AssertionError(message)


class Client:
    def __init__(self, base_url):
        self.base_url = base_url.rstrip("/")
        self.created = set()

    def request(self, path, payload=None, method=None, headers=None):
        data = None if payload is None else json.dumps(payload, ensure_ascii=False, allow_nan=False).encode("utf-8")
        request = urllib.request.Request(
            self.base_url + path, data=data, method=method or ("POST" if data is not None else "GET"),
            headers={"Content-Type": "application/json; charset=utf-8", **(headers or {})},
        )
        try:
            response = urllib.request.urlopen(request, timeout=45)
        except urllib.error.HTTPError as error:
            response = error
        except (urllib.error.URLError, OSError, http.client.HTTPException) as error:
            raise AssertionError(
                f"{request.get_method()} {path}: transport failure {type(error).__name__}: {error}"
            ) from error
        with response:
            try:
                raw = response.read().decode("utf-8")
            except (urllib.error.URLError, OSError, http.client.HTTPException) as error:
                raise AssertionError(
                    f"{request.get_method()} {path}: response-read failure {type(error).__name__}: {error}"
                ) from error
            try:
                body = json.loads(raw)
            except json.JSONDecodeError as error:
                raise AssertionError(f"{path}: HTTP {response.status} returned non-JSON") from error
            if path == "/api/harness/jobs" and response.status == 202:
                self.created.add(body["job_id"])
            return response.status, body

    def ok(self, path, payload=None, expected=200):
        status, body = self.request(path, payload)
        require(status == expected, f"{path}: expected HTTP {expected}, got {status}: {body}")
        return body

    def create(self, payload):
        created = self.ok("/api/harness/jobs", payload, 202)
        require(created.get("schema_version") == "harness-job-created/v1", "wrong create schema")
        for field in ("scenario_hash", "experiment_hash"):
            value = created.get(field, "")
            require(len(value) == 64 and all(c in "0123456789abcdef" for c in value), f"invalid {field}")
        return created

    def wait(self, created, timeout=130):
        deadline = time.monotonic() + timeout
        previous_sequence = 0
        while time.monotonic() < deadline:
            status = self.ok(created["status_url"])
            require(status["job_id"] == created["job_id"], "status job identity changed")
            require(status["scenario_hash"] == created["scenario_hash"], "scenario identity changed")
            require(status["experiment_hash"] == created["experiment_hash"], "experiment identity changed")
            require(status["sequence"] >= previous_sequence, "status sequence moved backwards")
            require(status["simulations"] <= status["max_simulations"], "simulation budget exceeded")
            previous_sequence = status["sequence"]
            if not status["running"]:
                return status
            time.sleep(0.02)
        raise AssertionError("harness did not terminate within the HTTP smoke deadline")

    def terminal_sse(self, created, last_event_id=None):
        headers = {"Accept": "text/event-stream"}
        if last_event_id is not None:
            headers["Last-Event-ID"] = str(last_event_id)
        request = urllib.request.Request(self.base_url + created["events_url"], headers=headers)
        with urllib.request.urlopen(request, timeout=10) as response:
            require("text/event-stream" in response.headers.get("Content-Type", ""), "SSE content type missing")
            frame = {}
            for raw in response:
                line = raw.decode("utf-8").rstrip("\r\n")
                if not line and "data" in frame:
                    data = json.loads(frame["data"])
                    require(frame.get("event") == "completed" and not data["running"], "late SSE did not return terminal snapshot")
                    require(data["job_id"] == created["job_id"], "SSE job identity mismatch")
                    require(int(frame["id"]) == data["sequence"], "SSE sequence mismatch")
                    return data
                if line.startswith(":"):
                    continue
                if ":" in line:
                    field, value = line.split(":", 1)
                    frame[field] = value.lstrip(" ")
        raise AssertionError("terminal SSE ended without a snapshot")


def payload():
    simulation = json.loads(FIXTURE.read_text(encoding="utf-8"))["simulation"]
    simulation.update(sequence=["盾刀"] * 6, macro_text=None, macro_duration=None,
                      network_delay=0, talents=[], recipes=[], equipment={},
                      initial_rage=50, dunya_reset_seed=42, pre_releases=[],
                      pauses=[], channel_ticks={}, timing_offsets={}, qijin_buffs={})
    return {"version": "AnYingQianJi", "mount": "FenShanJin", "simulation": simulation,
            "max_simulations": 32, "wall_time_ms": 60000, "max_rounds": 3,
            "max_pages": 2, "time_tolerance": 1 / 16}


def assert_error(client, path, payload, status, code):
    actual, body = client.request(path, payload)
    require(actual == status, f"{path}: expected error {status}, got {actual}: {body}")
    require(body.get("schema_version") == "harness-error/v1", "wrong error schema")
    require(body.get("error", {}).get("code") == code, f"wrong error code: {body}")


def replay_artifact(client, created, terminal):
    artifact = client.ok(created["artifacts_url"])
    require(artifact.get("schema_version") == "harness-artifacts/v1", "wrong artifact schema")
    require(artifact["experiment_hash"] == created["experiment_hash"], "artifact experiment identity changed")
    require(artifact["scenario_hash"] == created["scenario_hash"], "artifact scenario identity changed")
    require(len(artifact["runtime_hash"]) == 64, "runtime fingerprint missing")
    require(artifact["storage"] == "worker_memory" and not artifact["game_verified"], "artifact scope overstated")
    require(artifact["result"] == terminal["result"], "artifact and terminal result differ")
    best = artifact["result"]["best"]
    require(best and best["verified"], "missing fully tested best candidate")
    simulation = copy.deepcopy(artifact["scenario"]["simulation"])
    duration = artifact["result"]["window_seconds"]
    simulation.update(sequence=["__macro__"] * (math.ceil(duration / 0.25) + 20),
                      macro_text=best["macro_text"], macro_duration=duration,
                      channel_ticks={}, timing_offsets={}, qijin_buffs={}, lite=False, lite_keep_timeline=False)
    replay = client.ok("/api/simulate", simulation)
    require(str(replay["fingerprint"]) == best["fingerprint"], "artifact replay fingerprint differs")
    for field in ("dps", "total_damage", "fight_time"):
        require(math.isclose(replay[field], best["metrics"][field], rel_tol=1e-12, abs_tol=1e-8), f"artifact replay {field} differs")
    return artifact


def cancel_and_conflicts(client, base):
    # A long but valid target makes admission conflicts observable even in release.
    request = copy.deepcopy(base)
    request["simulation"]["sequence"] = ["盾刀"] * 800
    request.update(initial_macro="/cast [rage>100] 盾刀", max_simulations=256, max_rounds=12)
    legacy_paths = ["/api/optimizer/start", "/api/rl/train/start", "/api/rl/analyze/start",
                    "/api/rl/pretrain/start", "/api/equip/auto_optimize"]
    observed_legacy = set()
    observed_job_conflict = False
    observed_cancel = False
    for _ in range(4):
        created = client.create(request)
        with concurrent.futures.ThreadPoolExecutor(max_workers=7) as executor:
            futures = {executor.submit(client.request, path, {}): path for path in legacy_paths}
            duplicate = executor.submit(client.request, "/api/harness/jobs", request)
            for future, path in futures.items():
                status, body = future.result()
                if status == 409:
                    require(body.get("error", {}).get("code") == "compute_busy", f"wrong legacy conflict code: {body}")
                    observed_legacy.add(path)
                else:
                    # If the job already ended, malformed legacy schemas must still reject.
                    require(status in (400, 422), f"legacy probe unexpectedly started work: {path}: {status}")
            status, body = duplicate.result()
            if status == 409:
                require(body.get("error", {}).get("code") == "job_conflict", "wrong simultaneous job error")
                observed_job_conflict = True
            elif status == 202:
                # Task finished before the second admission; own both tasks for cleanup.
                client.ok(body["cancel_url"], {})
                client.wait(body)
            else:
                raise AssertionError(f"unexpected duplicate-create status {status}: {body}")
        cancellation = client.ok(created["cancel_url"], {})
        terminal = client.wait(created)
        if cancellation["accepted"]:
            require(terminal["status"] == "cancelled", "accepted cancellation was reported as success")
            observed_cancel = True
        # The terminal lease must be released; a second cancel is idempotent.
        require(client.ok(created["cancel_url"], {})["already_terminal"], "terminal cancel is not idempotent")
        if observed_cancel and observed_job_conflict and len(observed_legacy) == len(legacy_paths):
            return
    require(observed_job_conflict, "could not observe simultaneous harness 409")
    require(observed_legacy == set(legacy_paths), f"missing legacy admission conflicts: {set(legacy_paths) - observed_legacy}")
    require(observed_cancel, "could not observe accepted cancellation")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-url", required=True, help="isolated worker base URL, e.g. http://127.0.0.1:3028")
    args = parser.parse_args()
    client = Client(args.base_url)
    original = client.ok("/api/mounts/current")
    existing = client.ok("/api/harness/jobs")
    require(not any(job["running"] for job in existing["jobs"]), "refusing to disturb a pre-existing harness job")
    restored = False
    checks = 0
    try:
        switched = client.ok("/api/mounts/switch", {"version": "AnYingQianJi", "mount": "FenShanJin", "persist": False})
        require(switched.get("ok"), "runtime switch failed")
        base = payload()
        baseline = client.ok("/api/simulate", base["simulation"])
        require(not baseline["skipped"] and baseline["fight_time"] > 0, "smoke target is not a legal manual axis")
        checks += 1
        assert_error(client, "/api/harness/jobs", {"unknown_field": True}, 400, "invalid_request")
        assert_error(client, "/api/harness/jobs/does-not-exist", None, 404, "job_not_found")
        mismatch = copy.deepcopy(base)
        mismatch["version"] = "ShanHaiYuanLiu"
        assert_error(client, "/api/harness/jobs", mismatch, 409, "runtime_mismatch")
        checks += 3

        # Validation precedes runtime matching. Keep a mismatched version as a
        # second barrier: an older server missing one bound will return 409,
        # never enter the simulator with an unsafe event schedule.
        bounded_inputs = [
            ("tiny boss interval", {"boss_attack_interval": 1e-300}),
            ("oversized manual duration", {"macro_duration": 1e12}),
            ("tiny nested team period", {"team_buffs": [{
                "key": "test", "enabled": True, "stacks": 1,
                "first_release": 0, "duration": 10, "period": 1e-300,
                "release_times": [0, 30],
            }]}),
        ]
        for label, fields in bounded_inputs:
            invalid = copy.deepcopy(mismatch)
            invalid["simulation"].update(fields)
            try:
                assert_error(client, "/api/harness/jobs", invalid, 400, "invalid_request")
            except AssertionError as error:
                raise AssertionError(f"pre-simulation bound ({label}): {error}") from error
            checks += 1

        # No seed macro exercises the actual sequence-to-macro generator.
        generated = client.create(base)
        generated_terminal = client.wait(generated)
        require(generated_terminal["status"] == "completed", f"generator failed: {generated_terminal.get('error')}")
        require(generated_terminal["result"]["best"]["verified"], "generator candidate was not replayed")
        replay_artifact(client, generated, generated_terminal)
        checks += 2
        first_stream = client.terminal_sse(generated)
        second_stream = client.terminal_sse(generated, first_stream["sequence"])
        require(first_stream == second_stream, "terminal SSE reconnect is not stable")
        checks += 1

        repeated = client.create(base)
        repeated_terminal = client.wait(repeated)
        require(repeated["scenario_hash"] == generated["scenario_hash"], "same scenario hash is unstable")
        require(repeated["experiment_hash"] == generated["experiment_hash"], "same experiment hash is unstable")
        require(repeated_terminal["result"]["best"]["fingerprint"] == generated_terminal["result"]["best"]["fingerprint"], "deterministic candidate replay differs")
        checks += 1

        wrong = copy.deepcopy(base)
        wrong["initial_macro"] = "/cast [rage>100] 盾刀"
        repaired = client.create(wrong)
        repaired_terminal = client.wait(repaired)
        require(repaired_terminal["status"] == "completed", "repair did not complete")
        require(repaired_terminal["result"]["best"]["reproduced"], "blocked initial macro was not repaired")
        require(any(trial["accepted"] and trial["round"] > 0 for trial in repaired_terminal["result"]["history"]), "no independently replayed repair accepted")
        replay_artifact(client, repaired, repaired_terminal)
        checks += 2

        limited = copy.deepcopy(wrong)
        limited["max_simulations"] = 2
        limited_job = client.create(limited)
        limited_terminal = client.wait(limited_job)
        require(limited_terminal["status"] == "budget_exhausted", "budget exhaustion was reported as completion")
        require(limited_terminal["simulations"] == 2, "two-simulation budget not respected")
        require(limited_terminal["result"]["best"]["verified"] and not limited_terminal["result"]["best"]["reproduced"], "budget stop lost/mislabelled the tested candidate")
        replay_artifact(client, limited_job, limited_terminal)
        checks += 2

        cancel_and_conflicts(client, base)
        checks += 3
    finally:
        cleanup_errors = []
        for job_id in client.created:
            try:
                path = f"/api/harness/jobs/{job_id}"
                status, current = client.request(path)
                if status == 404:
                    continue  # bounded in-memory history may have evicted this terminal job
                if current["running"]:
                    client.ok(path + "/cancel", {})
                    client.wait({"job_id": job_id, "status_url": path,
                                 "scenario_hash": current["scenario_hash"], "experiment_hash": current["experiment_hash"]})
            except Exception as error:
                cleanup_errors.append(str(error))
        try:
            response = client.ok("/api/mounts/switch", {"version": original["version"], "mount": original["mount"], "persist": False})
            actual = client.ok("/api/mounts/current")
            restored = bool(response.get("ok")) and actual["version"] == original["version"] and actual["mount"] == original["mount"]
            require(restored, "original runtime was not restored")
        except Exception as error:
            cleanup_errors.append(str(error))
        if cleanup_errors:
            raise AssertionError("cleanup failed: " + "; ".join(cleanup_errors))
    print(f"PASS harness HTTP smoke: checks={checks} runtime_restored={str(restored).lower()} agent_session_writes=0")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"FAIL harness HTTP smoke: {error}", file=sys.stderr)
        sys.exit(1)
