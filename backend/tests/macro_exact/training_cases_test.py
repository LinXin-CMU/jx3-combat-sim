"""Offline cases keep source lineage, constructed targets and native proof separate."""
import copy
from contextlib import contextmanager
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location("exact_macro_training_cases_tested", ROOT / "tools/exact_macro_training_cases.py")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def record(identity="1", author="11", text="/cast [rage>49&nobuff:坚定|bufftime:嗜血<7] 盾刀"):
    post = {"ID": identity, "post_author": author, "post_title": "public", "post_subtype": "分山劲",
            "client": "std", "zlp": "暗影千机", "post_meta": {"data": [{"name": "shield", "macro": text}]}}
    return MODULE.corpus.records_for_post(post, {"collected_at": "2026-10-03"}, {})[0]


def manifest_and_review():
    records = [record(), record("2", "12", "/fcast 斩刀")]
    MODULE.corpus.assign_families(records)
    # Existing test decisions remain authoritative; these synthetic records are
    # the intentionally reviewed pilot, not final tests.
    for item in records:
        item["split"] = "review"
    versions = MODULE.source_versions(records)
    review = {"sources": [{"source_item_id": item["source_item_id"], "source_version_hash": versions[item["source_item_id"]],
                           "reference_b_status": "reviewed_non_b", "split": "train" if number == 0 else "validation",
                           "evidence": {"scope": "cached public body reviewed"}} for number, item in enumerate(records)]}
    return {"records": records}, review


def program():
    manifest, review = manifest_and_review()
    eligible = MODULE.reviewed_records(manifest, review)
    return MODULE.compose_program({"program_id": "synthetic", "pages": [{"record_id": "jx3box:1:block:0", "stance": "shield"}]}, eligible)


def scene():
    value = json.loads((ROOT / "backend/tests/fixtures/exact_macro_short.json").read_text(encoding="utf-8"))
    value.update(version="AnYingQianJi", horizon=8, time_tolerance_seconds=.0625, acceptance="skills_and_time")
    value["simulation"].update(sequence=["盾刀"], talents=[], recipes=[], macro_duration=8, network_delay=0)
    return value


def trace(events=None, **changes):
    if events is None:
        events = [{"name": "盾刀·一段", "skill_id": 13044, "cast_time": 0, "is_main": True,
                   "macro_page": 1, "macro_line": 1, "solidify": None, "channel_ticks": None, "channel_duration": None}]
    return dict({"status": "ok", "comparison": {"completed_full_replay": True, "reproduced": False},
                 "truncated": False, "actual": events}, **changes)


def certification(**changes):
    return dict({"status": "ok", "comparison": {"completed_full_replay": True, "reproduced": True}, "truncated": False}, **changes)


@contextmanager
def temporary_store():
    # A clean checkout has no cached/ignored corpus directory. Tests use their
    # own temporary store and never require or alter real research evidence.
    with tempfile.TemporaryDirectory() as folder, patch.object(MODULE.corpus, "STORE", Path(folder)):
        yield Path(folder)


