"""Corpus boundaries: provenance, precision, family splits and native gate."""
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from urllib.error import HTTPError

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location("exact_macro_corpus", ROOT / "tools/exact_macro_corpus.py")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def source(identity=1, author=9, macro="/cast [rage>49] 绝刀", **changes):
    post = {"ID": identity, "post_author": author, "author": "public author", "post_title": "public source",
            "post_subtype": "分山劲", "client": "std", "zlp": "暗影千机", "visible": "0",
            "post_meta": {"data": [{"name": "one", "macro": macro, "talent": ""}]}}
    post.update(changes)
    return MODULE.records_for_post(post, {"collected_at": "2026-10-02T00:00:00Z", "url": "https://cms.jx3box.com/api/cms/post/1"}, {"page": 1, "per": 4})


class CorpusTests(unittest.TestCase):
    def test_right_associative_and_or_and_fcast_identity(self):
        parsed = MODULE.parse_macro("/fcast [rage>49&buff:血怒|nobuff:坚定] 绝刀")
        self.assertEqual(parsed["parse_status"], "parsed_subset")
        line = parsed["ast"]["pages"][0]["lines"][0]
        tree = line["condition"]
        self.assertEqual((tree["op"], tree["right"]["op"]), ("&", "|"))
        self.assertEqual(tree["left"]["kind"], "rage")
        other = MODULE.parse_macro("/cast [rage>49&buff:血怒|nobuff:坚定] 绝刀")
        self.assertNotEqual(parsed["parsed_ast_hash"], other["parsed_ast_hash"])
        self.assertEqual(parsed["native_parse_status"], "not_checked")

    def test_original_precision_and_format_variants_are_separate_evidence(self):
        original = "/cast [bufftime:嗜血<8.8125] 绝刀\r\n"
        record = source(macro=original)[0]
        self.assertEqual(record["macro_blocks"][0]["raw_text"], original)
        self.assertEqual(record["raw_text_hash"], MODULE.digest(original))
        self.assertIn("8.8125", record["normalized_text"])
        first = MODULE.parse_macro("/cast [bufftime:嗜血<8.800] 绝刀")
        second = MODULE.parse_macro("/cast bufftime:嗜血<8.8 绝刀")
        self.assertEqual(first["parsed_ast_hash"], second["parsed_ast_hash"])
        self.assertNotEqual(record["parsed_ast_hash"], first["parsed_ast_hash"])

    def test_source_blocks_and_native_pages_are_not_flattened(self):
        blocks = [{"name": "shield", "macro": "#page shield\n/cast 盾刀\n#page blade\n/fcast 斩刀"},
                  {"name": "manual auxiliary", "macro": "/cast 血怒", "desc": "提前3秒按辅助宏"}]
        records = source(post_meta={"data": blocks})
        self.assertEqual(len(records), 2)
        self.assertEqual(len(records[0]["ast"]["pages"]), 2)
        self.assertEqual(records[0]["page_selection"]["status"], "native_explicit")
        self.assertEqual(records[1]["page_selection"]["status"], "unknown")
        self.assertFalse(records[1]["page_selection"]["source_blocks_flattened"])
        self.assertIn("提前3秒按辅助宏", records[1]["manual_steps"])
        self.assertNotIn("血怒", records[0]["normalized_text"])

    def test_unknown_metadata_is_not_guessed_from_title_or_post_date(self):
        record = source(zlp="", post_title="2026最新测试服宏", post_date="2026-10-02")[0]
        self.assertIsNone(record["season"])
        self.assertIsNone(record["test_server"])
        self.assertIsNone(record["talents"]["project_ids"])
        self.assertIsNone(record["recipes"])
        self.assertIn("season", record["missing_fields"])
        self.assertFalse(record["manual_steps_complete"])
        self.assertEqual(record["replay_status"], "not_run")

    def test_unsupported_input_stays_raw_and_has_no_ast_certificate(self):
        for macro in ("/cast [rage>49,life<0.5] 绝刀", "/skill 绝刀", "/cast [unknown:状态] 绝刀", "#page manual\n/cast 绝刀"):
            with self.subTest(macro=macro):
                record = source(macro=macro)[0]
                self.assertEqual(record["parse_status"], "unsupported")
                self.assertIsNone(record["parsed_ast_hash"])
                self.assertEqual(record["macro_blocks"][0]["raw_text"], macro)
                self.assertTrue(record["unsupported_features"])

    def test_numeric_skeleton_and_same_author_close_family_transitively(self):
        records = source(1, 8, "/cast [bufftime:嗜血<8.8] 绝刀")
        records += source(2, 9, "/cast [bufftime:嗜血<8.9] 绝刀")
        records += source(3, 9, "/cast 盾刀")
        MODULE.assign_families(records)
        self.assertEqual(len({record["lineage_group_id"] for record in records}), 1)
        self.assertEqual(records[0]["lineage_source_ids"], ["1", "2", "3"])

    def test_quarantine_propagates_to_siblings_and_numeric_variants(self):
        records = source(1, 8, "/cast [rage>49] 绝刀", post_title="武学助手对照")
        records += source(2, 9, "/cast [rage>50] 绝刀")
        MODULE.assign_families(records, reviewed_sources=["1", "2"])
        self.assertEqual({record["split"] for record in records}, {"quarantine"})
        self.assertEqual({record["reference_b_status"] for record in records}, {"quarantined"})
        article_marker = source(3, 10, post_content="来源为武学助手宏")
        MODULE.assign_families(article_marker, reviewed_sources=["3"], test_sources=["3"])
        self.assertEqual(article_marker[0]["split"], "quarantine")

    def test_external_quarantine_fingerprint_is_a_family_boundary(self):
        records = source(1, 8) + source(2, 9, "/cast [rage>50] 绝刀")
        MODULE.assign_families(records, {"raw_text_hash": [records[0]["raw_text_hash"]]})
        self.assertEqual({record["split"] for record in records}, {"quarantine"})

    def test_explicit_derivation_links_group_and_quarantine_unfetched_ancestor(self):
        records = source(1, 8, post_content='<p>修改自 <a href="https://www.jx3box.com/macro/325">原文</a></p>')
        records += source(2, 9, "/cast 盾刀", post_content='<a href="https://www.jx3box.com/macro/325">转载</a>')
        MODULE.assign_families(records, {"source_item_id": ["325"]})
        self.assertEqual(len({record["lineage_group_id"] for record in records}), 1)
        self.assertEqual({record["split"] for record in records}, {"quarantine"})
        self.assertEqual(records[0]["derived_from_source_ids"], ["325"])
        self.assertTrue(records[0]["source_derivation_notes"])

    def test_unreviewed_sources_cannot_enter_training_and_whole_family_review_required(self):
        records = source(1, 8) + source(2, 9, "/cast [rage>50] 绝刀")
        MODULE.assign_families(records, reviewed_sources=["1"])
        self.assertEqual({record["split"] for record in records}, {"review"})
        MODULE.assign_families(records, reviewed_sources=["1", "2"])
        self.assertEqual(len({record["split"] for record in records}), 1)
        self.assertIn(records[0]["split"], ("train", "validation", "test"))
        reversed_records = copy.deepcopy(list(reversed(records)))
        MODULE.assign_families(reversed_records, reviewed_sources=["1", "2"])
        self.assertEqual(reversed_records[0]["lineage_group_id"], records[0]["lineage_group_id"])
        self.assertEqual(reversed_records[0]["split"], records[0]["split"])

    def test_explicit_test_family_and_review_resume_invalidate_changed_source(self):
        records = source(1, 8) + source(2, 9, "/cast [rage>50] 绝刀")
        reviewed, tests, saved = MODULE.split_config(records, {}, ["1", "2"], ["1"])
        MODULE.assign_families(records, reviewed_sources=reviewed, test_sources=tests)
        self.assertEqual({record["split"] for record in records}, {"test"})
        reviewed, tests, _ = MODULE.split_config(records, saved)
        self.assertEqual(reviewed, {"1", "2"})
        changed = source(1, 8, "/cast [rage>51] 绝刀") + source(2, 9, "/cast [rage>50] 绝刀")
        reviewed, tests, _ = MODULE.split_config(changed, saved)
        MODULE.assign_families(changed, reviewed_sources=reviewed, test_sources=tests)
        self.assertEqual({record["split"] for record in changed}, {"review"})

    def test_safe_projection_drops_unrelated_network_identity_and_profiles(self):
        post = {"ID": 2, "post_meta": {"data": [{"macro": "/cast 盾刀"}]}, "ip": "do not persist",
                "author_info": {"phone": "do not persist"}, "post_author": 4, "comments": ["unrelated"]}
        safe = MODULE.safe_post(post)
        self.assertNotIn("ip", safe)
        self.assertNotIn("author_info", safe)
        self.assertNotIn("comments", safe)
        self.assertEqual(safe["post_author"], 4)
        self.assertEqual(safe["post_meta"]["data"][0]["macro"], "/cast 盾刀")
        listing = MODULE.safe_listing(dict(post, visible="1", post_content="restricted body"))
        self.assertNotIn("post_meta", listing)
        self.assertNotIn("post_content", listing)

    def test_raw_output_rejects_repository_source_and_parent_traversal(self):
        self.assertEqual(MODULE.output_path(MODULE.STORE / "test"), (MODULE.STORE / "test").resolve())
        for path in (ROOT / "docs/corpus", MODULE.STORE / "../../userdata"):
            with self.assertRaises(ValueError):
                MODULE.output_path(path)

    def test_certification_requires_complete_native_replay(self):
        good = {"status": "ok", "comparison": {"reproduced": True, "completed_full_replay": True}, "truncated": False}
        self.assertTrue(MODULE.certified(good))
        for patch in ({"status": "probe_budget"}, {"truncated": True}, {"comparison": {"reproduced": True}},
                      {"comparison": {"reproduced": False, "completed_full_replay": True}}):
            failed = dict(good, **patch)
            self.assertFalse(MODULE.certified(failed))

    def test_versioned_catalog_rejects_unknown_names_and_season_transfer(self):
        record = source(macro="/cast 盾刀")[0]
        request = {"version": "AnYingQianJi", "mount": "FenShanJin"}
        self.assertEqual(MODULE.inspect_compatibility(record, request)["status"], "catalog_compatible")
        unknown = source(macro="/cast [nobuff:未知气劲] 未知技能")[0]
        rejected = MODULE.inspect_compatibility(unknown, request)
        self.assertEqual(rejected["status"], "incompatible_or_unknown")
        self.assertTrue(any("未知气劲" in reason for reason in rejected["reasons"]))
        self.assertTrue(any("未知技能" in reason for reason in rejected["reasons"]))
        record["season"] = None
        self.assertEqual(MODULE.inspect_compatibility(record, request)["status"], "incompatible_or_unknown")
        request["version"] = "CangShengZhuShiTest"
        self.assertEqual(MODULE.inspect_compatibility(record, request)["status"], "unsupported_version")

    def test_pagination_merge_preserves_old_sources_but_not_stale_certificates(self):
        old = source(1, 8)
        old[0]["certification"] = {"certified": True}
        old[0]["replay_status"] = "certified_project_constructed"
        other_page = source(2, 9, "/cast 盾刀")
        merged = MODULE.merge_records(old, other_page)
        self.assertEqual(len(merged), 2)
        unchanged = MODULE.merge_records(merged, source(1, 8))
        self.assertEqual(unchanged[0]["certification"], {"certified": True})
        changed = MODULE.merge_records(merged, source(1, 8, "/cast [rage>50] 绝刀"))
        self.assertIsNone(changed[0]["certification"])
        self.assertEqual(changed[0]["replay_status"], "not_run")

    def test_exported_schema_contains_unknown_and_separate_native_status(self):
        schema = MODULE.record_schema()
        self.assertIn("null", schema["properties"]["season"]["type"])
        self.assertIn("native_parse_status", schema["required"])
        self.assertIn("review", schema["properties"]["split"]["enum"])
        self.assertIn("raw_text", schema["properties"]["macro_blocks"]["items"]["required"])

    def test_public_get_caches_safe_projection_and_resumes_without_authentication(self):
        payload = {"code": 0, "data": {"ID": 1, "visible": "0", "post_status": "publish", "post_meta": {"data": []}, "ip": "discard", "author_info": {"phone": "discard"}}}
        class Response:
            def __enter__(self): return self
            def __exit__(self, *args): pass
            def read(self, maximum): return json.dumps(payload).encode()
        url = "https://cms.jx3box.com/api/cms/post/1"
        with tempfile.TemporaryDirectory() as folder, patch.object(MODULE, "urlopen", return_value=Response()) as fetch, patch.object(MODULE.time, "sleep"):
            client = MODULE.PublicClient(Path(folder))
            first = client.get(url, "detail")
            second = client.get(url, "detail")
            self.assertEqual(first, second)
            self.assertEqual(fetch.call_count, 1)
            self.assertNotIn("ip", first)
            self.assertNotIn("author_info", first)
            self.assertTrue(client.requests[-1]["from_cache"])
            self.assertTrue(list((Path(folder) / "sources/1").glob("*.json")))
            headers = {key.lower() for key in fetch.call_args.args[0].headers}
            self.assertFalse(headers & {"cookie", "authorization"})

    def test_access_restriction_stops_without_retry_or_cached_content(self):
        url = "https://cms.jx3box.com/api/cms/post/1"
        error = HTTPError(url, 401, "login required", {}, None)
        self.addCleanup(error.close)
        with tempfile.TemporaryDirectory() as folder, patch.object(MODULE, "urlopen", side_effect=error) as fetch, patch.object(MODULE.time, "sleep"):
            client = MODULE.PublicClient(Path(folder), retries=2)
            with self.assertRaisesRegex(ValueError, "access restriction"):
                client.get(url, "detail")
            self.assertEqual(fetch.call_count, 1)
            self.assertFalse((Path(folder) / "cache").exists())
            self.assertEqual(client.requests[-1]["status"], "access_restricted")

    def test_remote_detail_identity_cannot_choose_a_storage_path(self):
        class Response:
            def __enter__(self): return self
            def __exit__(self, *args): pass
            def read(self, maximum):
                return json.dumps({"code": 0, "data": {"ID": "../../userdata", "visible": "0", "post_status": "publish"}}).encode()
        with tempfile.TemporaryDirectory() as folder, patch.object(MODULE, "urlopen", return_value=Response()), patch.object(MODULE.time, "sleep"):
            with self.assertRaisesRegex(ValueError, "identity"):
                MODULE.PublicClient(Path(folder)).get("https://cms.jx3box.com/api/cms/post/1", "detail")
            self.assertFalse(list(Path(folder).glob("**/*.json")))


if __name__ == "__main__":
    unittest.main()
