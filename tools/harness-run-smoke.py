#!/usr/bin/env python3
"""Exercise the autonomous Harness through an isolated local worker.

No credentials, provider responses, or private reasoning are printed. Select a
live profile explicitly to evaluate real model decisions. All numerical facts
come from the unchanged simulator and returned evidence, never test text.
"""
from __future__ import annotations
import argparse
import copy
import json
from pathlib import Path
import time
from urllib.parse import urlsplit
import importlib.util

spec = importlib.util.spec_from_file_location("harness_http", Path(__file__).with_name("harness-smoke.py"))
http = importlib.util.module_from_spec(spec)
spec.loader.exec_module(http)
Client = http.Client
ROOT = Path(__file__).resolve().parents[1]


def wait(client, created, timeout=300):
    started = time.monotonic()
    while time.monotonic() - started < timeout:
        result = client.ok(created["status_url"])
        if not result["running"]:
            return result
        time.sleep(0.25)
    client.ok(created["cancel_url"], {})
    raise AssertionError("autonomous run did not stop within test deadline")


def payload(kind, profile):
    sim = json.loads((ROOT / "backend/tests/agent_diagnostic_eval/scenario.json").read_text(encoding="utf-8"))["simulation"]
    original_talents, original_recipes = sim["talents"], sim["recipes"]
    sim.update(sequence=["盾刀"] * 5, macro_text=None, macro_duration=None, talents=[], recipes=[], network_delay=0)
    equipment = {"slots": {slot: {"equip_id": identifier, "strength": 0, "embedding": [], "enhance_id": 0, "enchant_id": 0} for slot, identifier in sim["equipment"].items()}, "stone_id": 0, "source_label": "isolated-test"}
    if kind == "macro":
        goal = "把当前手动技能轴写成可复现宏。先直接evaluate测试 /cast [rage>100] 盾刀 这个候选，根据真实反例自主修正，再交付验证后的候选。不要假称坏候选可用。"
    elif kind == "rotation":
        sim.update(sequence=["__macro__"] * 160, macro_text="/cast [rage>100] 盾刀", macro_duration=20)
        goal = "优化当前循环，只允许盾刀。当前宏存在阻塞，请自主找到可运行并提高DPS的宏，使用不同网络延迟留出验证，交付实际候选。"
    elif kind == "equipment":
        sim.update(sequence=["__macro__"] * 160, macro_text="/cast 盾刀", macro_duration=20)
        goal = "优化当前配装。锁定除帽子外全部部位，先搜索允许目录内的帽子，再从所得候选做独立validate回放，保留明确差异、基线和证据；不声称全局最优。"
    elif kind == "joint":
        sim.update(sequence=["__macro__"] * 320, macro_text="/cast 盾刀", macro_duration=40,
                   talents=original_talents, recipes=original_recipes, network_delay=60)
        goal = "联合实验：先在允许目录里搜索帽子（其他部位锁定），再从配装候选作为parent优化当前仅盾刀的输出循环，允许使用当前奇穴下所有有效技能；最后对联合候选做120毫秒延迟的独立validate，交付经过验证的配装和宏。若某阶段未提升，保留当前最好方案并明确说明，不夸大最优性。"
    else:
        raise ValueError("unknown evaluation case")
    body = {"goal": goal, "provider_profile": profile, "simulation": sim, "version": "AnYingQianJi", "mount": "FenShanJin",
            "constraints": {"duration_seconds": 20, "max_pages": 2, "allowed_skills": ["盾刀"]},
            "budget": {"max_model_calls": 16, "max_simulations": 96, "wall_time_ms": 240000, "max_output_tokens": 8192, "max_total_tokens": 96000}}
    if kind in {"equipment", "joint"}:
        body["equipment"] = equipment
        body["constraints"]["locked_slots"] = [slot for slot in equipment["slots"] if slot != "HAT"]
        body["constraints"]["max_candidates_per_slot"] = 3
    if kind == "joint":
        body["constraints"].update(duration_seconds=40, allowed_skills=[], max_pages=4)
        body["budget"].update(max_simulations=192, max_model_calls=24, max_total_tokens=256000)
    return body


