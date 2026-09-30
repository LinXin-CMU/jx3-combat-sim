"""Native condition-chain and bounded guard-synthesis contracts."""
import importlib.util
from itertools import product
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location("exact_macro_conditions", ROOT / "tools/exact_macro_conditions.py")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class Samples:
    def __init__(self, truth, atoms=None, costs=None, all_mask=None):
        self.truth = truth
        self.atoms = atoms if atoms is not None else ["feature" + str(index) for index in range(len(truth))]
        self.atom_costs = costs if costs is not None else [len(atom) for atom in self.atoms]
        self.all = all_mask if all_mask is not None else (1 << max(1, max(truth, default=0).bit_length())) - 1
        self.checks = 0

    def check(self):
        self.checks += 1


class ConditionTests(unittest.TestCase):
    def test_legacy_and_or_suffix_normalizes_and_ops_take_precedence(self):
        old = {"atoms": [0, 1], "any_atoms": [2, 3]}
        self.assertEqual(MODULE.normalize(old), {"atoms": [0, 1, 2, 3], "ops": ["&", "&", "|"]})
        self.assertEqual(MODULE.condition_key(old), ((0, 1, 2, 3), ("&", "&", "|")))
        self.assertEqual(MODULE.normalize({"atoms": [0, 1], "ops": ["|"], "any_atoms": [2]}),
                         {"atoms": [0, 1], "ops": ["|"]})
        self.assertTrue(MODULE.is_and({"atoms": [0, 1]}))
        self.assertFalse(MODULE.is_and(old))
        self.assertEqual(old, {"atoms": [0, 1], "any_atoms": [2, 3]})

    def test_equal_precedence_right_association_differs_from_normal_precedence(self):
        rule = {"atoms": [0, 1, 2], "ops": ["&", "|"]}
        self.assertEqual(MODULE.condition_mask(rule, [0b01, 0b10, 0b10], 0b11), 0)
        normal_precedence = (0b01 & 0b10) | 0b10
        self.assertEqual(normal_precedence, 0b10)
        for a, b, c in product((False, True), repeat=3):
            self.assertEqual(bool(MODULE.condition_mask(rule, [int(a), int(b), int(c)], 1)), a and (b or c))
            other = {"atoms": [0, 1, 2], "ops": ["|", "&"]}
            self.assertEqual(bool(MODULE.condition_mask(other, [int(a), int(b), int(c)], 1)), a or (b and c))

    def test_render_cost_and_empty_guard_use_native_text_only(self):
        atoms = ["rage>65", "buff:援戈", "nobuff:嗜血"]
        rule = {"atoms": [0, 1, 2], "ops": ["|", "&"]}
        text = MODULE.condition_text(rule, atoms)
        self.assertEqual(text, "rage>65|buff:援戈&nobuff:嗜血")
        costs = [len(atom.encode("utf-16-le")) // 2 for atom in atoms]
        self.assertEqual(MODULE.condition_cost(rule, costs), len(("[" + text + "] ").encode("utf-16-le")) // 2)
        self.assertNotIn("(", text)
        self.assertEqual(MODULE.condition_text({"atoms": []}, atoms), "")
        self.assertEqual(MODULE.condition_mask({"atoms": []}, [0, 0, 0], 31), 31)
        self.assertEqual(MODULE.condition_cost({"atoms": []}, costs), 0)
        with self.assertRaises(ValueError):
            MODULE.normalize({"atoms": [0, 1, 2], "ops": ["|"]})
        with self.assertRaises(ValueError):
            MODULE.normalize({"atoms": [0, 1], "ops": ["AND"]})

    def test_synthesis_finds_a_chain_beyond_and_prefix_or_suffix(self):
        # A OR (B AND C) alone covers both positive states and neither wrong
        # state. No AND-prefix/OR-suffix arrangement of these leaves does so.
        samples = Samples([0b0001, 0b0110, 0b1010], ["A", "B", "C"], [1, 1, 1], 0b1111)
        guards = MODULE.short_guards(0b0011, 0b1100, samples, max_terms=3)
        self.assertTrue(guards)
        self.assertIn({"atoms": [0, 1, 2], "ops": ["|", "&"]}, guards)
        self.assertTrue(all(MODULE.condition_mask(guard, samples.truth, samples.all) == 0b0011 for guard in guards))
        self.assertEqual(guards, MODULE.short_guards(0b0011, 0b1100, samples, max_terms=3))
        self.assertEqual(MODULE.short_guards(0b0011, 0b1100, samples, max_terms=2), [])
        self.assertGreater(samples.checks, 5)

    def test_same_truth_keeps_distinct_clock_alternatives(self):
        samples = Samples([0b01, 0b01, 0b01],
                          ["rage>50", "bufftime:嗜血<5.0", "bufftime:嗜血<5.1"], [7, 15, 15], 0b11)
        guards = MODULE.short_guards(0b01, 0b10, samples, max_terms=1)
        self.assertEqual({tuple(guard["atoms"]) for guard in guards}, {(0,), (1,), (2,)})
        self.assertTrue(all(MODULE.condition_mask(guard, samples.truth, samples.all) == 0b01 for guard in guards))

    def test_bounded_pool_and_search_return_only_valid_candidates(self):
        samples = Samples([0b0001, 0b0110, 0b1010] + [0b0101] * 300,
                          costs=[1, 1, 1] + [30] * 300, all_mask=0b1111)
        guards = MODULE.short_guards(0b0011, 0b1100, samples, max_terms=3,
                                     max_candidates=2, branch_limit=4, max_states=16, max_atoms=4)
        self.assertLessEqual(len(guards), 2)
        self.assertTrue(guards)
        self.assertTrue(all(len(guard["atoms"]) <= 3 for guard in guards))
        self.assertTrue(all(MODULE.condition_mask(guard, samples.truth, samples.all) == 0b0011 for guard in guards))
        self.assertEqual(MODULE.short_guards(1, 1, samples), [])
        self.assertEqual(MODULE.short_guards(0, 1, samples), [])
        self.assertEqual(MODULE.short_guards(1, 2, samples, max_states=0), [])

    def test_shallow_or_root_survives_many_and_distractors(self):
        # Every distractor covers P and shares one wrong state, making AND
        # subtrees attractive without solving the original problem alone.
        all_mask = (1 << 16) - 1
        truth = [0b0001, 0b0110, 0b1010]
        truth.extend(0b0111 | (1 << index) for index in range(4, 16))
        samples = Samples(truth, costs=[1] * len(truth), all_mask=all_mask)
        guards = MODULE.short_guards(0b0011, all_mask & ~0b0011, samples,
                                     max_terms=8, branch_limit=24, max_states=80)
        self.assertIn({"atoms": [0, 1, 2], "ops": ["|", "&"]}, guards)

    def test_per_samples_cache_isolated_results_and_new_catalog_invalidation(self):
        samples = Samples([1, 1, 1], ["rage>50", "bufftime:嗜血<5.0", "bufftime:嗜血<5.1"], [7, 15, 15], 3)
        first = MODULE.short_guards(1, 2, samples, max_terms=1)
        checks = samples.checks
        first[0]["atoms"].append(99)
        second = MODULE.short_guards(1, 2, samples, max_terms=1)
        self.assertLessEqual(samples.checks - checks, 2)
        self.assertTrue(all(99 not in guard["atoms"] for guard in second))
        self.assertTrue(any(guard["atoms"] == [1] for guard in second))
        samples.truth = [1, 2, 1]
        third = MODULE.short_guards(1, 2, samples, max_terms=1)
        self.assertFalse(any(guard["atoms"] == [1] for guard in third))


if __name__ == "__main__":
    unittest.main()
