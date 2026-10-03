"""Offline training keeps native feedback and source-family boundaries explicit."""
import copy
import importlib.util
from pathlib import Path
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("exact_train_tests", ROOT/"tools/exact_macro_train.py")
TRAIN = importlib.util.module_from_spec(spec)
spec.loader.exec_module(TRAIN)
LM = TRAIN._module("exact_macro_learning")


class TrainingTests(unittest.TestCase):
    def examples(self):
        result = []
        for split, family in (("train", "first-family"), ("validation", "second-family"), ("test", "third-family")):
            session = LM.LearningSession("contract-"+family, "oracle", family, training=False, split=split)
            for n in range(6):
                good = n%2 == 0
                context = dict(observed_rows=10, window_coverage=1 if good else 0.2,
                               forbidden_hits=0 if good else 8, static_compatible=good)
                values = LM.features([], [{"action": 0, "atoms": []}], [], 100, 80, context)
                record = session.prepare("candidate-"+str(n), "path-"+family, "collect", values, 100, 80)
                replay = dict(status="ok", truncated=False, comparison=dict(reproduced=good, completed_full_replay=True),
                              timings_ms=dict(round_trip=20 if good else 4))
                feedback = session.feedback(record, replay, binding=record["binding"])
                result.append(dict(case_id=family, source_family=family, split=split,
                                   version="version", mount="mount", record=record, feedback=feedback))
        return result

    def test_family_and_exact_source_copies_cannot_cross_splits(self):
        TRAIN.STORE.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=TRAIN.STORE) as directory:
            base = Path(directory)
            (base/"a.txt").write_text("/cast 技能", encoding="utf-8")
            (base/"b.txt").write_text(" /cast   技能\n", encoding="utf-8")
            common = dict(version="version", mount="mount", scene_path="scene.json")
            cases = [dict(common, case_id="a", source_family="a", split="train", macro_path="a.txt"),
                     dict(common, case_id="b", source_family="a", split="validation", macro_path="b.txt")]
            with self.assertRaisesRegex(ValueError, "family crosses"):
                TRAIN.validate_manifest(dict(schema_version=1, cases=cases), base)
            cases[1]["source_family"] = "different-family"
            with self.assertRaisesRegex(ValueError, "duplicate source"):
                TRAIN.validate_manifest(dict(schema_version=1, cases=cases), base)
            cases[1]["split"] = "train"
            cases[0]["holdout"] = True
            with self.assertRaisesRegex(ValueError, "holdout"):
                TRAIN.validate_manifest(dict(schema_version=1, cases=cases), base)

    def test_public_output_and_traversing_source_paths_are_rejected_before_io(self):
        for path in (ROOT/"docs/offline-model.json", TRAIN.STORE/"../../../docs/offline-model.json"):
            with self.subTest(path=path), self.assertRaisesRegex(ValueError, "inside"):
                TRAIN.store_path(path)
        with self.assertRaisesRegex(ValueError, "user data"):
            TRAIN.store_path(TRAIN.STORE/"userdata/model.json")
        case = dict(case_id="bad", source_family="family", split="train", version="version", mount="mount",
                    scene_path="scene.json", macro_path="../../../docs/secret.txt")
        with self.assertRaisesRegex(ValueError, "inside"):
            TRAIN.validate_manifest(dict(schema_version=1, cases=[case]), TRAIN.STORE/"fixture")

    def test_multi_page_source_is_not_implicitly_a_single_page_baseline(self):
        text = "#page [buff:甲]\n/cast [rage>3|bufftime:乙<5] 技能甲\n#page [nobuff:甲]\n/cast 技能乙"
        with self.assertRaisesRegex(ValueError, "unsupported"):
            TRAIN.parse_macro(text)
        pages = TRAIN.source_pages(text)
        self.assertEqual(len(pages), 2)
        rules, atoms, _ = TRAIN.parse_macro(pages[0])
        self.assertEqual(rules[0]["ops"], ["|"])
        self.assertEqual(atoms, ["rage>3", "bufftime:乙<5"])
        unbracketed, other_atoms, _ = TRAIN.parse_macro("/cast rage>3|bufftime:乙<5 技能甲")
        self.assertEqual(unbracketed, rules)
        self.assertEqual(other_atoms, atoms)

    def test_reverse_inflation_preserves_native_right_associated_or_tree(self):
        conditions = TRAIN._module("exact_macro_conditions")
        source = [{"action": 0, "atoms": [0, 1, 2], "ops": ["&", "|"]}]
        inflated = TRAIN.inflate(source)
        self.assertEqual(inflated[0]["ops"], ["&", "|", "&"])
        self.assertEqual(inflated[0]["atoms"], [0, 1, 2, 2])
        all_bits, truth = (1 << 8)-1, [0, 0, 0]
        for row in range(8):
            for atom in range(3):
                if row & (1 << atom):
                    truth[atom] |= 1 << row
        expected = conditions.condition_mask(source[0], truth, all_bits)
        self.assertEqual(conditions.condition_mask(inflated[0], truth, all_bits), expected)
        self.assertEqual(conditions.condition_mask(inflated[1], truth, all_bits), expected)

    def test_legacy_or_inflation_retains_action_and_repeats_the_native_last_leaf(self):
        conditions = TRAIN._module("exact_macro_conditions")
        all_bits, truth = (1 << 8)-1, [0, 0, 0]
        for row in range(8):
            for atom in range(3):
                if row & (1 << atom):
                    truth[atom] |= 1 << row
        for rule in ({"action": 7, "atoms": [0], "any_atoms": [1, 2]},
                     {"action": 9, "atoms": [], "any_atoms": [0, 1, 2]}):
            with self.subTest(rule=rule):
                original = copy.deepcopy(rule)
                condition = conditions.normalize(rule)
                inflated, retained = TRAIN.inflate([rule])
                self.assertEqual(inflated["action"], rule["action"])
                self.assertEqual(inflated["atoms"], condition["atoms"]+[condition["atoms"][-1]])
                self.assertEqual(inflated["ops"], condition["ops"]+["&"])
                self.assertEqual(conditions.condition_mask(inflated, truth, all_bits),
                                 conditions.condition_mask(rule, truth, all_bits))
                self.assertEqual(retained, original)
                self.assertEqual(rule, original)

    def test_fit_uses_only_train_labels_and_validation_never_updates(self):
        examples = self.examples()
        prior = LM.structural_prior([([{"action": 0, "atoms": []}], [])])
        model = TRAIN.fit_prior(examples, prior, epochs=12)
        changed = copy.deepcopy(examples)
        for example in changed:
            if example["split"] != "train":
                example["record"]["pre_replay_features"]["saving_fraction"] = -1
                example["feedback"]["validation_cost_ms"] = 999999
        self.assertEqual(TRAIN.fit_prior(changed, prior, epochs=12), model)
        before = copy.deepcopy(model)
        report = TRAIN.evaluate_prior(model, examples)
        self.assertEqual(model, before)
        self.assertTrue(report["frozen"])
        self.assertEqual(report["labels"], 6)
        self.assertEqual(report['static_screen']['labels'], 6)
        self.assertEqual(report['static_screen']['accuracy'], 1)
        unknown = copy.deepcopy(examples)
        for example in unknown:
            if example['split'] == 'validation':
                example['record']['pre_replay_features']['static_compatible_known'] = 0
        # Missing static context is not a failed screen or a baseline label.
        unknown_report = TRAIN.evaluate_prior(model, unknown)
        self.assertEqual(unknown_report['static_screen']['labels'], 0)
        self.assertIsNone(unknown_report['static_screen']['accuracy'])
        self.assertEqual(model["trained_labels"], 6)
        self.assertEqual(model["training_summary"]["positive_labels"], 3)
        self.assertNotIn("first-family", str(model))
        self.assertNotIn("candidate-", str(model))

    def test_static_only_train_family_is_bound_without_creating_labels_or_crossing_splits(self):
        examples = self.examples()
        prior = LM.structural_prior([([{"action": 0, "atoms": []}], [])])
        ordinary = TRAIN.fit_prior(examples, prior, epochs=2)
        model = TRAIN.fit_prior(examples, prior, epochs=2, structural_families={"static-only-family"})
        self.assertEqual(model["training_summary"]["train_family_hashes"],
                         sorted([TRAIN._hash("first-family"), TRAIN._hash("static-only-family")]))
        for key in ("weights", "cost_weights", "trained_labels", "cost_labels"):
            self.assertEqual(model[key], ordinary[key])
        self.assertEqual(model["trained_labels"], 6)
        with self.assertRaisesRegex(ValueError, "overlaps training"):
            LM.LearningSession.from_prior(model, contract_hash="unseen-contract", oracle_version="oracle",
                source_group="static-only-family", version="version", mount="mount")
        unknown = copy.deepcopy(examples[6])
        unknown["feedback"].update(label=None, category="solver_unknown")
        # Split isolation is checked before unknown feedback is filtered out.
        with self.assertRaisesRegex(ValueError, "crosses"):
            TRAIN.fit_prior(examples[:6]+[unknown], prior, epochs=2,
                            structural_families={"second-family"})

    def test_unknown_native_feedback_is_not_a_negative_training_label(self):
        examples = self.examples()
        unknown = copy.deepcopy(examples[0])
        unknown["record"]["binding"]["candidate_hash"] = "unknown"
        unknown["feedback"]["binding"] = dict(unknown["record"]["binding"])
        unknown["feedback"].update(label=None, category="solver_unknown")
        prior = LM.structural_prior([])
        model = TRAIN.fit_prior(examples+[unknown], prior, epochs=2)
        self.assertEqual(model["trained_labels"], 6)
        forged = copy.deepcopy(unknown)
        forged["feedback"]["label"] = 0
        with self.assertRaisesRegex(ValueError, "category"):
            TRAIN.fit_prior(examples+[forged], prior)
        forged = copy.deepcopy(examples[0])
        forged["feedback"]["binding"]["path_id"] = "foreign-path"
        with self.assertRaisesRegex(ValueError, "provenance"):
            TRAIN.fit_prior([forged], prior)

    def test_single_class_model_is_not_published_and_example_families_are_disjoint(self):
        prior = LM.structural_prior([])
        with self.assertRaisesRegex(ValueError, "both real"):
            TRAIN.fit_prior([e for e in self.examples() if e["feedback"]["label"]], prior)
        examples = self.examples()
        examples[6]["source_family"] = "first-family"
        with self.assertRaisesRegex(ValueError, "crosses"):
            TRAIN.fit_prior(examples, prior)


if __name__ == "__main__":
    unittest.main()