def run_case(client, body, kind, is_live, owned=None):
    created = client.ok("/api/harness/runs", body, 202)
    if owned is not None:
        owned.append(created["run_id"])
    result = wait(client, created)
    assert result["status"] == "completed", (kind, result["status"], result["message"])
    assert result["usage"]["simulations"] <= body["budget"]["max_simulations"]
    assert result["usage"]["model_calls"] <= body["budget"]["max_model_calls"]
    bundle = client.ok(created["artifacts_url"])
    assert bundle["artifacts"], "model delivered without real experiments"
    assert "reasoning_content" not in json.dumps(bundle), "private reasoning leaked to artifact bundle"
    selected_id = result["result"]["selected_artifact_id"]
    selected = next((a for a in bundle["artifacts"] if a["id"] == selected_id), None)
    assert selected is not None, "model did not select a tested candidate"
    best = selected["result"].get("best") or {}
    assert best.get("verified") is True, "selected candidate was never verified"
    assert best.get("constraints_passed") is not False and best.get("page_constraints_passed") is not False
    if kind == "macro":
        assert best.get("reproduced") is True, "selected macro does not reproduce reference"
        if is_live:
            assert len(bundle["artifacts"]) >= 2, "model skipped required counterexample experiment"
    if kind == "rotation":
        assert best.get("metrics", best).get("dps", 0) > 0, "blocked rotation not repaired"
    if kind in {"equipment", "joint"}:
        assert selected["equipment"] is not None
        for slot in body["constraints"]["locked_slots"]:
            assert selected["equipment"]["slots"][slot] == body["equipment"]["slots"][slot], "locked equipment changed"
    if kind == "joint":
        lineage = []
        ancestor = selected
        while ancestor:
            lineage.append(ancestor["kind"])
            ancestor = next((a for a in bundle["artifacts"] if a["id"] == ancestor.get("parent_id")), None)
        assert "optimize_equipment" in lineage, "joint candidate lost equipment ancestry"
        assert "search_rotation" in lineage or "evaluate" in lineage, "joint candidate omitted rotation experiment"
        assert selected["kind"] == "validate" and selected["result"]["validation"]["independent"], "joint candidate was not independently validated"
    # Reproducibility and typed writes use the server's exact frozen request.
    replay = client.ok("/api/simulate", selected["simulation"])
    metrics = best.get("metrics", best)
    if selected["kind"] != "validate":
        assert abs(replay["dps"] - metrics["dps"]) < 1e-6, "artifact replay differs"
    denied, _ = client.request(created["status_url"] + "/apply", {"artifact_id": selected_id, "expected_scenario_hash": "stale"})
    assert denied == 409
    transaction = client.ok(created["status_url"] + "/apply", {"artifact_id": selected_id, "expected_scenario_hash": result["scenario_hash"]})
    assert transaction["after"]["simulation"] == selected["simulation"]
    reverse = client.ok(created["status_url"] + "/undo", {"transaction_id": transaction["transaction_id"]})
    assert reverse["after"] == transaction["before"]
    return {"case": kind, "status": result["status"], "completion": result["result"]["completion"], "model_calls": result["usage"]["model_calls"],
            "simulations": result["usage"]["simulations"], "tokens": result["usage"]["total_tokens"], "elapsed_ms": result["usage"]["elapsed_ms"],
            "artifact_count": len(bundle["artifacts"]), "selected_kind": selected["kind"], "run_id": result["run_id"]}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--backend", default="http://127.0.0.1:3319")
    parser.add_argument("--profile", default="offline")
    parser.add_argument("--cases", default="macro,rotation,equipment")
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    assert urlsplit(args.backend).hostname in {"localhost", "127.0.0.1"}, "isolated local worker only"
    client = Client(args.backend)
    original = client.ok("/api/mounts/current")
    results, owned = [], []
    failure = None
    try:
        client.ok("/api/mounts/switch", {"version": "AnYingQianJi", "mount": "FenShanJin", "persist": False})
        caps = client.ok("/api/harness/capabilities")
        assert caps["workflow"] == "model_selected_experiments"
        invalid = payload("macro", args.profile)
        invalid["budget"]["max_simulations"] = 99999
        assert client.request("/api/harness/runs", invalid)[0] == 400
        for kind in args.cases.split(","):
            row = run_case(client, payload(kind, args.profile), kind, args.profile != "offline", owned)
            results.append(row)
            print(json.dumps(row, ensure_ascii=False), flush=True)
    except Exception as error:
        failure = str(error)
        raise
    finally:
        # Cancel only runs this evaluator created; never touch another user's work.
        for row in client.ok("/api/harness/runs")["runs"]:
            if row.get("running") and row.get("run_id") in owned:
                client.ok(f'/api/harness/runs/{row["run_id"]}/cancel', {})
        client.ok("/api/mounts/switch", {"version": original["version"], "mount": original["mount"], "persist": False})
        if args.report:
            args.report.parent.mkdir(parents=True, exist_ok=True)
            args.report.write_text(json.dumps({"schema": "harness-model-eval/v2", "profile": args.profile, "results": results,
                                               "requested_cases": args.cases.split(","), "failure": failure, "owned_run_ids": owned}, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"PASS autonomous harness: {len(results)} scenarios; typed apply/undo and full replay verified")


if __name__ == "__main__":
    main()
