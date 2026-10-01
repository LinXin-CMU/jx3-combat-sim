"""Learning changes scheduling only; native verdicts remain authoritative."""
import copy
import importlib.util
import json
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("exact_learning", ROOT / "tools/exact_macro_learning.py")
LEARNING = importlib.util.module_from_spec(spec)
spec.loader.exec_module(LEARNING)


def passed(ms=20):
    return {"status": "ok", "truncated": False, "actual_fingerprint": "actual-path",
            "comparison": {"reproduced": True, "completed_full_replay": True,
                           "acceptance_prefix": 12}, "timings_ms": {"verification_total": ms}}


def failed(ms=10):
    return {"status": "ok", "truncated": False, "probe_failure": {"kind": "cast_mismatch"},
            "comparison": {"reproduced": False, "completed_full_replay": False,
                           "acceptance_prefix": 5, "first_difference": {"index": 5}},
            "timings_ms": {"round_trip": ms}}


class LearningTests(unittest.TestCase):
    def setUp(self):
        self.atoms = ["rage>5", "bufftime:某时钟<5.0", "buff:某增益>2", "tnobuff:某状态"]
        self.source = [{"action": 0, "atoms": [0]}, {"action": 1, "atoms": [1, 2]}]
        self.trial = [{"action": 1, "atoms": [1, 2], "ops": ["|"]}]
        self.values = LEARNING.features(self.source, self.trial, self.atoms, 100, 70)

    def session(self, **kwargs):
        return LEARNING.LearningSession("contract", "oracle-v1", "lineage-group", **kwargs)

    def prepare(self, session, identity="candidate", before=100, after=70, path="source-path", kind="window"):
        return session.prepare(identity, path, kind, self.values, before, after)

    def feedback(self, session, record, replay, **kwargs):
        return session.feedback(record, replay, binding=dict(record["binding"]), **kwargs)

    def test_structural_schema_ignores_names_and_consistent_action_atom_reindexing(self):
        renamed = ["rage>5", "bufftime:另一时钟<5.0", "buff:另一增益>2", "tnobuff:另一状态"]
        source = [{"action": 42 if r["action"] else 17, "atoms": r["atoms"]} for r in self.source]
        trial = [{"action": 42, "atoms": [1, 2], "ops": ["|"]}]
        self.assertEqual(self.values, LEARNING.features(source, trial, renamed, 100, 70))
        permutation = [2, 0, 3, 1]
        inverse = {old: new for new, old in enumerate(permutation)}
        def remap(program):
            return [dict(r, atoms=[inverse[i] for i in r["atoms"]]) for r in program]
        self.assertEqual(self.values, LEARNING.features(remap(self.source), remap(self.trial),
                          [self.atoms[i] for i in permutation], 100, 70))
        self.assertEqual(set(self.values), set(LEARNING.FEATURE_NAMES))
        self.assertTrue(all(-1 <= value <= 1 for value in self.values.values()))
        serialized = json.dumps(self.values, ensure_ascii=False)
        self.assertNotIn("某增益", serialized)

    def test_post_replay_fields_nonfinite_and_invalid_chains_are_rejected(self):
        for key in ("matched_prefix", "actual_fingerprint", "first_difference", "author", "skill_name"):
            with self.subTest(key=key), self.assertRaises(ValueError):
                LEARNING.features(self.source, self.trial, self.atoms, 100, 70, {key: 1})
        with self.assertRaises(ValueError):
            LEARNING.features(self.source, self.trial, self.atoms, float("nan"), 70)
        with self.assertRaises(ValueError):
            LEARNING.features(self.source, [{"action": 0, "atoms": [0, 1], "ops": []}], self.atoms, 100, 70)

    def test_unknown_truncated_cancelled_static_and_infrastructure_never_train_negative(self):
        session = self.session(enabled=True)
        cases = [(dict(passed(), truncated=True), {}, "truncated"),
                 (passed(), {"cancelled": True}, "cancelled"),
                 (failed(), {"solver_status": "UNKNOWN"}, "solver_unknown"),
                 (failed(), {"static_rejected": True}, "static_rejection"),
                 (failed(), {"native": False}, "not_native"),
                 (dict(failed(), status="error"), {}, "infrastructure_error"),
                 (dict(failed(), status="unknown"), {}, "solver_unknown"),
                 (dict(failed(), status="budget_exhausted"), {}, "observation_unknown"),
                 ({"status": "ok", "comparison": {"reproduced": False}}, {}, "uncertified"),
                 ({"status": "ok", "comparison": {"reproduced": True}}, {}, "uncertified")]
        initial = session.to_dict()
        for n, (replay, flags, category) in enumerate(cases):
            result = self.feedback(session, self.prepare(session, str(n)), replay, **flags)
            self.assertEqual(result["category"], category)
            self.assertIsNone(result["label"])
            self.assertFalse(result["trained"])
        self.assertEqual(session.labels, 0)
        self.assertEqual(session.cost_labels, 0)
        self.assertEqual(session.weights, initial["weights"])

    def test_native_divergence_and_full_failure_are_distinct_known_negative_verdicts(self):
        session = self.session()
        result = self.feedback(session, self.prepare(session, "early"), failed())
        self.assertEqual((result["category"], result["label"]), ("native_divergence", 0))
        complete = dict(failed(), probe_failure=None)
        complete["comparison"] = {"reproduced": False, "completed_full_replay": True}
        result = self.feedback(session, self.prepare(session, "full"), complete)
        self.assertEqual((result["category"], result["label"]), ("native_full_failure", 0))
        self.assertEqual(session.labels, 2)

    def test_every_feedback_binding_component_and_pre_replay_snapshot_are_checked(self):
        session = self.session()
        record = self.prepare(session)
        for key in LEARNING.BINDING_NAMES:
            binding = dict(record["binding"], **{key: "different"})
            with self.subTest(key=key), self.assertRaises(ValueError):
                session.feedback(record, passed(), binding=binding)
        altered = copy.deepcopy(record)
        altered["pre_replay_features"]["saving_fraction"] = 0.8
        with self.assertRaises(ValueError):
            self.feedback(session, altered, passed())
        self.assertEqual(record["binding"]["path_id"], "source-path")
        result = self.feedback(session, record, passed())
        self.assertEqual(result["actual_path_id"], "actual-path")

    def test_unknown_can_be_completed_later_and_completed_feedback_is_deduplicated(self):
        session = self.session()
        record = self.prepare(session)
        self.feedback(session, record, dict(passed(), truncated=True))
        self.assertEqual(session.labels, 0)
        self.assertTrue(self.feedback(session, record, passed())["trained"])
        self.assertTrue(self.feedback(session, record, passed())["duplicate"])
        self.assertEqual(session.labels, 1)
        other_path = self.prepare(session, path="other-source-path")
        self.assertTrue(self.feedback(session, other_path, passed())["trained"])
        self.assertEqual(session.labels, 2)

    def test_test_split_heldout_lineage_or_contract_and_missing_lineage_freeze_training(self):
        configurations = [dict(split="test"), dict(split="validation"), dict(training=False),
                          dict(heldout_source_groups=["lineage-group"]), dict(heldout_contracts=["contract"])]
        for kwargs in configurations:
            with self.subTest(kwargs=kwargs):
                session = self.session(enabled=True, bandit_enabled=True, **kwargs)
                result = self.feedback(session, self.prepare(session), passed())
                self.assertEqual(result["label"], 1)
                self.assertFalse(result["trained"])
                self.assertFalse(session.strategy_feedback("window", [result], 20)["trained"])
                self.assertEqual(session.labels, 0)
        session = LEARNING.LearningSession("contract", "oracle-v1")
        self.assertFalse(self.feedback(session, self.prepare(session), passed())["trained"])

    def test_optional_order_is_identity_when_off_and_enabled_rank_never_drops_candidates(self):
        disabled = self.session()
        records = [self.prepare(disabled, str(i), after=90-i) for i in range(10)]
        self.assertEqual(disabled.rank(records), records)
        session = self.session(enabled=True, exploration_fraction=0.2)
        records = [self.prepare(session, str(i), after=90-i, kind="window" if i%2 else "family") for i in range(10)]
        order = session.rank(records)
        self.assertEqual({r["id"] for r in records}, {r["id"] for r in order})
        self.assertEqual(len(order), 10)
        self.assertEqual(sum(r["exploration_choice"] for r in order), 2)
        self.assertEqual(len({r["generator_kind"] for r in order[:2]}), 2)
        self.assertTrue(all(not r["learned_prediction"] for r in records))

    def test_online_logistic_and_cost_estimate_follow_real_feedback_only(self):
        success = self.session(enabled=True, min_labels=1)
        failure = self.session(enabled=True, min_labels=1)
        for i in range(60):
            self.feedback(success, self.prepare(success, str(i)), passed(200))
            self.feedback(failure, self.prepare(failure, str(i)), failed(20))
        good, bad = self.prepare(success, "new"), self.prepare(failure, "new")
        self.assertTrue(good["learned_prediction"])
        self.assertGreater(good["predicted_probability"], bad["predicted_probability"])
        self.assertGreater(good["predicted_cost_ms"], bad["predicted_cost_ms"])
        self.assertGreater(good["predicted_probability"], 0.5)
        self.assertLess(bad["predicted_probability"], 0.5)

    def test_bandit_uses_verified_gain_net_of_alternative_candidates_and_keeps_exploration(self):
        session = self.session(bandit_enabled=True)
        names = ["window", "family", "repair"]
        session.strategy_order(names, {"duplicate_fraction": 0.5, "previous_difference_kind": "time"})
        first = self.feedback(session, self.prepare(session, "a", after=80), passed())
        second = self.feedback(session, self.prepare(session, "b", after=70), passed())
        result = session.strategy_feedback("window", [first, second, first], 20)
        self.assertEqual(result["certified_saving"], 30)
        self.assertTrue(result["trained"])
        for name in names[1:]:
            session.strategy_feedback(name, [], 200)
        visited = {session.strategy_order(names)[0] for _ in range(30)}
        self.assertEqual(visited, set(names))
        self.assertEqual(set(session.strategy_order(names)), set(names))
        forged = dict(first, label=0)
        with self.assertRaises(ValueError):
            session.strategy_feedback("window", [forged], 20)

    def test_unknown_strategy_feedback_does_not_train_and_best_change_decays_evidence(self):
        session = self.session(bandit_enabled=True)
        record = self.prepare(session)
        unknown = self.feedback(session, record, dict(passed(), truncated=True))
        self.assertIsNone(session.strategy_feedback("window", [unknown], 20)["reward"])
        self.assertFalse(session.strategy_feedback("window", [], 20, solver_status="UNKNOWN")["trained"])
        result = self.feedback(session, record, passed())
        session.strategy_order(["window", "family"], {"clock_fraction": 0.5})
        session.strategy_feedback("window", [result], 20)
        before = session.to_dict()["arms"]["window"]
        session.best_changed("new-certified-path")
        after = session.to_dict()["arms"]["window"]
        self.assertEqual(after["count"], before["count"]*0.5)
        self.assertEqual(after["weights"], [w*0.5 for w in before["weights"]])
        self.assertEqual(session.epoch, 1)

    def test_versioned_serialization_contains_no_bodies_or_pending_examples_and_rejects_mismatch(self):
        session = self.session(enabled=True, bandit_enabled=True, min_labels=1)
        result = self.feedback(session, self.prepare(session), passed())
        session.strategy_order(["window", "family"])
        session.strategy_feedback("window", [result], 20)
        data = json.loads(json.dumps(session.to_dict()))
        restored = LEARNING.LearningSession.from_dict(data, contract_hash="contract", oracle_version="oracle-v1")
        self.assertEqual(restored.to_dict(), session.to_dict())
        self.assertEqual(self.prepare(restored)["predicted_probability"], self.prepare(session)["predicted_probability"])
        for text in ("pre_replay_features", "actual_path_id", "某时钟", "candidate"):
            self.assertNotIn(text, json.dumps(data, ensure_ascii=False))
        for key in ("serialization_version", "feature_schema_version", "model_version"):
            altered = dict(data, **{key: "wrong"})
            with self.subTest(key=key), self.assertRaises(ValueError):
                LEARNING.LearningSession.from_dict(altered, contract_hash="contract", oracle_version="oracle-v1")
        with self.assertRaises(ValueError):
            LEARNING.LearningSession.from_dict(data, contract_hash="other", oracle_version="oracle-v1")
        with self.assertRaises(ValueError):
            LEARNING.LearningSession.from_dict(data, contract_hash="contract", oracle_version="other")


if __name__ == "__main__":
    unittest.main()
