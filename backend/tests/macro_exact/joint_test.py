"""Cross-action rebuild contracts; sample guidance is never a replay certificate."""
import importlib.util
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[3]


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "tools" / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


MODULE = load("exact_macro_joint")
CONDITIONS = load("exact_macro_conditions")


def clone(rules):
    copied = []
    for rule in rules:
        result = dict(rule, atoms=list(rule["atoms"]))
        for field in ("ops", "any_atoms"):
            if field in rule:
                result[field] = list(rule[field])
        copied.append(result)
    return copied


def signature(rules):
    return tuple((rule["action"], CONDITIONS.condition_key(rule)) for rule in rules)


class Samples:
    def __init__(self, truth, allowed, executable=None, costs=None, atoms=None):
        self.all = (1 << max(mask.bit_length() for mask in truth + allowed)) - 1
        self.truth, self.allowed = list(truth), list(allowed)
        self.executable = list(executable) if executable is not None else [self.all] * len(allowed)
        self.atom_costs = list(costs) if costs is not None else [40] * len(truth)
        self.atoms = list(atoms) if atoms is not None else ["feature" + str(i) for i in range(len(truth))]
        self.action_costs = [7] * len(allowed)
        self.actions = [{"name": "action" + str(i)} for i in range(len(allowed))]
        self.required = 0
        for mask in allowed:
            self.required |= mask
        self.groups = {index: 1 << index for index in range(self.all.bit_length()) if self.required & (1 << index)}
        self.rows = []
        self.check_count = 0

    def check(self):
        self.check_count += 1

    def hit(self, rule):
        return self.executable[rule["action"]] & CONDITIONS.condition_mask(rule, self.truth, self.all)

    def rule_cost(self, rule):
        return self.action_costs[rule["action"]] + CONDITIONS.condition_cost(rule, self.atom_costs)

    def compatible(self, rules):
        remaining = self.all
        for rule in rules:
            hit = remaining & self.hit(rule)
            if hit & ~self.allowed[rule["action"]]:
                return False
            remaining &= ~hit
        return not remaining & self.required and all(group & ~remaining for group in self.groups.values())

    def covers(self, positive, negative):
        if not positive:
            return []
        if not negative:
            return [[]]
        return [[index] for index, truth in enumerate(self.truth)
                if not positive & ~truth and not negative & truth]


def singles(positive, negative, samples):
    return CONDITIONS.short_guards(positive, negative, samples,
                                   max_terms=1, max_candidates=6, max_states=16)


