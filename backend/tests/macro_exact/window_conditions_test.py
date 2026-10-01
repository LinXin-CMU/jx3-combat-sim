"""Existential release-window conditions remain hypotheses for native replay."""
import importlib.util
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("window_conditions", ROOT / "tools/exact_macro_conditions.py")
CONDITIONS = importlib.util.module_from_spec(spec)
spec.loader.exec_module(CONDITIONS)


class Samples:
    def __init__(self, truth, all_mask, costs=None, atoms=None):
        self.truth, self.all = list(truth), all_mask
        self.atom_costs = list(costs) if costs is not None else [2] * len(truth)
        self.atoms = list(atoms) if atoms is not None else ["feature" + str(i) for i in range(len(truth))]
        self.check_count = 0

    def check(self):
        self.check_count += 1


class WindowGuardsTests(unittest.TestCase):
    def guards(self, windows, negative, samples, **kwargs):
        result = CONDITIONS.window_guards(windows, negative, samples, **kwargs)
        keys = [CONDITIONS.condition_key(rule) for rule in result]
        self.assertEqual(len(keys), len(set(keys)))
        for rule in result:
            mask = CONDITIONS.condition_mask(rule, samples.truth, samples.all)
            self.assertFalse(mask & negative)
            self.assertTrue(all(mask & window for window in windows))
            self.assertEqual(len(rule["ops"]), max(0, len(rule["atoms"]) - 1))
        return result

    def test_one_witness_per_window_does_not_force_all_positive_rows(self):
        samples = Samples([1 | 4, 16], 31)
        windows = [1 | 2, 4 | 8]
        result = self.guards(windows, 16, samples, max_terms=1)
        self.assertEqual(result, [{"atoms": [0], "ops": []}])
        self.assertEqual(CONDITIONS.short_guards(15, 16, samples, max_terms=1), [])

    def test_and_cannot_mix_mutually_exclusive_states_into_one_witness(self):
        # Both features individually touch both windows, but each has a
        # forbidden state. Their AND avoids negatives while missing window 0:
        # feature0 is true at its state0; feature1 is true at its state1.
        samples = Samples([1 | 4 | 16, 2 | 4 | 32], 63)
        self.assertEqual(self.guards([3, 12], 48, samples, max_terms=3), [])

    def test_and_can_choose_a_common_row_without_covering_the_other_window_rows(self):
        samples = Samples([1 | 4 | 16, 1 | 4 | 32], 63)
        result = self.guards([3, 12], 48, samples, max_terms=2)
        self.assertEqual(result, [{"atoms": [0, 1], "ops": ["&"]}])
        self.assertEqual(CONDITIONS.condition_mask(result[0], samples.truth, samples.all), 5)

    def test_or_satisfies_whole_windows_and_preserves_native_right_association(self):
        samples = Samples([240, 204, 170], 255)
        for windows, negatives, ops, expected_mask in (
                ([32, 64], 31, ["&", "|"], 224),
                ([8, 16], 7, ["|", "&"], 248)):
            with self.subTest(ops=ops):
                result = self.guards(windows, negatives, samples, max_terms=3,
                                     max_states=384, preferred_atoms=[0, 1, 2])
                expected = {"atoms": [0, 1, 2], "ops": ops}
                self.assertIn(expected, result)
                self.assertEqual(CONDITIONS.condition_mask(expected, samples.truth, samples.all), expected_mask)
        # A|B&C is A|(B&C), whose row4 is true; (A|B)&C would lose it.
        self.assertEqual((240 | 204) & 170 & 16, 0)

    def test_overlapping_windows_can_share_a_witness_and_forbidden_rows_cannot_be_witnesses(self):
        samples = Samples([2, 4], 7)
        result = self.guards([3, 6], 4, samples, max_terms=1)
        self.assertEqual(result, [{"atoms": [0], "ops": []}])
        self.assertEqual(self.guards([4], 4, samples), [])

    def test_all_distinct_clock_aliases_survive_truth_signature_deduplication(self):
        atoms = ["rage>50"] + ["bufftime:嗜血<" + str(5 + i / 10) for i in range(4)]
        samples = Samples([1] * 5, 3, costs=[2, 14, 14, 14, 14], atoms=atoms)
        result = self.guards([1], 2, samples, max_terms=1, max_candidates=8)
        self.assertEqual({tuple(rule["atoms"]) for rule in result}, {(i,) for i in range(5)})

    def test_preferred_source_clock_leaf_survives_a_tiny_atom_quota(self):
        atoms = ["rage>50", "bufftime:嗜血<5.0", "bufftime:嗜血<5.1", "bufftime:嗜血<5.2"]
        samples = Samples([1] * 4, 3, costs=[2, 14, 14, 14], atoms=atoms)
        ordinary = self.guards([1], 2, samples, max_terms=1, max_atoms=1)
        preferred = self.guards([1], 2, samples, max_terms=1, max_atoms=1, preferred_atoms=[3])
        self.assertEqual(ordinary, [{"atoms": [0], "ops": []}])
        self.assertEqual(preferred, [{"atoms": [3], "ops": []}])

    def test_preferred_clock_can_add_a_distinct_schedule_footprint_to_a_truth_alias(self):
        samples = Samples([1, 3], 3, costs=[2, 14], atoms=["rage>50", "bufftime:嗜血<5.2"])
        result = self.guards([1], 2, samples, max_terms=2, preferred_atoms=[1])
        self.assertTrue(any(1 in rule["atoms"] for rule in result))
        self.assertTrue(any(rule["atoms"] == [0] for rule in result))

    def test_six_leaf_source_chain_remains_reachable_with_finite_search(self):
        samples = Samples([127 ^ (1 << (i + 1)) for i in range(6)], 127)
        result = self.guards([1], 126, samples, max_terms=6,
                             max_states=768, preferred_atoms=list(range(6)))
        self.assertTrue(result)
        self.assertTrue(any(len(rule["atoms"]) == 6 for rule in result))
        self.assertTrue(all(CONDITIONS.is_and(rule) for rule in result))

    def test_empty_inputs_finite_budgets_and_cache_isolation(self):
        samples = Samples([1, 2, 3], 7)
        self.assertEqual(self.guards([], 4, samples), [])
        self.assertEqual(self.guards([0], 4, samples), [])
        self.assertEqual(self.guards([1], 4, samples, max_states=0), [])
        self.assertEqual(self.guards([1], 4, samples, max_atoms=0), [])
        self.assertEqual(self.guards([1], 4, samples, max_candidates=0), [])
        first = self.guards([1, 2], 4, samples, max_states=16, max_candidates=2)
        self.assertLessEqual(len(first), 2)
        self.assertEqual(first, self.guards([2, 1], 4, samples, max_states=16, max_candidates=2))
        self.assertEqual(first, self.guards([1, 1, 2], 4, samples, max_states=16, max_candidates=2))
        first[0]["atoms"].append(999)
        second = self.guards([1, 2], 4, samples, max_states=16, max_candidates=2)
        self.assertNotIn(999, second[0]["atoms"])
        samples.truth = [4, 4, 4]
        self.assertEqual(self.guards([1, 2], 4, samples, max_states=16, max_candidates=2), [])

    def test_determinism_input_isolation_and_cancellation(self):
        samples = Samples([1 | 4 | 16, 1 | 4 | 32], 63)
        windows, negatives, preferred = [3, 12], 48, [1, 0]
        first = self.guards(windows, negatives, samples, max_terms=2, preferred_atoms=preferred)
        second = self.guards(windows, negatives, samples, max_terms=2, preferred_atoms=list(reversed(preferred)))
        self.assertEqual(first, second)
        self.assertEqual(windows, [3, 12])
        self.assertEqual(samples.truth, [21, 37])
        self.assertEqual(preferred, [1, 0])

        def stopped():
            samples.check_count += 1
            if samples.check_count > 20:
                raise RuntimeError("paused")

        samples.check_count = 0
        samples.check = stopped
        with self.assertRaisesRegex(RuntimeError, "paused"):
            CONDITIONS.window_guards(windows, negatives, samples, max_terms=2)
        self.assertEqual(windows, [3, 12])
        self.assertEqual(samples.truth, [21, 37])

    def test_cached_precomputed_features_follow_replaced_costs_and_clock_catalogue(self):
        samples = Samples([1, 1], 3, costs=[2, 7])
        self.assertEqual(self.guards([1], 2, samples, max_terms=1),
                         [{"atoms": [0], "ops": []}])
        samples.atom_costs = [9, 2]
        self.assertEqual(self.guards([1], 2, samples, max_terms=1),
                         [{"atoms": [1], "ops": []}])
        samples.atoms = ["rage>50", "bufftime:嗜血<5.2"]
        self.assertEqual(self.guards([1], 2, samples, max_terms=1),
                         [{"atoms": [1], "ops": []}, {"atoms": [0], "ops": []}])

    def test_cancellation_is_checked_during_catalogue_scan_and_on_cached_queries(self):
        samples = Samples([1] * 1024, 3,
                          atoms=["bufftime:嗜血<" + str(i) for i in range(1024)])

        def stopped():
            samples.check_count += 1
            if samples.check_count == 8:
                raise RuntimeError("paused during catalogue")

        samples.check = stopped
        with self.assertRaisesRegex(RuntimeError, "paused during catalogue"):
            CONDITIONS.window_guards([1], 2, samples, max_terms=1)
        self.assertEqual(samples.check_count, 8)
        self.assertFalse(hasattr(samples, "_native_window_cache"))

        samples.check = lambda: None
        expected = self.guards([1], 2, samples, max_terms=1)
        self.assertTrue(expected)

        def cached_stopped():
            raise RuntimeError("paused on cached query")

        samples.check = cached_stopped
        with self.assertRaisesRegex(RuntimeError, "paused on cached query"):
            CONDITIONS.window_guards([1], 2, samples, max_terms=1)


if __name__ == "__main__":
    unittest.main()
