#!/usr/bin/env python3
"""Produce isolated, fully certified exact-macro cases from reviewed public sources.

The review/plan and every source body, scene and native result belong in the
ignored corpus store. A constructed page composition is never an assertion
that an author's complete configuration or manual rotation was reconstructed.
This module neither learns nor retrieves a source answer at runtime.
"""
from __future__ import annotations

import argparse
import copy
import importlib.util
import json
import math
from pathlib import Path
import time

ROOT = Path(__file__).resolve().parents[1]
STORE = ROOT / "backend/target/exact-macro-corpus"
SPEC = importlib.util.spec_from_file_location("training_cases_corpus", ROOT / "tools/exact_macro_corpus.py")
corpus = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(corpus)
SCHEMA_VERSION = 1
TARGET_CONSTRUCTION_VERSION = "nonchannel-cast-checkpoints-v1"
RESERVED_SOURCE_IDS = frozenset({"101545", "106800"})
SUPPORTED = {"AnYingQianJi": "暗影千机", "ShanHaiYuanLiu": "山海源流"}
MOUNTS = {"分山劲": "FenShanJin", "铁骨衣": "TieGuYi"}
CONTRACT_KEYS = ("version", "mount", "simulation", "horizon", "time_tolerance_seconds", "acceptance")
REQUIRED_ENVIRONMENT = frozenset({
    "haste_level", "sequence", "talents", "recipes", "initial_rage", "network_delay",
    "attributes", "target", "equipment", "team_buffs", "formation", "pre_releases",
    "pauses", "boss_attack_interval", "hanjia_expectation", "dunya_reset_seed", "tiegu_mode",
    "experimental", "channel_ticks", "timing_offsets", "qijin_buffs", "macro_text", "macro_duration",
    "lite", "lite_keep_timeline",
})


def load_json(path):
    return json.loads(Path(path).read_text(encoding="utf-8-sig"))


def source_versions(records):
    """Use the corpus' review fingerprint; context changes invalidate approval."""
    ids = {record["source_item_id"] for record in records}
    _, _, saved = corpus.split_config(records, {}, reviewed_sources=ids)
    return saved["reviewed_source_versions"]


def reviewed_records(manifest, review):
    """Close and reserve whole families before inspecting any selected bodies."""
    records = manifest["records"]
    versions = source_versions(records)
    entries = review.get("sources", [])
    approvals = {str(item["source_item_id"]): item for item in entries}
    if len(approvals) != len(entries):
        raise ValueError("duplicate source review")
    forbidden = RESERVED_SOURCE_IDS | set(map(str, review.get("heldout_source_ids", [])))
    reserved_families = {record["lineage_group_id"] for record in records
                         if record["source_item_id"] in forbidden or record.get("split") in ("test", "quarantine")
                         or record.get("reference_b_status") == "quarantined"
                         or record.get("reference_b_marker_in_source")}
    reserved_families.update(map(str, review.get('heldout_source_families', [])))
    by_family = {}
    for record in records:
        by_family.setdefault(record["lineage_group_id"], []).append(record)
    eligible = {}
    for identity, item in approvals.items():
        if identity in forbidden:
            raise ValueError("held-out source cannot be reviewed for training")
        if item.get("source_version_hash") != versions.get(identity):
            raise ValueError("source review is stale or source is absent: " + identity)
        if item.get("reference_b_status") != "reviewed_non_b" or not item.get("evidence"):
            raise ValueError("explicit non-B provenance review evidence is required")
        if item.get("split") not in ("train", "validation"):
            raise ValueError("production only accepts frozen train/validation assignments")
    for family, members in by_family.items():
        ids = {record["source_item_id"] for record in members}
        selected = ids & approvals.keys()
        if not selected:
            continue
        if family in reserved_families:
            raise ValueError("reserved/quarantined family cannot enter training")
        if not ids <= approvals.keys():
            raise ValueError("every observed source in a family must be reviewed")
        splits = {approvals[identity]["split"] for identity in ids}
        if len(splits) != 1:
            raise ValueError("a source family cannot cross splits")
        for record in members:
            eligible[record["record_id"]] = dict(record, split=next(iter(splits)), reference_b_status="reviewed_non_b")
    return eligible


