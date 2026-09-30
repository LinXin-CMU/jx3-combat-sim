"""Family rebuild search contracts; masks are not a native replay certificate."""
import importlib.util
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[3]


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "tools" / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


MODULE = load("exact_macro_family")
CONDITIONS = load("exact_macro_conditions")


def clone(rules):
    result = []
    for rule in rules:
        copied = dict(rule, atoms=list(rule["atoms"]))
        for field in ("any_atoms", "ops"):
            if field in rule:
                copied[field] = list(rule[field])
        result.append(copied)
    return result


class Samples:
    def __init__(self, truth, allowed, executable=None, costs=None, atoms=None):
        self.truth = list(truth)
        self.all = (1 << max(mask.bit_length() for mask in truth + allowed)) - 1
        self.allowed = list(allowed)
        self.executable = list(executable) if executable is not None else [self.all] * len(allowed)
        self.atom_costs = list(costs) if costs is not None else [40] * len(truth)
        self.atoms = list(atoms) if atoms is not None else ["feature" + str(i) for i in range(len(truth))]
        self.required = 0
        for bits in allowed:
            self.required |= bits
        self.check_count = 0

    def check(self):
        self.check_count += 1

    def hit(self, rule):
        return self.executable[rule["action"]] & CONDITIONS.condition_mask(rule, self.truth, self.all)

    def rule_cost(self, rule):
        return 7 + CONDITIONS.condition_cost(rule, self.atom_costs)

    def compatible(self, rules):
        remaining = self.all
        for rule in rules:
            chosen = remaining & self.hit(rule)
            if chosen & ~self.allowed[rule["action"]]:
                return False
            remaining &= ~chosen
        return not remaining & self.required

    def covers(self, positives, negatives):
        if not positives:
            return []
        if not negatives:
            return [[]]
        return [[index] for index, mask in enumerate(self.truth)
                if not positives & ~mask and not negatives & mask]


def singles(positives, negatives, samples):
    return CONDITIONS.short_guards(positives, negatives, samples,
                                   max_terms=1, max_candidates=6, max_states=16)


def two_rule_scene():
    # A's early rule must protect state 0 from B. The late short A guard also
    # matches B's correct state, so it must remain after B. Neither new A line
    # alone produces a compatible complete program.
    samples = Samples([1, 2, 4, 1 | 8, 16, 1, 2 | 4 | 8],
                      [1 | 2 | 4, 8, 16], costs=[65, 60, 55, 5, 5, 2, 3])
    rules = [{"action": 0, "atoms": [0]}, {"action": 1, "atoms": [3]},
             {"action": 0, "atoms": [1]}, {"action": 0, "atoms": [2]},
             {"action": 2, "atoms": [4]}]
    expected = [{"action": 0, "atoms": [5]}, {"action": 1, "atoms": [3]},
                {"action": 0, "atoms": [6]}, {"action": 2, "atoms": [4]}]
    return samples, rules, expected


