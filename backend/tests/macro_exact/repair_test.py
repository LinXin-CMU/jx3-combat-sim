"""Native-chain safety and reached-prefix contracts for local repair proposals.

Repairs are hypotheses. Even a mathematically plausible timer gate must pass
the real executor's wait scheduling and a complete replay before acceptance.
"""
import copy
import importlib.util
from itertools import product
import json
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[3]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


repair = load("repair_contracts", ROOT/"tools/exact_macro_repair.py")
conditions = load("repair_contract_conditions", ROOT/"tools/exact_macro_conditions.py")


def row(values, cursor=0, time=2.0, executable=None, allowed=None, rejected=None, state=None):
    return {"truth": values, "cursor": cursor, "time": time,
        "executable": [True] if executable is None else executable,
        "allowed": [] if allowed is None else allowed,
        "rejected_actions": [0] if rejected is None else rejected,
        "wait_allowed": not allowed,
        "state": {"buffs": [{"name": "Clock", "remaining": 10.0}], "target_buffs": []}
            if state is None else state}


def failure(rows, cursor=0, actual=None, early=True, kind="cast_mismatch"):
    return {"status": "ok", "rows": rows,
        "actual": [{"macro_line": 1, "macro_page": 1}] if actual is None else actual,
        "probe_failure": {"kind": kind, "index": cursor},
        "comparison": {"time_tolerance_seconds": 0.0625,
            "first_difference": {"index": cursor,
                "expected": {"skill_id": 100, "time": 2.2},
                "actual": {"skill_id": 100, "time": 2.0 if early else 2.4}}}}


def packed(values):
    data = bytes(sum(bool(value) << bit for bit, value in enumerate(values[start:start+8]))
        for start in range(0, len(values), 8))
    return {"truth_hex": data.hex(), "truth_count": len(values)}