def compose_program(selection, records):
    """Keep source blocks separate; page headers are an audited new artifact."""
    selected = []
    for page in selection["pages"]:
        record = records.get(page["record_id"])
        if record is None:
            raise ValueError("unreviewed or held-out selected record")
        if record.get("parse_status") != "parsed_subset":
            raise ValueError("unsupported source block cannot be composed")
        if len(record["ast"]["pages"]) != 1 or record["ast"]["pages"][0]["stance_filter"] is not None:
            raise ValueError("nested or existing source pages require a separate composition")
        selected.append(record)
    if not selected or len({record["record_id"] for record in selected}) != len(selected):
        raise ValueError("page composition needs distinct source blocks")
    for field in ("lineage_group_id", "split", "mount", "season", "client"):
        if len({record.get(field) for record in selected}) != 1:
            raise ValueError("page composition must remain within one reviewed family and version/mount")
    if len(selected) > 1 and (not selection.get("composition_reviewed") or not selection.get("composition_evidence")):
        raise ValueError("multi-block composition needs explicit page-use evidence")
    stances = [page.get("stance") for page in selection["pages"]]
    if any(stance not in ("shield", "blade", "wall") for stance in stances) or len(set(stances)) != len(stances):
        raise ValueError("distinct explicit native stance pages are required")
    chunks = []
    for page, record in zip(selection["pages"], selected):
        text = record["macro_blocks"][0]["raw_text"]
        if corpus.digest(text) != record["raw_text_hash"]:
            raise ValueError("source block hash changed")
        chunks.append("#page " + page["stance"] + "\n" + text.rstrip("\r\n"))
    text = "\n".join(chunks) + "\n"
    parsed = corpus.parse_macro(text)
    if parsed["parse_status"] != "parsed_subset":
        raise ValueError("constructed program is outside the conservative native subset")
    record = selected[0]
    return {"program_id": selection["program_id"], "source_family": record["lineage_group_id"],
            "split": record["split"], "mount": MOUNTS.get(record["mount"]), "season": record["season"],
            "macro_text": text, "macro_hash": corpus.digest(text), "records": selected,
            "composition": {"origin": "project_constructed_native_stance_pages", "source_blocks_flattened": False,
                            "author_configuration_reconstructed": False, "pages": copy.deepcopy(selection["pages"]),
                            "evidence": selection.get("composition_evidence")}}


def validate_scene(request, program):
    if not set(CONTRACT_KEYS) <= request.keys():
        raise ValueError("explicit version/mount/environment/horizon/tolerance/acceptance required")
    if request["version"] not in SUPPORTED or SUPPORTED[request["version"]] != program["season"]:
        raise ValueError("source season and constructed scene do not match a supported version")
    if request["mount"] != program["mount"] or request["acceptance"] != "skills_and_time":
        raise ValueError("mount and exact acceptance must match")
    for field, minimum, maximum in (("horizon", 0, 1200), ("time_tolerance_seconds", 1e-7, .125)):
        number = request[field]
        if not isinstance(number, (int, float)) or not math.isfinite(number) or not minimum <= number <= maximum or (field == "horizon" and number == 0):
            raise ValueError("invalid fixed contract " + field)
    simulation = request["simulation"]
    if not REQUIRED_ENVIRONMENT <= simulation.keys():
        raise ValueError("all simulation environment fields must be explicit")
    if not simulation["sequence"] or any(name.startswith("__") for name in simulation["sequence"]):
        raise ValueError("generation seed must be a legal manual sequence")
    if simulation["macro_text"] is not None or simulation["lite"] or simulation["pauses"]:
        raise ValueError("scene must be a full manual request")
    if simulation["macro_duration"] != request["horizon"]:
        raise ValueError("explicit generation duration must equal frozen horizon")
    if not isinstance(simulation["network_delay"], int) or isinstance(simulation["network_delay"], bool) or simulation["network_delay"] < 0:
        raise ValueError("network_delay must be a nonnegative integer in milliseconds")
    if any(key in request for key in ("candidate", "atoms", "archive_path", "stop_on_divergence", "compact_result")):
        raise ValueError("scene may contain only the target contract, not native execution/output overrides")
    for record in program["records"]:
        compatible = corpus.inspect_compatibility(record, request)
        if compatible["status"] != "catalog_compatible":
            raise ValueError("source catalog incompatible: " + "; ".join(compatible["reasons"]))


