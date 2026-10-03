"""Whole-guard SAT and anonymous structural-prior boundary regressions."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest

import z3


ROOT = Path(__file__).resolve().parents[3]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


TOOLS = ROOT / "tools"
GUARDS = load("region_guards", TOOLS / "exact_macro_region_guards.py")
SAT = load("region_sat", TOOLS / "exact_macro_region_sat.py")
LEARNING = load("guard_prior_learning", TOOLS / "exact_macro_learning.py")
FIXTURES = load("guard_fixtures", Path(__file__).with_name("region_test.py"))
CONDITIONS = GUARDS.conditions()


class WholeGuardContracts(unittest.TestCase):
    def guards(self, source, samples, region, **kwargs):
        output, stats = GUARDS.guard_candidates(source, samples, region, **kwargs)
        self.assertTrue(output)
        self.assertTrue(all(guard["mask"] == CONDITIONS.condition_mask(guard, samples.truth, samples.all)
                            for guard in output))
        self.assertTrue(all(guard["cost"] == CONDITIONS.condition_cost(guard, samples.atom_costs)
                            for guard in output))
        return output, stats

    def test_equal_truth_only_merges_nonclock_leaves(self):
        samples = FIXTURES.Samples([1, 1, 1, 1, 2], [1], atoms=[
            "buff:long-name", "rage>0", "bufftime:X<1.0", "bufftime:X<1.1", "wait"])
        source = [{"action": 0, "atoms": [0]}]
        region = FIXTURES.problem_region((0,), (0, 1, 2, 3))
        region["events"] = (1,)
        output, _ = self.guards(source, samples, region, max_terms=1, max_candidates=12)
        matching = [guard for guard in output if guard["mask"] == 1]
        self.assertEqual({tuple(guard["clock_key"]) for guard in matching}, {(), (2,), (3,)})
        stable = [guard for guard in matching if not guard["clock_key"]]
        self.assertEqual(stable[0]["atoms"], [1])
        self.assertTrue(any(guard["atoms"] == [0] for guard in stable))

    def test_mask_composition_obeys_native_right_fold(self):
        samples, source, target = FIXTURES.RegionContracts().mixed_scope_case()
        region = FIXTURES.problem_region((0, 1), (0, 1, 2, 3))
        region["events"] = tuple(samples.groups.values())
        library, _ = self.guards(source, samples, region, max_terms=4, max_candidates=64)
        matching = [guard for guard in library if guard["mask"] == target]
        self.assertTrue(matching)
        self.assertTrue(any("&" in guard["ops"] and "|" in guard["ops"] for guard in matching))
        problem = SAT.RegionProblem(source, samples, FIXTURES.clone, region,
            max_slots=1, max_terms=4, timeout_ms=1000, guard_library=library)
        self.assertEqual(problem.solver.check(), z3.sat)
        output = problem.decode(problem.solver.model())
        self.assertEqual(samples.hit(output[0]), target)

    def test_whole_guard_keeps_first_witness_not_old_tail_obligation(self):
        samples = FIXTURES.Samples([1, 2], [3], atoms=["rage>0", "old" * 40],
            groups={0: 3}, cursors=[0, 0], required=2)
        source = [{"action": 0, "atoms": [1]}]
        region = FIXTURES.problem_region((0,), (0, 1))
        region["events"] = (3,)
        library, _ = self.guards(source, samples, region, max_terms=1)
        problem = SAT.RegionProblem(source, samples, FIXTURES.clone, region,
            max_slots=1, max_terms=1, timeout_ms=1000, guard_library=library)
        desired = next(i for i, guard in enumerate(library) if guard["atoms"] == [0])
        problem.solver.add(problem.slots[0]["on"], problem.slots[0]["guard_choices"][desired])
        self.assertEqual(problem.solver.check(), z3.sat)
        self.assertTrue(SAT.compatible_first_witness(problem.decode(problem.solver.model()), samples))

    def test_whole_guard_default_cannot_bypass_wait(self):
        samples = FIXTURES.Samples([2, 3], [2], atoms=["go", "old" * 40],
            groups={0: 2}, cursors=[0, 0], required=2)
        source = [{"action": 0, "atoms": [1]}]
        region = FIXTURES.problem_region((0,), (0, 1))
        region["events"] = (2,)
        library, _ = self.guards(source, samples, region, max_terms=1)
        problem = SAT.RegionProblem(source, samples, FIXTURES.clone, region,
            max_slots=1, max_terms=1, timeout_ms=1000, guard_library=library, default_action=0)
        self.assertEqual(problem.solver.check(), z3.unsat)

    def test_prior_generates_anonymous_shapes_without_starving_ordinary_guards(self):
        samples = FIXTURES.Samples([1, 2, 3, 4], [3], atoms=["buff:A", "rage>0", "old" * 40, "wait"])
        source = [{"action": 0, "atoms": [2]}]
        region = FIXTURES.problem_region((0,), (0, 1, 2))
        region["events"] = tuple(samples.groups.values())
        prior = LEARNING.structural_prior([([{"action": 999, "atoms": [0, 1], "ops": ["|"]}],
                                          ["buff:unrelated-name", "rage>999"])])
        library, stats = self.guards(source, samples, region, max_terms=2,
            max_candidates=16, structural_prior=prior)
        ordinary, _ = self.guards(source, samples, region, max_terms=2, max_candidates=16)
        def record(guard):
            return (tuple(guard["atoms"]), tuple(guard["ops"]), guard["mask"],
                    guard["cost"], tuple(guard["clock_key"]))
        self.assertTrue({record(guard) for guard in ordinary}
                        <= {record(guard) for guard in library})
        self.assertGreater(stats["guard_prior_proposals"], 0)
        self.assertTrue(any(guard["ops"] == ["|"] for guard in library))
        self.assertTrue(any(guard["atoms"] == [0] for guard in library))
        self.assertTrue(any(not guard["atoms"] for guard in library))

    def test_prior_cannot_replace_frozen_ordinary_or_source_expression(self):
        samples = SimpleNamespace(all=3, atoms=["buff:LongSourceA", "buff:LongSourceB", "buff:X"],
            truth=[1, 1, 1], atom_costs=[40, 40, 6], executable=[3], allowed=[1], check=lambda: None)
        samples.hit = lambda rule: 1
        source = [dict(action=0, atoms=[0, 1], ops=["&"])]
        region = dict(atoms=(0, 1, 2), donors=(0,), actions=(0,), events=(1,))
        prior = LEARNING.structural_prior([([dict(action=0, atoms=[0])], ["buff:Anonymous"])])
        plain, _ = self.guards(source, samples, region, max_terms=1,
            max_expansions=2, max_candidates=8)
        learned, stats = self.guards(source, samples, region, max_terms=1,
            max_expansions=2, max_candidates=8, structural_prior=prior)
        def record(guard):
            return (tuple(guard["atoms"]), tuple(guard["ops"]), guard["mask"],
                    guard["cost"], tuple(guard["clock_key"]))
        self.assertTrue({record(guard) for guard in plain} <= {record(guard) for guard in learned})
        self.assertTrue(any(guard["atoms"] == [0, 1] and guard["ops"] == ["&"]
                            and guard["cost"] == 84 for guard in learned))
        self.assertTrue(any(guard["atoms"] == [2] and guard["mask"] == 1 for guard in learned))
        self.assertGreater(stats["guard_prior_added"], 0)

    def test_invalid_prior_is_disabled_without_invented_syntax(self):
        samples = FIXTURES.Samples([1, 2], [1], atoms=["buff:A", "wait"])
        source = [{"action": 0, "atoms": [0]}]
        region = FIXTURES.problem_region((0,), (0,))
        region["events"] = (1,)
        _, stats = self.guards(source, samples, region,
            structural_prior={"schema_version": 1, "native_semantics": "different", "shapes": []})
        self.assertEqual(stats["guard_prior_shapes"], 0)

    def test_library_masks_cannot_fake_a_cross_observation_and(self):
        samples = FIXTURES.Samples([1, 2, 3], [3], atoms=["A", "B", "old" * 40],
            groups={0: 3}, cursors=[0, 0], required=2)
        source = [{"action": 0, "atoms": [2]}]
        region = FIXTURES.problem_region((0,), (0, 1))
        region["events"] = (3,)
        library = [dict(atoms=[], ops=[], mask=3, cost=0, clock_key=()),
            dict(atoms=[0, 1], ops=["&"], mask=0, cost=6, clock_key=())]
        problem = SAT.RegionProblem(source, samples, FIXTURES.clone, region,
            max_slots=1, max_terms=2, timeout_ms=1000, guard_library=library)
        problem.solver.add(problem.slots[0]["on"], problem.slots[0]["guard_choices"][1])
        self.assertEqual(problem.solver.check(), z3.unsat)


if __name__ == "__main__":
    unittest.main()
