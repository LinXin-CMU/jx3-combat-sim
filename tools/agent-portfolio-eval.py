"""Run a fixed task suite on an isolated worker; retain reports for independent review.

These checks measure observable output, not general semantic correctness. No
provider transcript or hidden reasoning is exported. There is no token cutoff.
"""
import argparse
import copy
import json
import time
from pathlib import Path
from urllib.parse import urlsplit
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[1]


def request(base, path, body=None):
    data = None if body is None else json.dumps(body, ensure_ascii=False).encode("utf-8")
    with urlopen(Request(base + path, data=data, headers={"Content-Type": "application/json"}), timeout=30) as response:
        return json.load(response)


def grade(case, result):
    content = (result.get("report") or {}).get("content") or {}
    completion = result.get("task_completion") or {}
    calls = (result.get("debug") or {}).get("tool_calls") or []
    checks = {
        "answer_present": bool(content.get("body_markdown") or content.get("summary")),
        "run_has_answer": result.get("status") in ("completed", "partially_verified", "refused"),
        "semantic_review_is_separate": completion.get("semantic_review_required") is True,
    }
    if case.get("acceptance"):
        checks["explicit_acceptance_passed"] = completion.get("status") == "checks_passed"
    if case["id"] == "network_comparison":
        comparisons = [call for call in calls if call.get("tool_name") == "compare_scenarios" and call.get("ok")]
        candidates = [candidate for call in comparisons for candidate in call.get("arguments", {}).get("candidates", [])]
        checks["only_requested_variable_changed"] = bool(candidates) and all(candidate.get("patch") == {"network_delay": 0} for candidate in candidates)
    if case["id"] == "macro_delivery":
        macros = [artifact for artifact in completion.get("artifacts", []) if artifact.get("language") == "jx3_macro"]
        checks["delivered_macro_tested"] = bool(macros) and all(item.get("status") == "simulation_matched" for item in macros)
    return checks


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--backend", required=True)
    parser.add_argument("--provider", default="deepseek-v4-flash")
    parser.add_argument("--trials", type=int, default=2)
    parser.add_argument("--output", required=True)
    parser.add_argument("--case", action="append", default=[])
    args = parser.parse_args()
    if urlsplit(args.backend).hostname not in ("localhost", "127.0.0.1"):
        parser.error("Use an isolated local worker.")
    if args.trials < 1:
        parser.error("trials must be positive")
    suite = json.loads((ROOT / "backend/tests/agent_task_eval/cases.json").read_text(encoding="utf-8"))
    fixture = json.loads((ROOT / "backend/tests/agent_diagnostic_eval/scenario.json").read_text(encoding="utf-8"))
    simulation = fixture["simulation"]
    simulation["sequence"] = ["__macro__"] * fixture["macro_slots"]
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    if output.exists():
        parser.error("Output already exists; keep prior trials and use a new output path.")
    rows = []
    original = request(args.backend, "/api/mounts/current")
    request(args.backend, "/api/mounts/switch", {"version": "AnYingQianJi", "mount": "FenShanJin", "persist": False})
    try:
        for trial in range(1, args.trials + 1):
            for case in suite["cases"]:
                if args.case and case["id"] not in args.case:
                    continue
                body = {"question": case["question"], "provider_profile": args.provider,
                        "simulation": copy.deepcopy(simulation), "acceptance": case.get("acceptance")}
                created = request(args.backend, "/api/agent/runs", body)
                print(json.dumps({"case": case["id"], "trial": trial, "state": "started"}), flush=True)
                deadline = time.monotonic() + 720
                while True:
                    status = request(args.backend, created["status_url"])
                    if not status.get("running"):
                        break
                    if time.monotonic() > deadline:
                        raise TimeoutError("Observer timed out; run was not cancelled. Inspect isolated worker before cleanup.")
                    time.sleep(1)
                result = status.get("result") or {}
                checks = grade(case, result)
                row = {"case_id": case["id"], "trial": trial, "question": case["question"],
                       "status": result.get("status"), "model": result.get("model"),
                       "prompt_version": result.get("prompt_version"), "scenario_hash": result.get("scenario_hash"),
                       "accounting": result.get("accounting"), "task_completion": result.get("task_completion"),
                       "report": (result.get("report") or {}).get("content"), "error": result.get("error"),
                       "calls": [{key: call.get(key) for key in ("tool_name", "arguments", "ok", "code", "reused", "evidence_ids")}
                                 for call in (result.get("debug") or {}).get("tool_calls", [])],
                       "objective_checks": checks, "objective_passed": all(checks.values()),
                       "semantic_review": {"status": "pending", "rubric": case["rubric"], "scores": None, "notes": ""}}
                rows.append(row)
                output.write_text(json.dumps({"schema_version": "agent-task-eval-results/v1", "scope": suite["description"], "trials": rows}, ensure_ascii=False, indent=2), encoding="utf-8")
                print(json.dumps({"case": case["id"], "trial": trial, "status": row["status"],
                                  "checks": checks, "seconds": round(result.get("accounting", {}).get("duration_ms", 0) / 1000, 2)}, ensure_ascii=False), flush=True)
    finally:
        request(args.backend, "/api/mounts/switch", {"version": original["version"], "mount": original["mount"], "persist": False})


if __name__ == "__main__":
    main()