def target_from_trace(seed, generated, macro_text=None):
    actual = generated.get("actual") or []
    if generated.get("status") not in ("ok", "semantic_mismatch") or generated.get("truncated", True) or not generated.get("comparison", {}).get("completed_full_replay") or not actual:
        raise ValueError("source trace is empty, truncated, incomplete or rejected")
    if any(event["cast_time"] > seed["horizon"] + 1e-7 for event in actual):
        raise ValueError("source cast extends beyond the frozen horizon; do not crop it")
    target = copy.deepcopy(seed)
    simulation = target["simulation"]
    simulation["sequence"] = [event["name"] if 90010 <= event["skill_id"] <= 90012 else event["name"].split("·")[0] for event in actual]
    for key in ("timing_offsets", "channel_ticks", "qijin_buffs", "solidified_casts"):
        simulation[key] = {}
    if all(event.get("solidify") for event in actual):
        simulation["solidified_casts"] = {str(index): event["solidify"] for index, event in enumerate(actual)}
        construction = "native concrete casts and relative event checkpoints"
    else:
        if any(event.get("channel_ticks") is not None or event.get("channel_duration") is not None for event in actual):
            raise ValueError("channel trace needs complete native frozen casts")
        parsed = corpus.parse_macro(macro_text or "")
        if parsed["parse_status"] != "parsed_subset":
            raise ValueError("source page/command identity is required for inferred checkpoints")
        previous_time, first_main = 0.0, True
        for index, event in enumerate(actual):
            try:
                line = parsed["ast"]["pages"][event["macro_page"] - 1]["lines"][event["macro_line"] - 1]
            except (KeyError, IndexError, TypeError):
                raise ValueError("native source page/line identity is absent or invalid") from None
            # A source command may resolve a combo-follow action with a different
            # name. Keep the native concrete name/id; full manual reconstruction
            # and subsequent source certification decide whether it is legal.
            delay = simulation["network_delay"] / 1000 if event["is_main"] and not first_main else 0.0
            decision_time = event["cast_time"] - delay
            if decision_time < previous_time - 1e-7:
                raise ValueError("inferred checkpoint would run backwards")
            wait = max(0.0, decision_time - previous_time)
            simulation["solidified_casts"][str(index)] = {"skill_id": event["skill_id"], "waits": [wait] if wait else [], "fcast": line["command"] == "fcast"}
            previous_time = event["cast_time"]
            if event["is_main"]:
                first_main = False
        construction = "nonchannel native cast times as relative checkpoint hypotheses, independently fully certified"
    return target, construction


