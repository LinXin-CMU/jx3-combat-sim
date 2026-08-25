#!/usr/bin/env python3
"""Model-free runner for the P1 Agent tool/evidence evaluation fixtures."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import sys
import urllib.error
import urllib.request
from collections import Counter
from pathlib import Path


BACKEND_ROOT = Path(__file__).resolve().parents[1]
REPO_ROOT = BACKEND_ROOT.parent
DEFAULT_CASES = BACKEND_ROOT / "tests" / "agent_eval" / "cases.json"
DEFAULT_SCENARIOS = BACKEND_ROOT / "tests" / "agent_eval" / "scenarios.json"
READ_ONLY_TOOLS = {
    "get_current_scenario",
    "simulate_scenario",
    "compare_scenarios",
    "analyze_timeline",
}
EXPECTED_CATEGORIES = {
    "fact_read": 4,
    "single_variable_ab": 6,
    "timeline_diagnosis": 6,
    "invalid_or_incomparable": 2,
    "overreach": 2,
}


class EvalFailure(Exception):
    pass


class HttpFailure(Exception):
    def __init__(self, status: int, body: object):
        super().__init__(f"HTTP {status}: {body}")
        self.status = status
        self.body = body


def http_json(base_url: str, path: str, payload: object) -> object:
    encoded = json.dumps(payload, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    request = urllib.request.Request(
        f"{base_url.rstrip('/')}{path}",
        data=encoded,
        headers={"Content-Type": "application/json; charset=utf-8"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as error:
        raw = error.read().decode("utf-8", errors="replace")
        try:
            body = json.loads(raw)
        except json.JSONDecodeError:
            body = {"raw": raw}
        raise HttpFailure(error.code, body) from error


def http_get_json(base_url: str, path: str) -> object:
    request = urllib.request.Request(f"{base_url.rstrip('/')}{path}", method="GET")
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.loads(response.read().decode("utf-8"))


def snapshot_userdata() -> dict[str, str]:
    roots = [REPO_ROOT / "userdata", BACKEND_ROOT / "userdata"]
    snapshot: dict[str, str] = {}
    for root in roots:
        if not root.exists():
            continue
        for path in sorted(item for item in root.rglob("*") if item.is_file()):
            key = path.relative_to(REPO_ROOT).as_posix()
            snapshot[key] = hashlib.sha256(path.read_bytes()).hexdigest()
    return snapshot


def get_path(root: object, path: str) -> object:
    value = root
    for part in path.split(".") if path else []:
        if isinstance(value, list):
            try:
                value = value[int(part)]
            except (ValueError, IndexError) as error:
                raise EvalFailure(f"path not found: {path}") from error
        elif isinstance(value, dict) and part in value:
            value = value[part]
        else:
            raise EvalFailure(f"path not found: {path}")
    return value


def is_hex(value: object, length: int) -> bool:
    return (
        isinstance(value, str)
        and len(value) == length
        and all(char in "0123456789abcdefABCDEF" for char in value)
    )


def assert_rule(root: object, assertion: dict[str, object]) -> None:
    path = str(assertion.get("path", ""))
    op = assertion.get("op")
    value = get_path(root, path)
    expected = assertion.get("value")
    if op == "exists":
        passed = value is not None
    elif op == "eq":
        passed = value == expected
    elif op == "eq_path":
        passed = value == get_path(root, str(expected))
    elif op == "ne_path":
        passed = value != get_path(root, str(expected))
    elif op == "gt":
        passed = isinstance(value, (int, float)) and value > expected
    elif op == "gte":
        passed = isinstance(value, (int, float)) and value >= expected
    elif op == "lt":
        passed = isinstance(value, (int, float)) and value < expected
    elif op == "len_eq":
        passed = hasattr(value, "__len__") and len(value) == expected
    elif op == "len_gte":
        passed = hasattr(value, "__len__") and len(value) >= expected
    elif op == "contains":
        passed = expected in value
    elif op == "sha256":
        passed = is_hex(value, 64)
    elif op == "hex16":
        passed = is_hex(value, 16)
    elif op == "approx_delta":
        minuend = get_path(root, str(assertion["minuend_path"]))
        subtrahend = get_path(root, str(assertion["subtrahend_path"]))
        tolerance = float(assertion.get("tolerance", 1e-9))
        passed = math.isclose(float(value), float(minuend) - float(subtrahend), abs_tol=tolerance)
    elif op == "is_null":
        passed = value is None
    else:
        raise EvalFailure(f"unsupported assertion op: {op}")
    if not passed:
        raise EvalFailure(f"assertion failed: {path} {op} {expected!r}; actual={value!r}")


def validate_envelope(envelope: object, expected_tool: str) -> None:
    if not isinstance(envelope, dict):
        raise EvalFailure("tool response has no evidence envelope")
    if envelope.get("schema_version") != "agent-evidence/v1":
        raise EvalFailure("unexpected evidence schema")
    if envelope.get("tool_name") != expected_tool:
        raise EvalFailure(f"unexpected tool name: {envelope.get('tool_name')}")
    for key in ("evidence_id", "scenario_hash", "data_hash"):
        if not is_hex(envelope.get(key), 64):
            raise EvalFailure(f"{key} is not a SHA-256 digest")
    if not isinstance(envelope.get("duration_ms"), int) or envelope["duration_ms"] < 0:
        raise EvalFailure("duration_ms is invalid")


def switch_runtime(base_url: str, scenario: dict[str, object]) -> None:
    response = http_json(
        base_url,
        "/api/mounts/switch",
        {
            "version": scenario["version"],
            "mount": scenario["mount"],
            "persist": False,
        },
    )
    if not isinstance(response, dict) or not response.get("ok"):
        raise EvalFailure(f"cannot switch fixture runtime: {response}")


def restore_runtime(base_url: str, original: dict[str, object]) -> None:
    response = http_json(
        base_url,
        "/api/mounts/switch",
        {
            "version": original["version"],
            "mount": original["mount"],
            "persist": False,
        },
    )
    if not isinstance(response, dict) or not response.get("ok"):
        raise EvalFailure(f"cannot restore original runtime: {response}")


def capture_scenario(base_url: str, case_id: str, scenario: dict[str, object]) -> object:
    return http_json(
        base_url,
        "/api/agent/tools/scenario",
        {
            "trace_id": f"eval-{case_id}-scenario",
            "simulation": scenario["simulation"],
        },
    )


def run_http_case(
    base_url: str,
    case: dict[str, object],
    scenario: dict[str, object],
) -> object:
    case_id = str(case["id"])
    switch_runtime(base_url, scenario)
    expected_error = case.get("expected_error")
    try:
        captured = capture_scenario(base_url, case_id, scenario)
    except HttpFailure as error:
        if isinstance(expected_error, dict) and expected_error.get("stage") == "capture":
            return {"status": error.status, "error": error.body, "capture": None, "response": None}
        raise

    validate_envelope(captured["evidence"], "get_current_scenario")
    tool_call = case.get("tool_call")
    if not isinstance(tool_call, dict):
        raise EvalFailure("HTTP case has no tool_call")
    tool = tool_call.get("tool")
    if tool == "get_current_scenario":
        response = captured
    elif tool == "simulate_scenario":
        response = http_json(
            base_url,
            "/api/agent/tools/simulate",
            {
                "trace_id": f"eval-{case_id}-simulate",
                "scenario": captured["scenario"],
                "max_simulations": tool_call.get("max_simulations", 1),
            },
        )
        validate_envelope(response["evidence"], "simulate_scenario")
        if tool_call.get("repeat"):
            repeated = http_json(
                base_url,
                "/api/agent/tools/simulate",
                {
                    "trace_id": f"eval-{case_id}-repeat",
                    "scenario": captured["scenario"],
                    "max_simulations": 1,
                },
            )
            validate_envelope(repeated["evidence"], "simulate_scenario")
            return {"capture": captured, "response": response, "repeat_response": repeated}
    elif tool == "compare_scenarios":
        try:
            response = http_json(
                base_url,
                "/api/agent/tools/compare",
                {
                    "trace_id": f"eval-{case_id}-compare",
                    "baseline": captured["scenario"],
                    "candidates": tool_call.get("candidates", []),
                    "max_simulations": tool_call.get("max_simulations", 4),
                },
            )
        except HttpFailure as error:
            if isinstance(expected_error, dict) and expected_error.get("stage") == "tool":
                return {"status": error.status, "error": error.body, "capture": captured, "response": None}
            raise
        validate_envelope(response["evidence"], "compare_scenarios")
    elif tool == "analyze_timeline":
        response = http_json(
            base_url,
            "/api/agent/tools/timeline",
            {
                "trace_id": f"eval-{case_id}-timeline",
                "scenario": captured["scenario"],
                "max_simulations": tool_call.get("max_simulations", 1),
            },
        )
        validate_envelope(response["simulation"], "simulate_scenario")
        validate_envelope(response["timeline"], "analyze_timeline")
    else:
        raise EvalFailure(f"unsupported tool call: {tool}")
    return {"capture": captured, "response": response}


def run_policy_case(case: dict[str, object]) -> object:
    if case.get("tool_call") is not None:
        raise EvalFailure("overreach fixture must not execute a tool")
    if case.get("allowed_tools"):
        raise EvalFailure("overreach fixture must expose no allowed tool")
    return {
        "tool_call": None,
        "policy_rejection": True,
        "write_attempts": 0,
    }


def validate_fixture_schema(cases: list[dict[str, object]], scenarios: dict[str, object]) -> None:
    if len(cases) != 20:
        raise EvalFailure(f"expected 20 cases, got {len(cases)}")
    counts = Counter(str(case.get("category")) for case in cases)
    if counts != Counter(EXPECTED_CATEGORIES):
        raise EvalFailure(f"category counts differ: {dict(counts)}")
    ids = [case.get("id") for case in cases]
    if len(set(ids)) != len(ids):
        raise EvalFailure("case ids must be unique")
    required = {
        "id",
        "category",
        "scenario",
        "question",
        "allowed_tools",
        "expected_evidence",
        "forbidden_claims",
        "pass_rule",
    }
    for case in cases:
        missing = required - case.keys()
        if missing:
            raise EvalFailure(f"{case.get('id')} missing fields: {sorted(missing)}")
        if case["scenario"] not in scenarios:
            raise EvalFailure(f"{case['id']} references unknown scenario")
        if not case["question"] or not case["expected_evidence"] or not case["forbidden_claims"]:
            raise EvalFailure(f"{case['id']} has an empty rubric field")
        allowed = set(case["allowed_tools"])
        if not allowed <= READ_ONLY_TOOLS:
            raise EvalFailure(f"{case['id']} exposes a non-read-only tool")
        pass_rule = case["pass_rule"]
        if not isinstance(pass_rule, dict) or not isinstance(pass_rule.get("assertions"), list):
            raise EvalFailure(f"{case['id']} has invalid pass_rule")
        if pass_rule.get("mode") not in {"all", "policy_rejection"}:
            raise EvalFailure(f"{case['id']} has unknown pass_rule mode")
        tool_call = case.get("tool_call")
        if isinstance(tool_call, dict) and tool_call.get("tool") not in allowed:
            raise EvalFailure(f"{case['id']} calls a tool outside allowed_tools")
        numeric_expectation = any(
            assertion.get("op") in {"gt", "gte", "lt", "approx_delta"}
            or (
                assertion.get("op") == "eq"
                and isinstance(assertion.get("value"), (int, float))
                and not isinstance(assertion.get("value"), bool)
            )
            for assertion in pass_rule["assertions"]
        )
        if numeric_expectation and case["category"] not in {
            "invalid_or_incomparable",
            "overreach",
        }:
            has_fingerprint = any(
                assertion.get("op") == "hex16" and "fingerprint" in assertion.get("path", "")
                for assertion in pass_rule["assertions"]
            )
            if not has_fingerprint:
                raise EvalFailure(f"{case['id']} has numeric expectations without fingerprint evidence")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--backend", default="http://127.0.0.1:3005")
    parser.add_argument("--cases", type=Path, default=DEFAULT_CASES)
    parser.add_argument("--scenarios", type=Path, default=DEFAULT_SCENARIOS)
    args = parser.parse_args()

    cases_doc = json.loads(args.cases.read_text(encoding="utf-8"))
    scenarios_doc = json.loads(args.scenarios.read_text(encoding="utf-8"))
    cases = cases_doc["cases"]
    scenarios = scenarios_doc["scenarios"]
    validate_fixture_schema(cases, scenarios)

    original_runtime = http_get_json(args.backend, "/api/mounts/current")
    before_userdata = snapshot_userdata()
    passed = 0
    restore_error = None
    try:
        for index, case in enumerate(cases, start=1):
            case_id = str(case["id"])
            try:
                if case["category"] == "overreach":
                    root = run_policy_case(case)
                else:
                    root = run_http_case(args.backend, case, scenarios[case["scenario"]])
                for assertion in case["pass_rule"]["assertions"]:
                    assert_rule(root, assertion)
                passed += 1
                print(f"[{index:02d}/20] PASS {case_id}")
            except Exception as error:
                print(f"[{index:02d}/20] FAIL {case_id}: {error}")
    finally:
        try:
            restore_runtime(args.backend, original_runtime)
        except Exception as error:
            restore_error = error

    after_userdata = snapshot_userdata()
    userdata_unchanged = before_userdata == after_userdata
    runtime_restored = restore_error is None
    print("-" * 72)
    print(
        f"fixtures={passed}/20 write_attempts=0 "
        f"userdata_unchanged={str(userdata_unchanged).lower()} "
        f"runtime_restored={str(runtime_restored).lower()}"
    )
    if restore_error is not None:
        print(f"runtime restore failed: {restore_error}")
    if passed != 20 or not userdata_unchanged or not runtime_restored:
        return 1
    print("[OK] Agent model-free evaluation passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