class FamilyEditsTests(unittest.TestCase):
    def proposals(self, rules, samples, **kwargs):
        reports = []
        proposals = list(MODULE.family_edits(rules, samples, clone,
                                            diagnostic=lambda info, _: reports.append(info), **kwargs))
        self.assertTrue(reports)
        self.assertTrue(all(samples.compatible(trial) for _, trial in proposals))
        baseline = sum(samples.rule_cost(rule) + 1 for rule in rules)
        self.assertTrue(all(sum(samples.rule_cost(rule) + 1 for rule in trial) < baseline
                            for _, trial in proposals))
        keys = [tuple((rule["action"], tuple(rule["atoms"]), tuple(rule.get("ops", ())),
                       tuple(rule.get("any_atoms", ()))) for rule in trial) for _, trial in proposals]
        self.assertEqual(len(keys), len(set(keys)))
        return proposals, reports[-1]

    def test_nonwhole_nonconsecutive_three_donor_subset_can_relocate(self):
        samples = Samples([1, 2, 4, 8, 2 | 16, 32, 1 | 4 | 8 | 16],
                          [1 | 2 | 4 | 8, 16, 32], costs=[70, 25, 65, 60, 5, 5, 3])
        rules = [{"action": 0, "atoms": [0]}, {"action": 0, "atoms": [1]},
                 {"action": 1, "atoms": [4]}, {"action": 0, "atoms": [2]},
                 {"action": 0, "atoms": [3]}, {"action": 2, "atoms": [5]}]
        expected = [{"action": 0, "atoms": [1]}, {"action": 1, "atoms": [4]},
                    {"action": 0, "atoms": [6]}, {"action": 2, "atoms": [5]}]
        self.assertTrue(samples.compatible(rules))
        proposals, report = self.proposals(rules, samples)
        self.assertIn(("family_rebuild_3_to_1", expected), proposals)
        self.assertEqual(report["actions"][0]["subsets"], 5)

    def test_three_to_two_joint_guards_at_different_priorities(self):
        samples, rules, expected = two_rule_scene()
        self.assertTrue(samples.compatible(rules))
        self.assertFalse(samples.compatible(expected[:2] + expected[3:]))
        self.assertFalse(samples.compatible(expected[1:]))
        proposals, report = self.proposals(rules, samples, condition_search=singles)
        self.assertIn(("family_rebuild_3_to_2", expected), proposals)
        self.assertGreater(report["guard_cache_hits"], 0)
        self.assertGreater(samples.check_count, 20)

    def test_six_to_three_joint_priority_rebuild(self):
        samples = Samples([1, 2, 4, 8, 16, 32, 1 | 2 | 64, 4 | 8 | 128,
                           256, 1 | 2, 4 | 8 | 64, 16 | 32 | 128],
                          [63, 64, 128, 256], costs=[60] * 6 + [5, 5, 5, 2, 2, 2])
        rules = [{"action": 0, "atoms": [0]}, {"action": 0, "atoms": [1]},
                 {"action": 1, "atoms": [6]}, {"action": 0, "atoms": [2]},
                 {"action": 0, "atoms": [3]}, {"action": 2, "atoms": [7]},
                 {"action": 0, "atoms": [4]}, {"action": 0, "atoms": [5]},
                 {"action": 3, "atoms": [8]}]
        expected = [{"action": 0, "atoms": [9]}, {"action": 1, "atoms": [6]},
                    {"action": 0, "atoms": [10]}, {"action": 2, "atoms": [7]},
                    {"action": 0, "atoms": [11]}, {"action": 3, "atoms": [8]}]
        self.assertTrue(samples.compatible(rules))
        proposals, _ = self.proposals(rules, samples, condition_search=singles)
        self.assertIn(("family_rebuild_6_to_3", expected), proposals)

    def test_action_quotas_do_not_allow_cheapest_family_to_dominate(self):
        truth = [1 << i for i in range(8)] + [15, 240]
        samples = Samples(truth, [15, 240], costs=[100] * 4 + [20] * 4 + [2, 2])
        rules = [{"action": int(i >= 4), "atoms": [i]} for i in range(8)]
        proposals, report = self.proposals(rules, samples, limit=2)
        changed = set()
        for _, trial in proposals:
            for action in (0, 1):
                if sum(rule["action"] == action for rule in trial) < 4:
                    changed.add(action)
        self.assertEqual(changed, {0, 1})
        self.assertEqual([action["selected"] for action in report["actions"]], [1, 1])

    def test_ineligible_actions_are_not_guard_negative_samples(self):
        samples = Samples([1, 2, 4, 8], [7, 8, 0], executable=[7, 15, 0], costs=[50, 50, 50, 5])
        rules = [{"action": 2, "atoms": []}] * 3 + [
            {"action": 0, "atoms": [0]}, {"action": 0, "atoms": [1]},
            {"action": 0, "atoms": [2]}, {"action": 1, "atoms": [3]}]
        self.assertTrue(samples.compatible(rules))
        proposals, _ = self.proposals(rules, samples)
        self.assertTrue(any(any(rule["action"] == 0 and not rule["atoms"] for rule in trial)
                            for _, trial in proposals))
        self.assertTrue(all(sum(rule["action"] == 2 for rule in trial) == 3 for _, trial in proposals))

    def test_wait_and_native_ops_are_preserved(self):
        samples = Samples([1, 2, 4, 7, 8, 8], [7, 0], costs=[50, 50, 50, 2, 5, 5])
        # The survivor's right-associated native expression is false in every
        # state. Its structure must survive cloning, and state 3 must WAIT.
        survivor = {"action": 1, "atoms": [4, 5], "ops": ["&"]}
        rules = [{"action": 0, "atoms": [0]}, survivor,
                 {"action": 0, "atoms": [1]}, {"action": 0, "atoms": [2]}]
        # Action 1 is ineligible; its condition does not create a first match.
        samples.executable[1] = 0
        saved = clone(rules)
        proposals, _ = self.proposals(rules, samples, condition_search=singles)
        self.assertTrue(proposals)
        self.assertTrue(all(survivor in trial for _, trial in proposals))
        self.assertTrue(all(not any(rule["action"] == 0 and not rule["atoms"] for rule in trial)
                            for _, trial in proposals))
        self.assertEqual(rules, saved)

    def test_native_or_guard_is_used_without_parenthesis_rewriting(self):
        samples = Samples([1, 2, 4, 3, 4, 8], [7], costs=[50, 50, 50, 2, 2, 2])
        rules = [{"action": 0, "atoms": [i]} for i in range(3)]

        def native(positives, negatives, sample):
            return CONDITIONS.short_guards(positives, negatives, sample, max_terms=2,
                                           max_candidates=4, max_states=48)

        proposals, _ = self.proposals(rules, samples, condition_search=native)
        expected = [{"action": 0, "atoms": [3, 4], "ops": ["|"]}]
        self.assertIn(("family_rebuild_3_to_1", expected), proposals)

    def test_distinct_clock_guards_survive_truth_and_cost_pruning(self):
        samples = Samples([1, 2, 4, 7, 7, 7, 8], [7],
                          costs=[60, 60, 60, 2, 14, 15, 3],
                          atoms=["old0", "old1", "old2", "rage>50",
                                 "bufftime:嗜血<5.0", "bufftime:嗜血<5.1", "wrong"])
        rules = [{"action": 0, "atoms": [i]} for i in range(3)]
        proposals, _ = self.proposals(rules, samples, condition_search=singles)
        short_guards = {tuple(trial[0]["atoms"]) for kind, trial in proposals
                        if kind == "family_rebuild_3_to_1"}
        self.assertTrue({(3,), (4,), (5,)} <= short_guards)

    def test_query_expansion_subset_and_output_limits_are_static_bounds(self):
        calls = []
        samples, rules, _ = two_rule_scene()

        def search(positives, negatives, sample):
            calls.append((positives, negatives))
            return singles(positives, negatives, sample)

        proposals, report = self.proposals(rules, samples, condition_search=search,
                                           max_guard_queries=2, max_expansions=8, limit=3)
        self.assertLessEqual(len(calls), 2)
        self.assertEqual(len(calls), len(set(calls)))
        self.assertLessEqual(report["expanded_states"], 8)
        self.assertLessEqual(len(proposals), 3)
        self.assertGreater(report["guard_budget_skips"], 0)
        truth = [1 << i for i in range(9)]
        large = Samples(truth, [511])
        large_rules = [{"action": 0, "atoms": [i]} for i in range(9)]
        _, bounded = self.proposals(large_rules, large, max_subsets=11,
                                    max_expansions=1, max_guard_queries=0)
        self.assertEqual(bounded["subsets_enumerated"], 11)
        small = Samples(truth[:7], [127])
        small_rules = [{"action": 0, "atoms": [i]} for i in range(7)]
        _, exhaustive = self.proposals(small_rules, small, max_expansions=1, max_guard_queries=0)
        self.assertEqual(exhaustive["subsets_enumerated"], 99)

    def test_determinism_input_isolation_and_cancel_checks(self):
        samples, rules, _ = two_rule_scene()
        saved = clone(rules)
        first, _ = self.proposals(rules, samples, condition_search=singles)
        second, _ = self.proposals(rules, samples, condition_search=singles)
        self.assertEqual(first, second)
        self.assertEqual(rules, saved)
        self.assertEqual(list(MODULE.family_edits(rules, samples, clone, limit=0)), [])

        def stopped():
            samples.check_count += 1
            if samples.check_count > 20:
                raise RuntimeError("paused")

        samples.check_count = 0
        samples.check = stopped
        with self.assertRaisesRegex(RuntimeError, "paused"):
            list(MODULE.family_edits(rules, samples, clone, condition_search=singles))
        self.assertEqual(rules, saved)


if __name__ == "__main__":
    unittest.main()
