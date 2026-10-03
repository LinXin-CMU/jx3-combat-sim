"""Necessary local-region contracts; SAT proposals need native certification."""
import importlib.util
from pathlib import Path
import unittest

import z3


ROOT = Path(__file__).resolve().parents[3]


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "tools" / (name + ".py"))
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


REGION = load("exact_macro_region")
SAT = load("exact_macro_region_sat")
CONDITIONS = load("exact_macro_conditions")


def clone(rules):
    copied = []
    for rule in rules:
        value = dict(rule, atoms=list(rule["atoms"]))
        for field in ("ops", "any_atoms"):
            if field in rule:
                value[field] = list(rule[field])
        copied.append(value)
    return copied


class Samples:
    def __init__(self, truth, allowed, *, executable=None, atoms=None,
                 groups=None, cursors=None, required=None):
        self.all = (1 << max(mask.bit_length() for mask in truth + allowed)) - 1
        self.truth, self.allowed = list(truth), list(allowed)
        self.executable = list(executable) if executable is not None else [self.all] * len(allowed)
        self.atoms = list(atoms) if atoms is not None else ["buff:x" + str(i) for i in range(len(truth))]
        self.atom_costs = [len(text.encode("utf-16-le")) // 2 for text in self.atoms]
        self.action_costs = [8] * len(allowed)
        self.actions = [dict(name="A" + str(i), fcast=False) for i in range(len(allowed))]
        union = 0
        for mask in allowed:
            union |= mask
        self.required = union if required is None else required
        self.groups = ({index: 1 << index for index in range(self.all.bit_length())
                        if union & (1 << index)} if groups is None else groups)
        self.rows = [{"cursor": cursors[index] if cursors is not None else index}
                     for index in range(self.all.bit_length())]
        self.check_count = 0

    def check(self):
        self.check_count += 1

    def hit(self, rule):
        return self.executable[rule["action"]] & CONDITIONS.condition_mask(rule, self.truth, self.all)

    def selections(self, rules):
        remaining, selected = self.all, []
        for rule in rules:
            hit = remaining & self.hit(rule)
            selected.append(hit)
            remaining &= ~hit
        return selected, remaining

    def rule_cost(self, rule):
        return self.action_costs[rule["action"]] + CONDITIONS.condition_cost(rule, self.atom_costs)

    def compatible(self, rules):
        selected, remaining = self.selections(rules)
        return (not any(mask & ~self.allowed[rule["action"]] for rule, mask in zip(rules, selected))
                and not remaining & self.required
                and all(mask & ~remaining for mask in self.groups.values()))


def problem_region(donors, atoms, actions=(0,), gaps=(0,)):
    return dict(donors=donors, atoms=atoms, actions=actions, gaps=gaps)


class RegionContracts(unittest.TestCase):
    def models(self, source, samples, region, **options):
        reports = []
        options.setdefault("timeout_ms", 1000)
        output = list(SAT.region_models(source, samples, clone, lambda solver: solver.check(),
            lambda info, _: reports.append(info), region, **options))
        self.assertTrue(reports)
        self.assertTrue(all(SAT.compatible_first_witness(candidate, samples) for candidate in output))
        return output, reports

    def test_native_chain_is_right_associated_and_renders_without_parentheses(self):
        truth = [sum(1 << row for row in range(16) if row & (1 << feature)) for feature in range(4)]
        target = truth[0] & (truth[1] | (truth[2] & truth[3]))
        samples = Samples(truth + [target], [target], atoms=["A", "B", "C", "D", "old" * 40])
        source = [{"action": 0, "atoms": [4]}]
        problem = SAT.RegionProblem(source, samples, clone, problem_region((0,), (0, 1, 2, 3)),
            max_slots=1, max_terms=4, timeout_ms=1000)
        slot = problem.slots[0]
        problem.solver.add(slot["on"], slot["length"] == 4)
        for variable, value in zip(slot["atoms"], range(4)):
            problem.solver.add(variable == value)
        for variable, value in zip(slot["ops"], (True, False, True)):
            problem.solver.add(variable == value)
        self.assertEqual(problem.solver.check(), z3.sat)
        candidate = problem.decode(problem.solver.model())
        self.assertEqual(CONDITIONS.condition_text(candidate[0], samples.atoms), "A&B|C&D")
        self.assertEqual(samples.hit(candidate[0]), target)

    def mixed_scope_case(self, permutation=(0, 1, 2, 3)):
        truth = [sum(1 << row for row in range(16) if row & (1 << feature)) for feature in range(4)]
        target = truth[0] & (truth[1] | (truth[2] & truth[3]))
        reordered = [truth[index] for index in permutation]
        atoms = ["ABCD"[index] for index in permutation]
        inverse = {old: permutation.index(old) for old in range(4)}
        samples = Samples(reordered, [target], atoms=atoms)
        source = [{"action": 0, "atoms": [inverse[0], inverse[1]]},
                  {"action": 0, "atoms": [inverse[0], inverse[2], inverse[3]]}]
        return samples, source, target

    def test_solver_learns_mixed_shared_scope_across_old_rule_ownership(self):
        samples, source, target = self.mixed_scope_case()
        output, _ = self.models(source, samples, problem_region((0, 1), (0, 1, 2, 3)),
                                max_slots=1, max_terms=4, max_models=1)
        self.assertTrue(output)
        self.assertEqual(samples.hit(output[0][0]), target)
        self.assertIn("|", output[0][0]["ops"])
        self.assertIn("&", output[0][0]["ops"])
        self.assertLess(sum(samples.rule_cost(rule) + 1 for rule in output[0]),
                        sum(samples.rule_cost(rule) + 1 for rule in source))

    def test_equal_cost_atom_catalog_permutation_keeps_scope_and_behavior(self):
        outputs = []
        for permutation in ((0, 1, 2, 3), (3, 1, 0, 2)):
            samples, source, target = self.mixed_scope_case(permutation)
            candidates, _ = self.models(source, samples, problem_region((0, 1), (0, 1, 2, 3)),
                                        max_slots=1, max_terms=4, max_models=1)
            self.assertTrue(candidates)
            outputs.append((samples.hit(candidates[0][0]),
                            sum(samples.rule_cost(rule) + 1 for rule in candidates[0])))
        self.assertEqual(outputs[0], outputs[1])

    def test_window_atoms_must_share_one_observation_witness(self):
        samples = Samples([1, 2, 3], [3], atoms=["A", "B", "old" * 40],
                          groups={0: 3}, cursors=[0, 0], required=2)
        source = [{"action": 0, "atoms": [2]}]
        problem = SAT.RegionProblem(source, samples, clone, problem_region((0,), (0, 1)),
            max_slots=1, max_terms=2, timeout_ms=1000)
        slot = problem.slots[0]
        problem.solver.add(slot["on"], slot["length"] == 2,
                           slot["atoms"][0] == 0, slot["atoms"][1] == 1, slot["ops"][0])
        self.assertEqual(problem.solver.check(), z3.unsat)

    def test_early_legal_witness_does_not_require_old_deadline_snapshot_to_hit(self):
        samples = Samples([1, 2], [3], atoms=["A", "old" * 40],
                          groups={0: 3}, cursors=[0, 0], required=2)
        source = [{"action": 0, "atoms": [1]}]
        output, _ = self.models(source, samples, problem_region((0,), (0,)),
                                max_slots=1, max_terms=1)
        early = [{"action": 0, "atoms": [0]}]
        self.assertFalse(samples.compatible(early))
        self.assertTrue(SAT.compatible_first_witness(early, samples))
        self.assertIn(early, output)

    def test_wait_before_first_witness_and_terminal_wait_remain_binding(self):
        samples = Samples([2, 7], [2], atoms=["go", "old" * 40],
                          groups={0: 2}, cursors=[0, 0, 1], required=2)
        source = [{"action": 0, "atoms": [1]}]
        default = [{"action": 0, "atoms": []}]
        self.assertFalse(SAT.compatible_first_witness(default, samples))
        output, reports = self.models(source, samples, problem_region((0,), (0,)),
            max_slots=1, max_terms=1, default_action=0)
        self.assertEqual(output, [])
        self.assertEqual(reports[-1]["status"], "unsat")

    def test_same_action_two_to_two_can_repartition_across_original_lines(self):
        samples = Samples([7, 8, 21, 5, 26], [15, 16],
            atoms=["old-left" * 15, "old-right" * 15, "B", "x", "y"])
        source = [{"action": 0, "atoms": [0]}, {"action": 1, "atoms": [2]},
                  {"action": 0, "atoms": [1]}]
        expected = [{"action": 0, "atoms": [3]}, {"action": 1, "atoms": [2]},
                    {"action": 0, "atoms": [4]}]
        self.assertTrue(samples.compatible(source))
        output, _ = self.models(source, samples, problem_region((0, 2), (3, 4), gaps=(0, 1)),
                                max_slots=2, max_terms=2, max_models=8)
        self.assertIn(expected, output)
        self.assertEqual(len(output[0]), 3)
        self.assertNotEqual(samples.selections(source)[0], samples.selections(expected)[0])

    def test_cross_action_exception_and_default_are_chosen_together(self):
        samples = Samples([1, 2, 1], [1, 2], atoms=["old-A" * 15, "old-B" * 15, "x"])
        source = [{"action": 0, "atoms": [0]}, {"action": 1, "atoms": [1]}]
        output, _ = self.models(source, samples, problem_region((0, 1), (2,), actions=(0, 1)),
            max_slots=2, max_terms=1, default_action=1)
        self.assertIn([{"action": 0, "atoms": [2]}, {"action": 1, "atoms": []}], output)

    def test_unavailable_wrong_action_does_not_need_false_condition(self):
        samples = Samples([1, 2], [1, 2], executable=[1, 3], atoms=["old" * 30, "B"])
        source = [{"action": 0, "atoms": [0]}, {"action": 1, "atoms": [1]}]
        output, _ = self.models(source, samples, problem_region((0,), (), gaps=(0, 1)),
                                max_slots=1, max_terms=0)
        self.assertTrue(any(candidate[0] == {"action": 0, "atoms": []} for candidate in output))

    def test_fixed_predecessor_can_legally_shield_a_matching_default(self):
        samples = Samples([1, 2], [1, 2], atoms=["A", "old" * 30])
        source = [{"action": 0, "atoms": [0]}, {"action": 1, "atoms": [1]}]
        output, _ = self.models(source, samples, problem_region((1,), (), actions=(1,), gaps=(0, 1)),
                                max_slots=1, max_terms=0)
        self.assertEqual(output, [[{"action": 0, "atoms": [0]}, {"action": 1, "atoms": []}]])

    def test_cost_is_copyable_utf16_including_guard_separators_and_newlines(self):
        samples = Samples([1, 2, 1], [1, 2], atoms=["old-A" * 15, "old-B" * 15, "x"])
        source = [{"action": 0, "atoms": [0]}, {"action": 1, "atoms": [1]}]
        problem = SAT.RegionProblem(source, samples, clone,
            problem_region((0, 1), (2,), actions=(0, 1)), max_slots=2, max_terms=1, timeout_ms=1000)
        self.assertEqual(problem.solver.check(), z3.sat)
        model = problem.solver.model()
        decoded = problem.decode(model)
        self.assertEqual(model.eval(problem.total_cost).as_long(),
                         sum(samples.rule_cost(rule) + 1 for rule in decoded))

    def test_truth_equal_clocks_remain_separate_structure_suggestions(self):
        samples = Samples([1, 1, 1, 2], [1], atoms=["old" * 20, "bufftime:X<1.0", "bufftime:X<1.1", "B"])
        source = [{"action": 0, "atoms": [0]}, {"action": 0, "atoms": [0]}]
        proposal = REGION.propose_region(source, samples, (0, 1), max_atoms=4, structural=False)
        self.assertTrue({1, 2} <= set(proposal["atoms"]))

    def test_unknown_is_reported_and_does_not_claim_unsat(self):
        samples = Samples([1], [1], atoms=["old" * 20])
        source = [{"action": 0, "atoms": [0]}, {"action": 0, "atoms": [0]}]
        reports = []
        output = list(REGION.region_edits(source, samples, clone, lambda _: z3.unknown,
            lambda info, _: reports.append(info), regions=[(0, 1)], default_branches=False,
            structural=False, max_models=1))
        self.assertEqual(output, [])
        self.assertEqual(next(info for info in reports if info["kind"] == "region_sat")["status"], "unknown")
        self.assertEqual(reports[-1]["unknown_checks"], 1)
        self.assertEqual(reports[-1]["unsat_checks"], 0)

    def test_cancel_propagates_instead_of_becoming_negative_feedback(self):
        samples = Samples([1], [1], atoms=["old" * 20])
        source = [{"action": 0, "atoms": [0]}]
        reports = []

        def stop(_):
            raise InterruptedError("cancelled")

        with self.assertRaises(InterruptedError):
            list(SAT.region_models(source, samples, clone, stop,
                lambda info, _: reports.append(info), problem_region((0,), (0,)),
                max_slots=1, max_terms=1))
        self.assertEqual(reports[0]["status"], "cancelled")

    def test_region_discovery_includes_nonadjacent_same_action_pair(self):
        samples = Samples([1, 2, 4], [5, 2])
        source = [{"action": 0, "atoms": [0]}, {"action": 1, "atoms": [1]},
                  {"action": 0, "atoms": [2]}]
        self.assertIn((0, 2), REGION.discover_regions(source, samples, max_regions=8))


if __name__ == "__main__":
    unittest.main()
