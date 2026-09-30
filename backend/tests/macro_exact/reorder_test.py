"""Contracts for atomic global-position candidate generation (no oracle)."""
import importlib.util
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location("exact_macro_reorder", ROOT / "tools/exact_macro_reorder.py")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def clone(rules):
    result = []
    for rule in rules:
        copied = dict(rule, atoms=list(rule["atoms"]))
        if "any_atoms" in rule:
            copied["any_atoms"] = list(rule["any_atoms"])
        result.append(copied)
    return result


class Samples:
    """Four decision states with exact first-match and WAIT constraints."""
    all = 0b1111
    executable = [all, all, all, 0]
    allowed = [0b0011, 0b0100, 0b1000, 0]
    # Two long original guards, a shorter joint guard, and B's guard.
    truth = [0b0001, 0b0010, 0b0111, 0b0100, 0]
    atom_costs = [70, 65, 7, 5, 4]

    def __init__(self):
        self.check_count = 0

    def check(self):
        self.check_count += 1

    def hit(self, rule):
        bits = self.executable[rule["action"]]
        for atom in rule["atoms"]:
            bits &= self.truth[atom]
        if rule.get("any_atoms"):
            union = 0
            for atom in rule["any_atoms"]:
                union |= self.truth[atom]
            bits &= union
        return bits

    def rule_cost(self, rule):
        terms = rule["atoms"] + rule.get("any_atoms", [])
        return 7 + (2 + sum(self.atom_costs[atom] + 1 for atom in terms) if terms else 0)

    def compatible(self, rules):
        remaining = self.all
        for rule in rules:
            selected = remaining & self.hit(rule)
            if selected & ~self.allowed[rule["action"]]:
                return False
            remaining &= ~selected
        return remaining == 0

    def covers(self, positives, negatives):
        if not positives:
            return []
        if not negatives:
            return [[]]
        return [[atom] for atom, truth in enumerate(self.truth)
                if not positives & ~truth and not negatives & truth]


def scene(or_blocker=False):
    blocker = {"action": 1, "atoms": [3]}
    if or_blocker:
        blocker = {"action": 1, "atoms": [], "any_atoms": [3, 4]}
    return [
        {"action": 0, "atoms": [0]},
        {"action": 3, "atoms": []},
        {"action": 3, "atoms": []},
        blocker,
        {"action": 3, "atoms": []},
        {"action": 3, "atoms": []},
        {"action": 0, "atoms": [1]},
        {"action": 2, "atoms": []},
    ]


class PriorityEditsTests(unittest.TestCase):
    def test_nonadjacent_joint_move_replace_delete_crosses_three_positions(self):
        samples, rules = Samples(), scene()
        self.assertTrue(samples.compatible(rules))
        survivors = clone(rules[1:6] + rules[7:])
        # Moving A's old guard and deleting its donor loses the second A state.
        move_only = survivors[:3] + clone([rules[0]]) + survivors[3:]
        self.assertFalse(samples.compatible(move_only))
        # Replacing A in place and deleting its donor steals B's decision.
        replace_only = [{"action": 0, "atoms": [2]}] + survivors
        self.assertFalse(samples.compatible(replace_only))
        expected = survivors[:3] + [{"action": 0, "atoms": [2]}] + survivors[3:]
        self.assertTrue(samples.compatible(expected))
        proposals = list(MODULE.priority_edits(rules, samples, clone))
        self.assertIn(("priority_rebuild_pair", expected), proposals)
        self.assertGreater(samples.check_count, 10)
        self.assertEqual(rules, scene(), "generating edits must not mutate the certified input")

    def test_first_match_ineligible_actions_and_or_guards_are_preserved(self):
        samples, rules = Samples(), scene(or_blocker=True)
        survivors = clone(rules[1:6] + rules[7:])
        expected = survivors[:3] + [{"action": 0, "atoms": [2]}] + survivors[3:]
        proposals = list(MODULE.priority_edits(rules, samples, clone))
        self.assertIn(("priority_rebuild_pair", expected), proposals)
        self.assertTrue(all(samples.compatible(trial) for _, trial in proposals))
        self.assertEqual(expected[2]["any_atoms"], [3, 4])
        # The first two unconditional actions are ineligible; B still protects
        # its state before the widened A guard can be selected.
        self.assertEqual(samples.hit(expected[0]), 0)
        self.assertEqual(samples.hit(expected[1]), 0)
        wrong_priority = [expected[3]] + expected[:3] + expected[4:]
        self.assertFalse(samples.compatible(wrong_priority))

    def test_cost_unique_limit_and_deterministic_order(self):
        samples, rules = Samples(), scene()
        baseline = sum(samples.rule_cost(rule) + 1 for rule in rules)
        proposals = list(MODULE.priority_edits(rules, samples, clone, limit=4))
        self.assertEqual(proposals, list(MODULE.priority_edits(rules, samples, clone, limit=4)))
        self.assertLessEqual(len(proposals), 4)
        self.assertTrue(proposals)
        costs = [sum(samples.rule_cost(rule) + 1 for rule in trial) for _, trial in proposals]
        self.assertEqual(costs, sorted(costs))
        self.assertTrue(all(cost < baseline for cost in costs))
        identities = [tuple((rule["action"], tuple(rule["atoms"]), tuple(rule.get("any_atoms", [])))
                            for rule in trial) for _, trial in proposals]
        self.assertEqual(len(identities), len(set(identities)))
        self.assertEqual(list(MODULE.priority_edits(rules, samples, clone, limit=0)), [])

    def test_wait_state_cannot_be_stolen_by_a_shorter_guard(self):
        samples, rules = Samples(), scene()
        # The last state must WAIT, so no action is acceptable there.
        samples.allowed = [0b0011, 0b0100, 0, 0]
        samples.compatible = lambda trial: self._compatible_with_wait(samples, trial)
        rules[-1] = {"action": 2, "atoms": [4]}
        self.assertTrue(samples.compatible(rules))
        proposals = list(MODULE.priority_edits(rules, samples, clone))
        self.assertTrue(proposals)
        self.assertTrue(all(samples.compatible(trial) for _, trial in proposals))
        self.assertFalse(samples.compatible([{"action": 0, "atoms": []}] + rules[1:]))

    @staticmethod
    def _compatible_with_wait(samples, rules):
        remaining = samples.all
        for rule in rules:
            selected = remaining & samples.hit(rule)
            if selected & ~samples.allowed[rule["action"]]:
                return False
            remaining &= ~selected
        return remaining == 0b1000


if __name__ == "__main__":
    unittest.main()
