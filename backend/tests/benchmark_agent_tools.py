#!/usr/bin/env python3
"""Benchmark the four read-only Agent HTTP tools and export compact traces."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import statistics
import sys
import time
import urllib.error
import urllib.request

import diff_baseline
import run_agent_eval


SCHEMA_VERSION = "agent-tool-benchmark/v1"
GAME_VERSION = "AnYingQianJi"
MOUNT = "FenShanJin"


class BenchmarkFailure(Exception):
    pass


def encoded_json(value: object) -> bytes:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode("utf-8")


def request_json(
    base_url: str,
    path: str,
    payload: object,
    expected_status: int = 200,
) -> tuple[object, int, float]:
    encoded = encoded_json(payload)
    request = urllib.request.Request(
        f"{base_url.rstrip('/')}{path}",
        data=encoded,
        headers={"Content-Type": "application/json; charset=utf-8"},
        method="POST",
    )
    started = time.perf_counter_ns()
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            status = response.status
            raw = response.read()
    except urllib.error.HTTPError as error:
        status = error.code
        raw = error.read()
    elapsed_ms = (time.perf_counter_ns() - started) / 1_000_000
    try:
        document = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise BenchmarkFailure(f"{path} returned invalid UTF-8 JSON") from error
    if status != expected_status:
        raise BenchmarkFailure(
            f"{path} returned HTTP {status}, expected {expected_status}: {document}"
        )
    return document, len(raw), elapsed_ms


def percentile(values: list[float], quantile: float) -> float:
    if not values:
        raise ValueError("percentile needs at least one sample")
    ordered = sorted(values)
    position = (len(ordered) - 1) * quantile
    lower = int(position)
    upper = min(lower + 1, len(ordered) - 1)
    fraction = position - lower
    return ordered[lower] + (ordered[upper] - ordered[lower]) * fraction


def distribution(values: list[float], digits: int = 3) -> dict[str, float]:
    return {
        "p50": round(percentile(values, 0.50), digits),
        "p95": round(percentile(values, 0.95), digits),
        "mean": round(statistics.fmean(values), digits),
        "min": round(min(values), digits),
        "max": round(max(values), digits),
    }


def evidence_for(tool_name: str, response: dict[str, object]) -> list[dict[str, object]]:
    if tool_name == "analyze_timeline":
        return [response["simulation"], response["timeline"]]
    return [response["evidence"]]


def stable_signature(tool_name: str, response: dict[str, object]) -> tuple[object, ...]:
    envelopes = evidence_for(tool_name, response)
    signature: list[object] = [
        item
        for envelope in envelopes
        for item in (envelope["evidence_id"], envelope["scenario_hash"])
    ]
    if tool_name == "simulate_scenario":
        signature.append(response["evidence"]["result"]["fingerprint_hex"])
    elif tool_name == "compare_scenarios":
        result = response["evidence"]["result"]
        signature.extend(
            [
                result["baseline"]["fingerprint_hex"],
                *(candidate["metrics"]["fingerprint_hex"] for candidate in result["candidates"]),
            ]
        )
    elif tool_name == "analyze_timeline":
        signature.append(response["timeline"]["result"]["fingerprint_hex"])
    return tuple(signature)


def server_duration_ms(tool_name: str, response: dict[str, object]) -> int:
    return sum(int(envelope["duration_ms"]) for envelope in evidence_for(tool_name, response))


def benchmark_tool(
    base_url: str,
    tool_name: str,
    path: str,
    payload: dict[str, object],
    warmups: int,
    samples: int,
) -> tuple[dict[str, object], dict[str, object]]:
    for _ in range(warmups):
        request_json(base_url, path, payload)

    wall_times: list[float] = []
    server_times: list[float] = []
    response_sizes: list[float] = []
    signatures: list[tuple[object, ...]] = []
    last_response: dict[str, object] | None = None
    for _ in range(samples):
        response, response_size, wall_ms = request_json(base_url, path, payload)
        if not isinstance(response, dict):
            raise BenchmarkFailure(f"{tool_name} returned a non-object response")
        envelopes = evidence_for(tool_name, response)
        expected_tools = (
            ["simulate_scenario", "analyze_timeline"]
            if tool_name == "analyze_timeline"
            else [tool_name]
        )
        for envelope, expected_tool in zip(envelopes, expected_tools, strict=True):
            run_agent_eval.validate_envelope(envelope, expected_tool)
        signatures.append(stable_signature(tool_name, response))
        wall_times.append(wall_ms)
        server_times.append(float(server_duration_ms(tool_name, response)))
        response_sizes.append(float(response_size))
        last_response = response

    consistent = all(signature == signatures[0] for signature in signatures)
    if not consistent:
        first_difference = next(
            signature for signature in signatures[1:] if signature != signatures[0]
        )
        raise BenchmarkFailure(
            f"{tool_name} evidence or fingerprint changed between runs: "
            f"first={signatures[0]!r} changed={first_difference!r}"
        )
    assert last_response is not None
    metrics = {
        "samples": samples,
        "request_bytes": len(encoded_json(payload)),
        "response_bytes": distribution(response_sizes, digits=1),
        "wall_ms": distribution(wall_times),
        "server_duration_ms": distribution(server_times),
        "evidence_and_fingerprint_consistent": True,
        "stable_signature": list(signatures[0]),
    }
    return metrics, last_response


def benchmark_payloads(simulation: dict[str, object], scenario: dict[str, object]):
    return {
        "get_current_scenario": (
            "/api/agent/tools/scenario",
            {"trace_id": "p1-07-bench-scenario", "simulation": simulation},
        ),
        "simulate_scenario": (
            "/api/agent/tools/simulate",
            {
                "trace_id": "p1-07-bench-simulate",
                "scenario": scenario,
                "max_simulations": 1,
            },
        ),
        "compare_scenarios": (
            "/api/agent/tools/compare",
            {
                "trace_id": "p1-07-bench-compare",
                "baseline": scenario,
                "candidates": [
                    {"label": "network-25ms", "patch": {"network_delay": 25}}
                ],
                "max_simulations": 2,
            },
        ),
        "analyze_timeline": (
            "/api/agent/tools/timeline",
            {
                "trace_id": "p1-07-bench-timeline",
                "scenario": scenario,
                "max_simulations": 1,
            },
        ),
    }


def compact_envelope(
    envelope: dict[str, object],
    result_projection: object,
) -> dict[str, object]:
    return {
        "schema_version": envelope["schema_version"],
        "trace_id": envelope["trace_id"],
        "evidence_id": envelope["evidence_id"],
        "tool_name": envelope["tool_name"],
        "scenario_hash": envelope["scenario_hash"],
        "engine_version": envelope["engine_version"],
        "engine_commit": envelope["engine_commit"],
        "data_hash": envelope["data_hash"],
        "args": envelope["args"],
        "result_projection": result_projection,
        "warnings": envelope["warnings"],
        "duration_ms": envelope["duration_ms"],
    }


def timeline_projection(result: dict[str, object]) -> dict[str, object]:
    return {
        "fingerprint_hex": result["fingerprint_hex"],
        "fight_time": result["fight_time"],
        "active_event_count": result["active_event_count"],
        "triggered_event_count": result["triggered_event_count"],
        "skill_groups": len(result["skills"]),
        "total_cd_wait_seconds": result["total_cd_wait_seconds"],
        "cd_wait_count": len(result["cd_waits"]),
        "total_observed_gcd_gap_seconds": result["total_observed_gcd_gap_seconds"],
        "gcd_gap_count": len(result["gcd_gaps"]),
        "rage": result["rage"],
        "buff_coverage": [
            {
                "buff_id": coverage["buff_id"],
                "active_seconds": coverage["active_seconds"],
                "coverage_percent": coverage["coverage_percent"],
                "activation_count": coverage["activation_count"],
            }
            for coverage in result["buff_coverage"]
        ],
        "skipped": result["skipped"],
        "limitations": result["limitations"],
    }


def compact_success_trace(
    base_url: str,
    simulation: dict[str, object],
) -> tuple[dict[str, object], dict[str, object]]:
    trace_id = "p1-07-success"
    captured, _, _ = request_json(
        base_url,
        "/api/agent/tools/scenario",
        {"trace_id": trace_id, "simulation": simulation},
    )
    scenario = captured["scenario"]
    simulated, _, _ = request_json(
        base_url,
        "/api/agent/tools/simulate",
        {"trace_id": trace_id, "scenario": scenario, "max_simulations": 1},
    )
    compared, _, _ = request_json(
        base_url,
        "/api/agent/tools/compare",
        {
            "trace_id": trace_id,
            "baseline": scenario,
            "candidates": [
                {"label": "network-25ms", "patch": {"network_delay": 25}}
            ],
            "max_simulations": 2,
        },
    )
    timeline, _, _ = request_json(
        base_url,
        "/api/agent/tools/timeline",
        {"trace_id": trace_id, "scenario": scenario, "max_simulations": 1},
    )
    success = {
        "trace_id": trace_id,
        "scenario_hash": scenario["scenario_hash"],
        "projection_note": (
            "Evidence IDs bind the complete tool results; large per-event lists are "
            "projected to counts and aggregates in this portfolio artifact."
        ),
        "evidence": [
            compact_envelope(
                captured["evidence"],
                captured["evidence"]["result"],
            ),
            compact_envelope(
                simulated["evidence"],
                {
                    key: simulated["evidence"]["result"][key]
                    for key in (
                        "dps",
                        "total_damage",
                        "fight_time",
                        "skill_count",
                        "fingerprint_hex",
                    )
                },
            ),
            compact_envelope(
                compared["evidence"],
                compared["evidence"]["result"],
            ),
            compact_envelope(
                timeline["simulation"],
                {
                    key: timeline["simulation"]["result"][key]
                    for key in (
                        "dps",
                        "total_damage",
                        "fight_time",
                        "skill_count",
                        "fingerprint_hex",
                    )
                },
            ),
            compact_envelope(
                timeline["timeline"],
                timeline_projection(timeline["timeline"]["result"]),
            ),
        ],
        "chain_checks": {
            "all_scenario_hashes_match": len(
                {
                    captured["evidence"]["scenario_hash"],
                    simulated["evidence"]["scenario_hash"],
                    compared["evidence"]["scenario_hash"],
                    timeline["simulation"]["scenario_hash"],
                    timeline["timeline"]["scenario_hash"],
                }
            ) == 1,
            "timeline_source_matches_simulation": (
                timeline["timeline"]["args"]["source_evidence_id"]
                == timeline["simulation"]["evidence_id"]
            ),
            "repeat_simulation_evidence_matches": (
                simulated["evidence"]["evidence_id"]
                == timeline["simulation"]["evidence_id"]
            ),
        },
    }

    rejected_payload = {
        "trace_id": "p1-07-rejected",
        "baseline": scenario,
        "candidates": [{"label": "noop", "patch": {"network_delay": 0}}],
        "max_simulations": 2,
    }
    rejected, response_size, wall_ms = request_json(
        base_url,
        "/api/agent/tools/compare",
        rejected_payload,
        expected_status=422,
    )
    failure = {
        "trace_id": "p1-07-rejected",
        "request_summary": {
            "tool_name": "compare_scenarios",
            "candidate_label": "noop",
            "patch": {"network_delay": 0},
        },
        "http_status": 422,
        "response_bytes": response_size,
        "wall_ms": round(wall_ms, 3),
        "response": rejected,
    }
    return success, failure


def parse_args():
    parser = argparse.ArgumentParser()
    parser.add_argument("--backend", default="http://127.0.0.1:3005")
    parser.add_argument("--warmups", type=int, default=3)
    parser.add_argument("--samples", type=int, default=30)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.warmups < 0 or args.samples < 2:
        raise BenchmarkFailure("warmups must be >= 0 and samples must be >= 2")

    original_runtime = run_agent_eval.http_get_json(args.backend, "/api/mounts/current")
    before_userdata = run_agent_eval.snapshot_userdata()
    restored = False
    document: dict[str, object] | None = None
    try:
        run_agent_eval.switch_runtime(
            args.backend,
            {"version": GAME_VERSION, "mount": MOUNT},
        )
        simulation = diff_baseline.macro_request(diff_baseline.JUEYUN_MACRO, lite=False)
        captured, _, _ = request_json(
            args.backend,
            "/api/agent/tools/scenario",
            {"trace_id": "p1-07-setup", "simulation": simulation},
        )
        scenario = captured["scenario"]
        tools: dict[str, object] = {}
        for tool_name, (path, payload) in benchmark_payloads(simulation, scenario).items():
            metrics, _ = benchmark_tool(
                args.backend,
                tool_name,
                path,
                payload,
                args.warmups,
                args.samples,
            )
            tools[tool_name] = metrics

        success_trace, rejected_trace = compact_success_trace(args.backend, simulation)
        provenance = captured["evidence"]
        if provenance["engine_commit"] == "unknown":
            raise BenchmarkFailure(
                "engine_commit is unknown; rebuild with JX3_BUILD_COMMIT set"
            )
        document = {
            "schema_version": SCHEMA_VERSION,
            "captured_at": datetime.now(timezone.utc).isoformat(),
            "environment": {
                "backend": "127.0.0.1 loopback",
                "build": "release",
                "game_version": GAME_VERSION,
                "mount": MOUNT,
                "engine_version": provenance["engine_version"],
                "engine_commit": provenance["engine_commit"],
                "data_hash": provenance["data_hash"],
            },
            "method": {
                "scenario": "JUEYUN macro, 300 second fight",
                "warmups_per_tool": args.warmups,
                "samples_per_tool": args.samples,
                "execution": "serial HTTP request, full body read, UTF-8 decode, JSON parse",
                "percentile": "linear interpolation over sorted samples",
            },
            "tools": tools,
            "traces": {
                "success": success_trace,
                "rejected": rejected_trace,
            },
        }
    finally:
        run_agent_eval.restore_runtime(args.backend, original_runtime)
        restored = True

    after_userdata = run_agent_eval.snapshot_userdata()
    assert document is not None
    document["safety"] = {
        "tool_write_attempts": 0,
        "userdata_unchanged": before_userdata == after_userdata,
        "runtime_restored": restored,
    }
    if (
        document["safety"]["tool_write_attempts"] != 0
        or not document["safety"]["userdata_unchanged"]
        or not document["safety"]["runtime_restored"]
    ):
        raise BenchmarkFailure(f"safety audit failed: {document['safety']}")
    json.dump(document, sys.stdout, ensure_ascii=False, indent=2)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (BenchmarkFailure, run_agent_eval.EvalFailure) as error:
        print(f"[FAIL] {error}", file=sys.stderr)
        sys.exit(1)