class NativeRepairContracts(unittest.TestCase):
    def test_native_or_gate_binds_the_complete_original_chain(self):
        atoms = ["buff:A", "buff:B", "bufftime:Clock<9.8"]
        rules = [{"action": 0, "atoms": [0, 1], "ops": ["|"]}]
        trace = failure([row([True, False, False])])
        proposals = repair.edits(rules, atoms, trace)
        gate = next(trial[0] for kind, trial in proposals if kind == "repair_clock_gate")
        self.assertEqual(gate, {"action": 0, "atoms": [2, 0, 1], "ops": ["&", "|"]})
        self.assertEqual(conditions.condition_text(gate, atoms), "bufftime:Clock<9.8&buff:A|buff:B")
        appended = {"atoms": [0, 1, 2], "ops": ["|", "&"]}
        self.assertEqual(conditions.condition_mask(appended, [1, 0, 0], 1), 1)
        for a, b, clock in product((False, True), repeat=3):
            values = [int(a), int(b), int(clock)]
            self.assertEqual(bool(conditions.condition_mask(gate, values, 1)), clock and (a or b))
        self.assertEqual(rules, [{"action": 0, "atoms": [0, 1], "ops": ["|"]}])

    def test_alternating_native_ops_survive_a_front_gate(self):
        atoms = ["buff:A", "buff:B", "buff:C", "buff:D", "bufftime:Clock<9.8"]
        original = {"action": 0, "atoms": [0, 1, 2, 3], "ops": ["|", "&", "|"]}
        gate = next(trial[0] for kind, trial in repair.edits([original], atoms,
            failure([row([True, False, False, False, False])])) if kind == "repair_clock_gate")
        self.assertEqual(gate["ops"], ["&", "|", "&", "|"])
        for values in product((0, 1), repeat=5):
            self.assertEqual(conditions.condition_mask(gate, values, 1),
                values[4] & conditions.condition_mask(original, values, 1))

    def test_legacy_any_atoms_normalizes_without_changing_its_meaning(self):
        atoms = ["buff:P", "buff:A", "buff:B", "bufftime:Clock<9.8"]
        rules = [{"action": 0, "atoms": [0], "any_atoms": [1, 2]}]
        before = copy.deepcopy(rules)
        gate = next(trial[0] for kind, trial in repair.edits(rules, atoms,
            failure([row([True, True, False, False])])) if kind == "repair_clock_gate")
        self.assertEqual(gate, {"action": 0, "atoms": [3, 0, 1, 2], "ops": ["&", "&", "|"]})
        for values in product((0, 1), repeat=4):
            self.assertEqual(conditions.condition_mask(gate, values, 1),
                values[3] & conditions.condition_mask(before[0], values, 1))
        self.assertEqual(rules, before)

    def test_or_positive_witness_is_preserved_without_unused_rule_drop(self):
        atoms = ["buff:A", "buff:B", "bufftime:Clock<9.8"]
        rules = [{"action": 0, "atoms": [0, 1], "ops": ["|"]}]
        rows = [row([True, False, True], cursor=0, allowed=[0], rejected=[]),
                row([True, False, False], cursor=1)]
        trace = failure(rows, cursor=1, actual=[{"macro_line": 1}, {"macro_line": 1}])
        proposals = repair.edits(rules, atoms, trace)
        self.assertNotIn("repair_drop_unused", [kind for kind, _ in proposals])
        gate = next(trial[0] for kind, trial in proposals if kind == "repair_clock_gate")
        self.assertTrue(conditions.condition_mask(gate, [1, 0, 1], 1))
        self.assertFalse(conditions.condition_mask(gate, [1, 0, 0], 1))

    def test_split_keeps_the_whole_native_expression_in_both_branches(self):
        atoms = ["buff:A", "buff:B", "buff:C", "buff:D"]
        rules = [{"action": 0, "atoms": [0, 1], "ops": ["|"]}]
        rows = [row([True, False, True, False], cursor=0, allowed=[0], rejected=[]),
                row([False, True, False, True], cursor=1, allowed=[0], rejected=[]),
                row([True, False, False, False], cursor=2)]
        trace = failure(rows, cursor=2, actual=[{"macro_line": 1}]*3)
        split = next(trial for kind, trial in repair.edits(rules, atoms, trace) if kind == "repair_split")
        self.assertEqual(len(split), 2)
        for branch in split:
            self.assertEqual(branch["atoms"][1:], [0, 1])
            self.assertEqual(branch["ops"], ["&", "|"])
        for values in ([1, 0, 1, 0], [0, 1, 0, 1]):
            self.assertTrue(any(conditions.condition_mask(branch, values, 1) for branch in split))
        self.assertFalse(any(conditions.condition_mask(branch, [1, 0, 0, 0], 1) for branch in split))

    def test_threshold_substitution_preserves_every_unrelated_operator(self):
        atoms = ["bufftime:Clock<9.8", "bufftime:Clock<9.9", "buff:B", "buff:C", "buff:D"]
        rules = [{"action": 0, "atoms": [0, 2], "ops": ["|"]},
                 {"action": 1, "atoms": [3, 2, 4], "ops": ["|", "&"]},
                 {"action": 2, "atoms": [3], "any_atoms": [2, 4]}]
        before = copy.deepcopy(rules)
        proposals = repair.edits(rules, atoms,
            failure([row([True, True, False, True, False], executable=[1, 1, 1])], early=False))
        trial = next(trial for kind, trial in proposals if kind == "repair_threshold")
        self.assertEqual(trial[0], {"action": 0, "atoms": [1, 2], "ops": ["|"]})
        self.assertEqual(trial[1], rules[1])
        self.assertEqual(conditions.condition_key(trial[2]), conditions.condition_key(rules[2]))
        trial[1]["ops"].pop()
        trial[2]["atoms"].append(0)
        self.assertEqual(rules, before)

    def test_threshold_direction_handles_less_and_greater_for_early_and_late(self):
        for operator, early, first in (("<", True, 1.9), ("<", False, 2.1),
                                      (">", True, 2.1), (">", False, 1.9)):
            with self.subTest(operator=operator, early=early):
                atoms = [f"bufftime:Clock{operator}{point:.1f}" for point in (2.0, 1.9, 2.1, 1.8, 2.2)]
                rules = [{"action": 0, "atoms": [0]}]
                proposals = repair.edits(rules, atoms, failure([row([True]*5)], early=early))
                trial = next(trial for kind, trial in proposals if kind == "repair_threshold")
                self.assertEqual(atoms[trial[0]["atoms"][0]], f"bufftime:Clock{operator}{first:.1f}")

    def test_pure_terminal_wait_has_no_forced_missing_action(self):
        rules = [{"action": 0, "atoms": [0]}]
        trace = failure([row([False], rejected=[])], actual=[], kind="missed_decision_time")
        self.assertEqual(repair.edits(rules, ["bufftime:Clock<9.8"], trace), [])

    def test_missing_deadline_uses_only_aligned_allowed_executable_rules(self):
        atoms = ["bufftime:Clock<15.3", "bufftime:Clock<15.4", "buff:B"]
        rules = [{"action": 0, "atoms": [0]},
                 {"action": 1, "atoms": [0, 2], "ops": ["|"]},
                 {"action": 2, "atoms": [0]}]
        trace = failure([row([False, True, False], cursor=1, executable=[1, 1, 0],
            allowed=[1, 2], rejected=[])], cursor=1, actual=[{}], kind="missed_decision_time")
        proposals = repair.edits(rules, atoms, trace)
        self.assertTrue(proposals)
        self.assertIn("repair_threshold", [kind for kind, _ in proposals])
        source = repair.copy_rules(rules)
        for kind, trial in proposals:
            if kind == "repair_threshold":
                self.assertEqual(trial[0], source[0])
                self.assertEqual(trial[1], {"action": 1, "atoms": [1, 2], "ops": ["|"]})
                self.assertEqual(trial[2], source[2])
            else:
                self.assertEqual(kind, "repair_coverage")
                inserted = [index for index in range(len(trial))
                    if trial[:index]+trial[index+1:] == source]
                self.assertTrue(inserted)
                self.assertTrue(all(trial[index]["action"] == 1 for index in inserted))
        trace["rows"][-1]["cursor"] = 2
        self.assertEqual(repair.edits(rules, atoms, trace), [])

    def test_unvisited_suffix_rows_cannot_change_repair_proposals(self):
        atoms = ["buff:A", "buff:B", "bufftime:Clock<9.8"]
        rules = [{"action": 0, "atoms": [0, 1], "ops": ["|"]}]
        trace = failure([row([True, False, False])])
        expected = repair.edits(rules, atoms, trace)
        trace["rows"].extend([row([False, True, True], cursor=1, allowed=[0]),
                              row([True, True, True], cursor=7, allowed=[0])])
        trace["actual"].extend([{"macro_line": 1}, {"macro_line": 1}])
        self.assertEqual(repair.edits(rules, atoms, trace), expected)
        trace["rows"] = [row([True, False, False], cursor=1)]
        self.assertEqual(repair.edits(rules, atoms, trace), [])
        trace = failure([row([True, False, False])], actual=[{"macro_line": 1, "macro_page": 2}])
        self.assertEqual(repair.edits(rules, atoms, trace), [])

    def test_thin_truth_catalog_cannot_be_used_as_full_repair_columns(self):
        rules = [{"action": 0, "atoms": [0, 1], "ops": ["|"]}]
        trace = failure([row([False])])
        self.assertEqual(repair.edits(rules,
            ["buff:A", "buff:B", "bufftime:Clock<9.8"], trace), [])

    def test_legal_priority_move_does_not_require_later_wrong_guard_false(self):
        atoms = ["buff:A", "buff:B", "buff:C", "buff:D"]
        rules = [{"action": 0, "atoms": [0, 1], "ops": ["|"]},
                 {"action": 1, "atoms": [2, 3], "ops": ["|"]}]
        trace = failure([row([True, False, True, False], executable=[1, 1], allowed=[1])])
        trial = next(trial for kind, trial in repair.edits(rules, atoms, trace) if kind == "repair_priority")
        self.assertEqual([rule["action"] for rule in trial], [1, 0])
        self.assertTrue(conditions.condition_mask(trial[1], [1, 0, 1, 0], 1))
        self.assertTrue(conditions.condition_mask(trial[0], [1, 0, 1, 0], 1))
        trace["rows"][-1]["executable"][1] = False
        self.assertNotIn("repair_priority", [kind for kind, _ in repair.edits(rules, atoms, trace)])

    def test_coverage_uses_native_or_priority_to_protect_earlier_selections(self):
        atoms = ["buff:A", "buff:B", "buff:C"]
        rules = [{"action": 0, "atoms": [0, 1], "ops": ["|"]},
                 {"action": 1, "atoms": [1]}]
        rows = [row([True, False, False], cursor=0, executable=[1, 1], allowed=[0], rejected=[]),
                row([False, False, True], cursor=1, executable=[1, 1], allowed=[1], rejected=[])]
        trace = failure(rows, cursor=1, actual=[{"macro_line": 1}], kind="missed_decision_time")
        proposals = repair.edits(rules, atoms, trace)
        unrestricted = [trial for kind, trial in proposals if kind == "repair_coverage"
            and any(rule["action"] == 1 and not rule["atoms"] for rule in trial)]
        self.assertTrue(unrestricted)
        for trial in unrestricted:
            for values, expected in (([1, 0, 0], 0), ([0, 0, 1], 1)):
                selected = next(rule["action"] for rule in trial
                    if conditions.condition_mask(rule, values, 1))
                self.assertEqual(selected, expected)

    def test_packed_truth_uses_native_byte_and_bit_order(self):
        expected = [True, False, True, False, False, False, False, True, True]
        self.assertEqual(list(repair.truth({"truth_hex": "8501", "truth_count": 9})), expected)
        self.assertEqual(packed(expected), {"truth_hex": "8501", "truth_count": 9})

    def test_packed_and_expanded_trace_produce_identical_repairs(self):
        atoms = ["buff:A", "buff:B"] + [f"buff:unused{n}" for n in range(6)] + ["bufftime:Clock<9.8"]
        rules = [{"action": 0, "atoms": [0, 1], "ops": ["|"]}]
        values = [True, False]+[False]*7
        trace = failure([row(values)])
        expected = repair.edits(rules, atoms, trace)
        packed_trace = copy.deepcopy(trace)
        packed_trace["rows"][-1].pop("truth")
        packed_trace["rows"][-1].update(packed(values))
        self.assertEqual(repair.edits(rules, atoms, packed_trace), expected)

    def test_clock_gates_require_false_native_truth_and_correct_future_window(self):
        atoms = ["bufftime:Clock<9.8", "bufftime:Clock<9.7", "bufftime:Clock<9.9",
                 "bufftime:Clock>9.8", "tbufftime:Target<4.8", "bufftime:Absent<9.8"]
        state = {"buffs": [{"name": "Clock", "remaining": 10.0}],
                 "target_buffs": [{"name": "Target", "remaining": 5.0}]}
        observed = row([False, False, True, False, False, False], state=state)
        difference = failure([observed])["comparison"]["first_difference"]
        self.assertEqual(repair.clock_gates(atoms, observed, difference, 0.0625), [0, 4])
        difference["actual"]["skill_id"] = 101
        self.assertEqual(repair.clock_gates(atoms, observed, difference, 0.0625), [])
        difference["actual"] = {"skill_id": 100, "time": 2.3}
        self.assertEqual(repair.clock_gates(atoms, observed, difference, 0.0625), [])

    def test_proposal_limit_and_cancellation_checks_are_finite(self):
        atoms = ["buff:A", "buff:B", "bufftime:Clock<9.8"]
        rules = [{"action": 0, "atoms": [0, 1], "ops": ["|"]}]
        trace = failure([row([True, False, False])])
        calls = []
        proposals = repair.edits(rules, atoms, trace, check=lambda: calls.append(True), limit=1)
        self.assertEqual(len(proposals), 1)
        self.assertTrue(calls)
        self.assertEqual(repair.edits(rules, atoms, trace, limit=0), [])