class JointEditsTests(unittest.TestCase):
    def proposals(self, rules, samples, **kwargs):
        reports = []
        kwargs.setdefault("witness_fallback", False)
        proposals = list(MODULE.joint_edits(rules, samples, clone,
                                            diagnostic=lambda info, _: reports.append(info), **kwargs))
        bound = sum(samples.rule_cost(rule) + 1 for rule in rules)
        self.assertTrue(reports)
        self.assertTrue(all(samples.compatible(trial) for _, trial in proposals))
        self.assertTrue(all(sum(samples.rule_cost(rule) + 1 for rule in trial) < bound for _, trial in proposals))
        self.assertEqual(len(proposals), len({signature(trial) for _, trial in proposals}))
        self.assertTrue(all(kind.startswith("joint_") for kind, _ in proposals))
        return proposals, reports[-1]

    def assertProposal(self, expected, proposals):
        self.assertIn(signature(expected), [signature(trial) for _, trial in proposals])

    def test_two_action_guards_and_order_are_rebuilt_together(self):
        samples = Samples([1, 2, 3, 2, 4], [1, 2], costs=[60, 50, 2, 2, 2])
        rules = [{"action": 0, "atoms": [0]}, {"action": 1, "atoms": [1]}]
        expected = [{"action": 1, "atoms": [3]}, {"action": 0, "atoms": [2]}]
        self.assertTrue(samples.compatible(rules))
        self.assertFalse(samples.compatible([expected[1], expected[0]]))
        proposals, report = self.proposals(rules, samples, condition_search=singles)
        self.assertProposal(expected, proposals)
        self.assertEqual(report["action_sets"][0]["actions"], [0, 1])
        self.assertGreater(report["conflict_action_pairs"], 0)

    def test_native_or_guard_competes_with_another_action(self):
        samples = Samples([1, 2, 4, 8, 3 | 8, 4, 8, 16], [7, 8],
                          costs=[50, 50, 50, 50, 2, 2, 2, 2])
        rules = [{"action": 0, "atoms": [i]} for i in range(3)] + [{"action": 1, "atoms": [3]}]
        expected = [{"action": 1, "atoms": [6]},
                    {"action": 0, "atoms": [4, 5], "ops": ["|"]}]

        def native(positive, negative, sample):
            return CONDITIONS.short_guards(positive, negative, sample,
                                           max_terms=2, max_candidates=6, max_states=48)

        proposals, _ = self.proposals(rules, samples, condition_search=native,
                                      max_regions=12, max_expansions=256)
        self.assertProposal(expected, proposals)

    def test_existing_or_is_refined_by_a_native_and_prefix(self):
        samples = Samples([1 | 4, 2, 4, 3 | 8, 1 | 4], [3, 4],
                          costs=[50, 50, 70, 2, 2])
        source_or = {"action": 0, "atoms": [0, 1], "ops": ["|"]}
        rules = [{"action": 1, "atoms": [2]}, source_or]
        expected = [{"action": 0, "atoms": [3, 0, 1], "ops": ["&", "|"]},
                    {"action": 1, "atoms": [4]}]
        self.assertTrue(samples.compatible(rules))
        self.assertGreater(samples.rule_cost(expected[0]), samples.rule_cost(source_or))
        proposals, report = self.proposals(rules, samples, condition_search=singles,
                                          max_replacements=2, max_regions=4,
                                          max_expansions=64, limit=64)
        self.assertProposal(expected, proposals)
        self.assertGreater(report["refined_guards"], 0)
        self.assertEqual(CONDITIONS.condition_mask(expected[0], samples.truth, samples.all), 3)

    def test_replacement_can_increase_row_count_and_cross_survivor_gaps(self):
        samples = Samples([3, 4, 1, 2 | 4, 1 | 4, 8, 16], [3, 4, 8],
                          costs=[70, 70, 2, 2, 2, 3, 3])
        survivor = {"action": 2, "atoms": [5]}
        rules = [{"action": 0, "atoms": [0]}, survivor, {"action": 1, "atoms": [1]}]
        expected = [{"action": 0, "atoms": [2]}, {"action": 1, "atoms": [4]},
                    {"action": 0, "atoms": [3]}, survivor]
        self.assertFalse(samples.compatible(expected[:2] + expected[3:]))
        proposals, _ = self.proposals(rules, samples, condition_search=singles,
                                      max_replacements=3, max_expansions=256, limit=128)
        self.assertProposal(expected, proposals)
        self.assertTrue(any(len(trial) > len(rules) for _, trial in proposals))

    def test_executable_masks_and_wait_samples_keep_their_meanings(self):
        samples = Samples([1, 3, 4], [1, 2], executable=[1, 3], costs=[50, 50, 2])
        rules = [{"action": 0, "atoms": [0]}, {"action": 1, "atoms": [1]}]
        expected = [{"action": 0, "atoms": []}, {"action": 1, "atoms": []}]
        proposals, _ = self.proposals(rules, samples, condition_search=singles)
        self.assertProposal(expected, proposals)
        samples.executable = [7, 7]
        protected, _ = self.proposals(rules, samples, condition_search=singles)
        self.assertTrue(all(not any(not rule["atoms"] for rule in trial) for _, trial in protected))

    def test_clock_variants_are_not_truth_aliases(self):
        samples = Samples([1, 2, 3, 3, 3, 2, 4], [1, 2],
                          costs=[60, 60, 2, 14, 15, 2, 2],
                          atoms=["old_a", "old_b", "rage>50", "bufftime:嗜血<5.0",
                                 "bufftime:嗜血<5.1", "b_short", "wait"])
        rules = [{"action": 0, "atoms": [0]}, {"action": 1, "atoms": [1]}]
        proposals, report = self.proposals(rules, samples, condition_search=singles,
                                          max_replacements=2, max_regions=4, max_expansions=64)
        for atom in (2, 3, 4):
            self.assertProposal([{"action": 1, "atoms": [5]}, {"action": 0, "atoms": [atom]}], proposals)
        self.assertGreaterEqual(report["action_sets"][0]["clock_variants_selected"], 3)
        self.assertGreater(report["primitive_pools"][0]["stable_selected"], 1)

    def test_many_clock_aliases_do_not_starve_stable_refinement_features(self):
        truth = [3, 4, 5, 6, 1, 2, 8] + [7] * 20
        atoms = ["old_a", "old_b", "mechanism_gate", "resource_gate", "state_a", "state_b", "wait"]
        atoms += ["bufftime:嗜血<" + str(5 + index / 10) for index in range(20)]
        samples = Samples(truth, [3, 4], costs=[60, 60, 2, 2, 2, 2, 2] + [20] * 20, atoms=atoms)
        rules = [{"action": 0, "atoms": [0]}, {"action": 1, "atoms": [1]}]
        _, report = self.proposals(rules, samples, condition_search=singles,
                                   max_expansions=1, max_feature_guards=6)
        for pool in report["primitive_pools"]:
            self.assertEqual(pool["stable_quota"], 3)
            self.assertEqual(pool["clock_quota"], 3)
            self.assertEqual(pool["stable_selected"], 3)
            self.assertEqual(pool["clock_selected"], 3)

    def test_incompatible_seed_targets_actual_allowed_actions(self):
        samples = Samples([3, 2, 1, 4], [1, 2], costs=[40, 40, 2, 2])
        rules = [{"action": 0, "atoms": [0]}, {"action": 1, "atoms": [1]}]
        self.assertFalse(samples.compatible(rules))
        proposals, _ = self.proposals(rules, samples, condition_search=singles,
                                      max_replacements=2, max_regions=4, max_expansions=64)
        self.assertTrue(proposals)
        self.assertProposal([{"action": 0, "atoms": [2]}, {"action": 1, "atoms": [1]}], proposals)

    def test_external_incumbent_ceiling_allows_a_larger_repaired_seed(self):
        samples = Samples([3, 2, 1, 4], [1, 2], costs=[2, 2, 20, 2])
        rules = [{"action": 0, "atoms": [0]}, {"action": 1, "atoms": [1]}]
        seed_cost = sum(samples.rule_cost(rule) + 1 for rule in rules)
        expected = [{"action": 0, "atoms": [2]}, {"action": 1, "atoms": [1]}]
        self.assertGreater(sum(samples.rule_cost(rule) + 1 for rule in expected), seed_cost)
        self.assertEqual(list(MODULE.joint_edits(rules, samples, clone, condition_search=singles,
                                               witness_fallback=False)), [])
        reports = []
        proposals = list(MODULE.joint_edits(rules, samples, clone, condition_search=singles,
                                           witness_fallback=False, cost_bound=80,
                                           diagnostic=lambda info, _: reports.append(info)))
        self.assertProposal(expected, proposals)
        self.assertTrue(all(samples.compatible(trial) for _, trial in proposals))
        self.assertTrue(all(sum(samples.rule_cost(rule) + 1 for rule in trial) < 80 for _, trial in proposals))
        self.assertEqual(reports[-1]["seed_chars"], seed_cost - 1)
        self.assertEqual(reports[-1]["ceiling_chars"], 79)

    def test_wait_allowed_success_window_can_shift_its_witness(self):
        samples = Samples([1, 2, 4, 8], [3, 4], costs=[50, 2, 50, 2])
        samples.required = 4
        samples.groups = {0: 3, 1: 4}
        rules = [{"action": 0, "atoms": [0]}, {"action": 1, "atoms": [2]}]
        expected = [{"action": 0, "atoms": [1]}, {"action": 1, "atoms": [2]}]
        self.assertTrue(samples.compatible(rules))
        self.assertTrue(samples.compatible(expected))
        proposals, _ = self.proposals(rules, samples, condition_search=singles)
        self.assertProposal(expected, proposals)

    def test_repair_priority_prefers_small_structural_changes_to_future_rule_cuts(self):
        samples = Samples([3, 2, 1, 0, 4], [1, 2, 0], costs=[2, 2, 20, 50, 2])
        future = {"action": 2, "atoms": [3]}
        rules = [{"action": 0, "atoms": [0]}, future, {"action": 1, "atoms": [1]}]
        reports = []
        proposals = list(MODULE.joint_edits(rules, samples, clone, condition_search=singles,
                                           witness_fallback=False, cost_bound=160,
                                           max_regions=2, max_expansions=96, limit=128,
                                           diagnostic=lambda info, _: reports.append(info)))
        self.assertTrue(proposals)
        first = proposals[0][1]
        self.assertIn(signature([future])[0], signature(first))
        cuts = [trial for _, trial in proposals if not any(rule["action"] == 2 for rule in trial)]
        self.assertTrue(cuts)
        first_cost = sum(samples.rule_cost(rule) + 1 for rule in first)
        self.assertLess(min(sum(samples.rule_cost(rule) + 1 for rule in trial) for trial in cuts), first_cost)
        self.assertTrue(all(samples.compatible(trial) for _, trial in proposals))
        report = reports[-1]
        self.assertTrue(report["repair_priority"])
        self.assertEqual(report["selected_edit_distances"], sorted(report["selected_edit_distances"]))
        self.assertEqual(report["selected_edit_distances"][0], 2)
        self.assertEqual(report["regions_enumerated"], 2)

    def test_success_projection_is_separate_and_keeps_one_branch_prefix(self):
        samples = Samples([1, 2, 4], [1, 2], costs=[3, 3, 3])
        rules = [{"action": 0, "atoms": [0]}, {"action": 1, "atoms": [1]}]
        reports = []
        strict = list(MODULE.joint_edits(rules, samples, clone, condition_search=singles,
                                        witness_fallback=False, max_replacements=2))
        self.assertEqual(strict, [])
        proposals = list(MODULE.joint_edits(rules, samples, clone, condition_search=singles,
                                           diagnostic=lambda info, _: reports.append(info),
                                           max_replacements=2))
        self.assertTrue(proposals)
        self.assertTrue(all(kind.startswith("joint_witness_") for kind, _ in proposals))
        self.assertEqual([report["guide_kind"] for report in reports],
                         ["full_source_trajectory", "success_witness_projection"])
        self.assertFalse(any(samples.compatible(trial) for _, trial in proposals))
        samples.global_keep_mask = 4
        self.assertEqual(list(MODULE.joint_edits(rules, samples, clone, condition_search=singles)), [])

    def test_budgets_determinism_input_isolation_and_cancellation(self):
        samples = Samples([1, 2, 3, 2, 4], [1, 2], costs=[60, 50, 2, 2, 2])
        rules = [{"action": 0, "atoms": [0]}, {"action": 1, "atoms": [1]}]
        saved = clone(rules)
        calls = []

        def search(positive, negative, sample):
            calls.append((positive, negative))
            return singles(positive, negative, sample)

        first, report = self.proposals(rules, samples, condition_search=search,
                                       max_expansions=8, max_guard_queries=2, max_regions=2, limit=3)
        self.assertLessEqual(report["expanded_states"], 8)
        self.assertLessEqual(len(calls), 2)
        self.assertEqual(len(calls), len(set(calls)))
        self.assertLessEqual(report["regions_enumerated"], 2)
        self.assertLessEqual(len(first), 3)
        second, _ = self.proposals(rules, samples, condition_search=singles,
                                   max_expansions=8, max_guard_queries=2, max_regions=2, limit=3)
        self.assertEqual(first, second)
        self.assertEqual(rules, saved)
        self.assertEqual(list(MODULE.joint_edits(rules, samples, clone, limit=0)), [])

        def stopped():
            samples.check_count += 1
            if samples.check_count > 20:
                raise RuntimeError("paused")

        samples.check_count = 0
        samples.check = stopped
        with self.assertRaisesRegex(RuntimeError, "paused"):
            list(MODULE.joint_edits(rules, samples, clone))
        self.assertEqual(rules, saved)


if __name__ == "__main__":
    unittest.main()