def produce_case(program, variant, out, runner, oracle_hash):
    out = corpus.output_path(out)
    seed = copy.deepcopy(variant["request"])
    validate_scene(seed, program)
    identity = corpus.digest(corpus.canonical({"source": program["macro_hash"], "scene": seed, "oracle": oracle_hash,
                                             "construction_version": TARGET_CONSTRUCTION_VERSION}))[:24]
    folder = corpus.output_path(Path(out) / "cases" / identity)
    folder.mkdir(parents=True, exist_ok=True)
    (folder / "source-macro.txt").write_text(program["macro_text"], encoding="utf-8")
    corpus.write_json(folder / "generation-scene.json", seed)
    frozen = {key: copy.deepcopy(seed[key]) for key in CONTRACT_KEYS if key != "simulation"}
    native = dict(copy.deepcopy(seed), candidate=program["macro_text"], atoms=["rage<0"],
                  compact_result=False, stop_on_divergence=False)
    start = time.perf_counter()
    generated = runner(native)
    generation_seconds = time.perf_counter() - start
    if any(native[key] != seed[key] for key in CONTRACT_KEYS):
        raise RuntimeError("source generation mutated the fixed contract")
    corpus.write_json(folder / "source-trace.json", generated)
    evidence = {"case_id": identity, "program_id": program["program_id"], "variant_id": variant["variant_id"],
                "source_family": program["source_family"], "split": program["split"],
                "version": seed["version"], "mount": seed["mount"], "fixed_contract": frozen,
                "oracle_executable_hash": oracle_hash, "generation_seconds": generation_seconds,
                "author_configuration_reconstructed": False, "scenario_origin": "explicit_project_constructed_configuration",
                "source_body_in_target": False, "source_raw_hashes": [record["raw_text_hash"] for record in program["records"]],
                "source_record_ids": [record["record_id"] for record in program["records"]], "composition": program["composition"]}
    try:
        target, construction = target_from_trace(seed, generated, program["macro_text"])
    except ValueError as error:
        evidence.update(certified=False, limitation=str(error), stage="target_construction")
        corpus.write_json(folder / "case.json", evidence)
        return None, evidence
    if any(target[key] != value for key, value in frozen.items()):
        raise RuntimeError("target construction changed the fixed contract")
    corpus.write_json(folder / "scene.json", target)
    native = dict(copy.deepcopy(target), candidate=program["macro_text"], atoms=["rage<0"],
                  compact_result=False, stop_on_divergence=False)
    start = time.perf_counter()
    result = runner(native)  # Independent full replay; never accept trace generation itself.
    certification_seconds = time.perf_counter() - start
    corpus.write_json(folder / "certification.json", result)
    if any(native[key] != target[key] for key in CONTRACT_KEYS):
        raise RuntimeError("certification mutated the target contract")
    passed = corpus.certified(result)
    evidence.update(certified=passed, target_construction=construction,
                    active_count=len(generated["actual"]), certification_seconds=certification_seconds,
                    contract_hash=corpus.digest(corpus.canonical(target)), comparison=result.get("comparison"),
                    stage="full_certification")
    corpus.write_json(folder / "case.json", evidence)
    if not passed:
        return None, evidence
    relative = folder.relative_to(out)
    item = {key: evidence[key] for key in ("case_id", "source_family", "split", "version", "mount", "active_count", "contract_hash")}
    item.update(scene_path=(relative / "scene.json").as_posix(), macro_path=(relative / "source-macro.txt").as_posix(),
                evidence_path=(relative / "case.json").as_posix(), certified=True,
                macro_has_native_stance_pages=True, author_configuration_reconstructed=False,
                scenario_origin=evidence["scenario_origin"], source_macro_role="offline_target_production_and_certification_only",
                program_id=program["program_id"], variant_id=variant["variant_id"])
    return item, evidence