class TrainingCaseTests(unittest.TestCase):
    def test_explicit_heldout_family_is_rejected_before_production(self):
        manifest, review = manifest_and_review()
        review['heldout_source_families'] = [manifest['records'][0]['lineage_group_id']]
        with self.assertRaisesRegex(ValueError, 'reserved/quarantined'):
            MODULE.reviewed_records(manifest, review)

    def test_whole_family_closure_and_split_are_checked_before_augmentation(self):
        manifest, review = manifest_and_review()
        manifest["records"][1]["lineage_group_id"] = manifest["records"][0]["lineage_group_id"]
        with self.assertRaisesRegex(ValueError, "cannot cross splits"):
            MODULE.reviewed_records(manifest, review)
        review["sources"].pop()
        with self.assertRaisesRegex(ValueError, "every observed source"):
            MODULE.reviewed_records(manifest, review)

    def test_source_context_change_invalidates_review(self):
        manifest, review = manifest_and_review()
        manifest["records"][0]["source_context_hash"] = "changed"
        with self.assertRaisesRegex(ValueError, "stale"):
            MODULE.reviewed_records(manifest, review)

    def test_reserved_source_and_quarantine_propagate_to_all_siblings(self):
        for reserved in ("101545", "106800"):
            manifest, review = manifest_and_review()
            item = record(reserved, "other", "/cast 盾压")
            item["lineage_group_id"] = manifest["records"][0]["lineage_group_id"]
            manifest["records"].append(item)
            with self.assertRaisesRegex(ValueError, "reserved/quarantined"):
                MODULE.reviewed_records(manifest, review)
        for field, value in (("split", "test"), ("split", "quarantine"), ("reference_b_marker_in_source", True)):
            manifest, review = manifest_and_review()
            manifest["records"][0][field] = value
            if field == "reference_b_marker_in_source":
                review["sources"][0]["source_version_hash"] = MODULE.source_versions(manifest["records"])["1"]
            with self.assertRaisesRegex(ValueError, "reserved/quarantined"):
                MODULE.reviewed_records(manifest, review)

    def test_page_composition_preserves_native_chain_and_requires_evidence(self):
        manifest, review = manifest_and_review()
        second = record("1", "11", "/fcast [rage<50] 盾回")
        second["record_id"] = "jx3box:1:block:1"
        second["macro_blocks"][0]["source_block_index"] = 1
        manifest["records"].append(second)
        MODULE.corpus.assign_families(manifest["records"])
        for item in manifest["records"]:
            item["split"] = "review"
        review["sources"][0]["source_version_hash"] = MODULE.source_versions(manifest["records"])["1"]
        eligible = MODULE.reviewed_records(manifest, review)
        selection = {"program_id": "pair", "pages": [{"record_id": "jx3box:1:block:0", "stance": "shield"},
                                                          {"record_id": "jx3box:1:block:1", "stance": "blade"}]}
        with self.assertRaisesRegex(ValueError, "page-use evidence"):
            MODULE.compose_program(selection, eligible)
        selection.update(composition_reviewed=True, composition_evidence="explicit source shield/blade usage")
        made = MODULE.compose_program(selection, eligible)
        self.assertIn(manifest["records"][0]["macro_blocks"][0]["raw_text"], made["macro_text"])
        self.assertIn("#page shield\n", made["macro_text"])
        self.assertIn("#page blade\n/fcast [rage<50] 盾回", made["macro_text"])
        tree = MODULE.corpus.parse_macro(made["macro_text"])["ast"]["pages"][0]["lines"][0]["condition"]
        self.assertEqual((tree["op"], tree["right"]["op"]), ("&", "|"))
        self.assertFalse(made["composition"]["source_blocks_flattened"])
        self.assertFalse(made["composition"]["author_configuration_reconstructed"])

    def test_explicit_scene_contract_rejects_delay_units_and_execution_overrides(self):
        made, request = program(), scene()
        with patch.object(MODULE.corpus, "inspect_compatibility", return_value={"status": "catalog_compatible"}):
            MODULE.validate_scene(request, made)
            for change in ({"horizon": 9}, {"acceptance": "skills_and_state"}, {"archive_path": "elsewhere"}):
                with self.assertRaises(ValueError):
                    MODULE.validate_scene(dict(request, **change), made)
            request["simulation"]["network_delay"] = .0625
            with self.assertRaisesRegex(ValueError, "milliseconds"):
                MODULE.validate_scene(request, made)

    def test_relative_waits_account_for_delay_and_fcast_and_keep_concrete_combo(self):
        request = scene()
        request["simulation"]["network_delay"] = 62
        text = "#page shield\n/cast 阵云结晦\n/fcast 血怒\n"
        events = [{"name": "阵云结晦", "skill_id": 30769, "cast_time": 0, "is_main": True, "macro_page": 1, "macro_line": 1},
                  {"name": "血怒", "skill_id": 13040, "cast_time": 1.5, "is_main": False, "macro_page": 1, "macro_line": 2},
                  {"name": "月照连营", "skill_id": 30855, "cast_time": 2.062, "is_main": True, "macro_page": 1, "macro_line": 1}]
        target, _ = MODULE.target_from_trace(request, trace(events), text)
        frozen = target["simulation"]["solidified_casts"]
        self.assertEqual(frozen["0"]["waits"], [])
        self.assertEqual(frozen["1"], {"skill_id": 13040, "waits": [1.5], "fcast": True})
        self.assertAlmostEqual(frozen["2"]["waits"][0], .5)
        self.assertEqual(target["simulation"]["sequence"][2], "月照连营")
        self.assertEqual(target["time_tolerance_seconds"], request["time_tolerance_seconds"])
        self.assertNotIn("candidate", target)
        self.assertNotIn("macro_text", json.dumps({key: target[key] for key in MODULE.CONTRACT_KEYS if key != "simulation"}))
        self.assertIsNone(target["simulation"]["macro_text"])

    def test_channel_incomplete_empty_and_over_horizon_are_never_cropped_or_frozen(self):
        request, text = scene(), "#page shield\n/cast 盾刀"
        for result in (trace([]), trace(truncated=True), trace(comparison={"completed_full_replay": False}),
                       trace([{**trace()["actual"][0], "cast_time": 8.1}]),
                       trace([{**trace()["actual"][0], "channel_ticks": 3}])):
            with self.assertRaises(ValueError):
                MODULE.target_from_trace(request, result, text)

    def test_generated_trace_is_not_a_training_label_without_second_full_certificate(self):
        made = program()
        with temporary_store() as folder, patch.object(MODULE.corpus, "inspect_compatibility", return_value={"status": "catalog_compatible"}):
            calls = []
            def runner(request):
                calls.append(copy.deepcopy(request))
                return trace() if len(calls) == 1 else certification(comparison={"reproduced": True, "completed_full_replay": False})
            item, evidence = MODULE.produce_case(made, {"variant_id": "short", "request": scene()}, Path(folder), runner, "oracle")
            self.assertIsNone(item)
            self.assertFalse(evidence["certified"])
            self.assertEqual(len(calls), 2)
            self.assertTrue(all(call["stop_on_divergence"] is False and call["compact_result"] is False for call in calls))
            self.assertEqual(calls[0]["horizon"], calls[1]["horizon"])
            self.assertEqual(calls[0]["time_tolerance_seconds"], calls[1]["time_tolerance_seconds"])
            self.assertEqual(calls[0]["candidate"], calls[1]["candidate"])

    def test_complete_certificate_exports_relative_paths_and_contract_hash(self):
        made = program()
        with temporary_store() as folder, patch.object(MODULE.corpus, "inspect_compatibility", return_value={"status": "catalog_compatible"}):
            answers = iter((trace(), certification()))
            item, evidence = MODULE.produce_case(made, {"variant_id": "short", "request": scene()}, Path(folder), lambda _: next(answers), "oracle")
            self.assertTrue(evidence["certified"])
            self.assertEqual(item["split"], "train")
            self.assertFalse(Path(item["scene_path"]).is_absolute())
            target = MODULE.load_json(Path(folder) / item["scene_path"])
            self.assertEqual(item["contract_hash"], MODULE.corpus.digest(MODULE.corpus.canonical(target)))
            self.assertFalse(item["author_configuration_reconstructed"])

    def test_native_mutation_cannot_silently_relax_generation_or_certification(self):
        for mutation_call in (1, 2):
            made = program()
            with temporary_store() as folder, patch.object(MODULE.corpus, "inspect_compatibility", return_value={"status": "catalog_compatible"}):
                calls = []
                def runner(request):
                    calls.append(request)
                    if len(calls) == mutation_call:
                        request["time_tolerance_seconds"] = .125
                    return trace() if len(calls) == 1 else certification()
                with self.assertRaisesRegex(RuntimeError, "mutated"):
                    MODULE.produce_case(made, {"variant_id": "short", "request": scene()}, Path(folder), runner, "oracle")

    def test_raw_artifacts_cannot_escape_ignored_store(self):
        with self.assertRaises(ValueError):
            MODULE.corpus.output_path(ROOT / "docs/training-data")


if __name__ == "__main__":
    unittest.main()