class NativeClockReplay(unittest.TestCase):
    def test_predicted_gate_still_requires_actual_wake_truth_and_full_replay(self):
        exe = ROOT/"backend/target/release/jx3-combat-sim.exe"
        if not exe.is_file():
            exe = exe.with_suffix("")
        if not exe.is_file():
            self.skipTest("native release oracle is unavailable")
        synth = load("repair_native_synth", ROOT/"tools/exact-macro-synth.py")
        scene = json.loads((ROOT/"backend/tests/fixtures/exact_macro_short.json").read_text(encoding="utf-8-sig"))
        scene["simulation"].update(sequence=["盾刀"], timing_offsets={"0": 0.2},
            pre_releases=[{"skill": "血怒", "time_before": 1.0}])
        scene.update(horizon=1.0, acceptance="skills_and_time", time_tolerance_seconds=0.0625)
        atoms = ["rage>-1", "rage<0", "bufftime:血怒<8.8"]
        actions = [{"name": "盾刀", "fcast": False}]
        rules = [{"action": 0, "atoms": [0, 1], "ops": ["|"]}]
        oracle = synth.Oracle(exe)
        try:
            failed = oracle.run(dict(scene, candidate=synth.macro_text(rules, atoms, actions),
                atoms=atoms, stop_on_divergence=True))
            self.assertFalse(failed["comparison"]["reproduced"])
            self.assertFalse(failed["comparison"]["completed_full_replay"])
            gate = next(trial for kind, trial in repair.edits(rules, atoms, failed)
                if kind == "repair_clock_gate")
            self.assertEqual(gate[0]["ops"], ["&", "|"])
            replay = oracle.run(dict(scene, candidate=synth.macro_text(gate, atoms, actions),
                atoms=atoms, stop_on_divergence=False))
            self.assertTrue(replay["comparison"]["completed_full_replay"])
            self.assertFalse(replay["truncated"])
            first, crossing = replay["rows"][:2]
            self.assertFalse(first["truth"][2])
            self.assertAlmostEqual(first["wait_next_time"], 0.2)
            self.assertAlmostEqual(crossing["time"], first["wait_next_time"])
            # Strict '<' is still false at the actual equality boundary; the
            # native next WAIT and actual cast, rather than predicted crossing,
            # determine whether the target's tolerance is met.
            self.assertFalse(crossing["truth"][2])
            self.assertGreater(crossing["wait_next_time"], 0.2+0.0625)
            self.assertGreater(replay["actual"][0]["cast_time"], 0.2+0.0625)
            self.assertFalse(replay["comparison"]["reproduced"])
        finally:
            oracle.close()
            oracle.proc.stdout.close()


if __name__ == "__main__":
    unittest.main()