def produce(manifest, review, plan, out, exe, runner=None):
    out = corpus.output_path(out)
    eligible = reviewed_records(manifest, review)
    programs = [compose_program(selection, eligible) for selection in plan["programs"]]
    if len({program["program_id"] for program in programs}) != len(programs):
        raise ValueError("duplicate program identity")
    if len({variant["variant_id"] for variant in plan["variants"]}) != len(plan["variants"]):
        raise ValueError("duplicate configuration identity")
    out.mkdir(parents=True, exist_ok=True)
    exe = Path(exe).resolve()
    oracle_hash = corpus.digest(exe.read_bytes())
    if runner is None:
        runner = lambda request: corpus.oracle_run(exe, request)
    corpus.write_json(out / "source-review.json", review)
    corpus.write_json(out / "production-plan.json", plan)
    families = {}
    for record in manifest["records"]:
        family = families.setdefault(record["lineage_group_id"], {"source_family": record["lineage_group_id"],
                                      "source_item_ids": set(), "splits": set(), "reference_b_statuses": set()})
        family["source_item_ids"].add(record["source_item_id"])
        family["splits"].add(eligible.get(record["record_id"], record)["split"])
        family["reference_b_statuses"].add(eligible.get(record["record_id"], record)["reference_b_status"])
    split_rows = [{"source_family": value["source_family"], "source_item_ids": sorted(value["source_item_ids"]),
                   "split": next(iter(value["splits"])) if len(value["splits"]) == 1 else "mixed_reserved",
                   "reference_b_statuses": sorted(value["reference_b_statuses"])} for value in families.values()]
    reserved_families = sorted({row["source_family"] for row in split_rows if row["split"] in ("test", "quarantine", "mixed_reserved")
                               or set(row["source_item_ids"]) & RESERVED_SOURCE_IDS} | {"holdout:fixed325", "fixed325"}
                              | set(review.get("heldout_source_families", [])))
    corpus.write_json(out / "split-manifest.json", {"schema_version": 1, "source_families": split_rows,
                       "reserved_source_families": reserved_families,
                       "assignment_policy": review.get("assignment_policy"), "source_review_path": "source-review.json",
                       "source_corpus_dataset_hash": manifest.get("dataset_hash"),
                       "source_bodies_in_split_manifest": False})
    items, failures, library = [], [], []
    for program in programs:
        folder = out / "programs" / corpus.digest(program["program_id"])[:20]
        folder.mkdir(parents=True, exist_ok=True)
        (folder / "macro.txt").write_text(program["macro_text"], encoding="utf-8")
        for record in program["records"]:
            (folder / ("block-" + str(record["macro_blocks"][0]["source_block_index"]) + ".txt")).write_text(record["macro_blocks"][0]["raw_text"], encoding="utf-8")
        corpus.write_json(folder / "composition.json", program["composition"])
        added = False
        for variant in plan["variants"]:
            if variant["request"]["mount"] != program["mount"] or SUPPORTED.get(variant["request"]["version"]) != program["season"]:
                continue
            item, evidence = produce_case(program, variant, out, runner, oracle_hash)
            if item:
                if any(previous["case_id"] == item["case_id"] for previous in items):
                    raise ValueError("identical source/configuration case cannot be augmented twice")
                items.append(item)
                added = True
            else:
                failures.append({key: evidence.get(key) for key in ("case_id", "program_id", "variant_id", "source_family", "split", "stage", "limitation", "comparison")})
            print(corpus.canonical({"case_id": evidence["case_id"], "split": evidence["split"], "certified": evidence["certified"], "active_count": evidence.get("active_count"), "limitation": evidence.get("limitation")}), flush=True)
        if added:
            library.append({"source_family": program["source_family"], "split": program["split"], "macro_path": (folder.relative_to(out) / "macro.txt").as_posix(),
                            "version": next(version for version, season in SUPPORTED.items() if season == program["season"]), "mount": program["mount"],
                            "program_id": program["program_id"], "macro_has_native_stance_pages": True})
    if corpus.digest(exe.read_bytes()) != oracle_hash:
        raise RuntimeError("native executable changed during corpus production")
    index = {"schema_version": SCHEMA_VERSION, "cases": items, "records": library, "failures": failures,
             "oracle_executable_hash": oracle_hash,
             "source_review_path": "source-review.json", "split_manifest_path": "split-manifest.json",
             "target_construction_version": TARGET_CONSTRUCTION_VERSION,
             "reserved_source_families": reserved_families,
             "split_policy": "Reviewed source families assigned before configuration augmentation; no snapshot split",
             "reserved_source_ids": sorted(RESERVED_SOURCE_IDS | set(map(str, review.get("heldout_source_ids", [])))),
             "runtime_answer_retrieval_allowed": False, "author_configuration_reconstructed": False,
             "summary": {"certified_cases": len(items), "failed_cases": len(failures),
                         "source_families": len({item["source_family"] for item in items}),
                         "split_cases": {split: sum(item["split"] == split for item in items) for split in ("train", "validation")},
                         "split_families": {split: len({item["source_family"] for item in items if item["split"] == split}) for split in ("train", "validation")}}}
    corpus.write_json(out / "index.json", index)
    return index


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--review", type=Path, required=True)
    parser.add_argument("--plan", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--exe", type=Path, default=ROOT / "backend/target/release/jx3-combat-sim.exe")
    args = parser.parse_args(argv)
    for path in (args.manifest, args.review, args.plan):
        corpus.output_path(path)
    index = produce(load_json(args.manifest), load_json(args.review), load_json(args.plan), args.out, args.exe)
    print(corpus.canonical(index["summary"]))


if __name__ == "__main__":
    main()
