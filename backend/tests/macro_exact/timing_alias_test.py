"""Timer-alias proposals preserve native chains and remain replay hypotheses."""
import copy
import importlib.util
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[3]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


repair = load("timing_alias_repair", ROOT/"tools/exact_macro_repair.py")
compress = load("timing_alias_compress", ROOT/"tools/exact_macro_compress.py")
conditions = load("timing_alias_conditions", ROOT/"tools/exact_macro_conditions.py")


def fixture(atoms, values, rules=None, latest=2.2625, cast_time=2.2):
    rules = [{"action": 0, "atoms": [0]}] if rules is None else rules
    action_count = max(rule["action"] for rule in rules)+1
    observed = dict(truth=values, executable=[True]*action_count, allowed=[0],
        cursor=0, time=2.0, decision_latest=latest, wait_allowed=False, state=None)
    samples = compress.Samples([observed], atoms,
        [{"name": "skill"+str(i), "fcast": False} for i in range(action_count)], lambda: None)
    replay = dict(status="ok", truncated=False,
        actual=[{"macro_line": 1, "macro_page": 1, "cast_time": cast_time}],
        comparison={"reproduced": True, "completed_full_replay": True,
            "time_tolerance_seconds": 0.0625})
    return rules, samples, replay


class TimingAliasContracts(unittest.TestCase):
    def test_active_self_and_target_timers_are_inferred_but_absent_timer_is_not(self):
        atoms = ["bufftime:VeryLongClock<9.9", "bufftime:N<9.8", "bufftime:N>9.9", "bufftime:N<10.1",
                 "tbufftime:T<4.8", "tbufftime:T>4.9", "tbufftime:T<5.1",
                 "bufftime:Absent<9.8", "bufftime:Absent>0.1", "bufftime:Absent<10.1"]
        values = [True, False, True, True, False, True, True, False, False, False]
        rules, samples, replay = fixture(atoms, values)
        proposals = repair.timing_aliases(rules, samples, replay)
        replacements = {trial[0]["atoms"][0] for _, trial in proposals}
        self.assertTrue({1, 4} <= replacements)
        self.assertNotIn(7, replacements)
        self.assertTrue(all(kind == "beam_timing_alias" for kind, _ in proposals))

    def test_native_or_operators_and_unrelated_guards_survive_without_source_compatibility(self):
        atoms = ["bufftime:VeryLongClock<9.9", "buff:P", "buff:Q",
                 "bufftime:N<9.8", "bufftime:N>9.9", "bufftime:N<10.1"]
        rules = [{"action": 0, "atoms": [0, 1, 2], "ops": ["|", "&"]},
                 {"action": 1, "atoms": [1], "any_atoms": [2]}]
        original = copy.deepcopy(rules)
        rules, samples, replay = fixture(atoms, [True, False, False, False, True, True], rules)
        trial = next(trial for _, trial in repair.timing_aliases(rules, samples, replay)
            if trial[0]["atoms"][0] == 3)
        self.assertEqual(trial[0]["ops"], ["|", "&"])
        self.assertEqual(trial[0]["atoms"], [3, 1, 2])
        self.assertEqual(conditions.condition_key(trial[1]), conditions.condition_key(original[1]))
        self.assertTrue(samples.compatible(rules))
        # The false-now replacement intentionally asks the native scheduler for
        # another allowed time; it need not select at the source's snapshot.
        self.assertFalse(samples.compatible(trial))
        trial[0]["ops"].pop()
        trial[1]["atoms"].append(0)
        self.assertEqual(rules, original)

    def test_original_decision_deadline_includes_delay_and_rejects_outside_clocks(self):
        atoms = ["bufftime:VeryLongClock<9.9", "bufftime:N<9.8", "bufftime:N<9.3",
                 "bufftime:N>9.9", "bufftime:N<10.0", "bufftime:N<10.1"]
        values = [True, False, False, True, False, True]
        rules, samples, replay = fixture(atoms, values, cast_time=2.7)
        # A 0.5-second cast delay moves the cast timestamp while the native
        # decision window remains centered at 2.2, as recorded in the row.
        replacements = {trial[0]["atoms"][0] for _, trial in repair.timing_aliases(rules, samples, replay)}
        self.assertIn(1, replacements)
        self.assertNotIn(2, replacements)
        samples.rows[0]["decision_latest"] = None
        self.assertEqual(repair.timing_aliases(rules, samples, replay), [])
        samples.rows[0]["decision_latest"] = 2.2625
        samples.rows[0]["time"] = 2.2
        self.assertEqual(repair.timing_aliases(rules, samples, replay), [])

    def test_alias_cost_never_exceeds_the_source_rule(self):
        atoms = ["bufftime:S<9.9", "bufftime:T<9.8", "bufftime:T>9.9", "bufftime:T<10.1",
                 "bufftime:ExpensiveClock<9.8", "bufftime:ExpensiveClock>9.9", "bufftime:ExpensiveClock<10.1"]
        rules, samples, replay = fixture(atoms, [True, False, True, True, False, True, True])
        proposals = repair.timing_aliases(rules, samples, replay)
        self.assertTrue(proposals)
        self.assertTrue(any(trial[0]["atoms"] == [1] for _, trial in proposals))
        self.assertTrue(all(4 not in trial[0]["atoms"] for _, trial in proposals))
        self.assertTrue(all(samples.rule_cost(trial[0]) <= samples.rule_cost(rules[0])
            for _, trial in proposals))

    def test_deterministic_line_shares_limit_and_cancellation(self):
        atoms = ["bufftime:OldLongOne<9.9", "bufftime:OldLongTwo<9.9",
                 "bufftime:T<9.8", "bufftime:T<9.9", "bufftime:T>9.9", "bufftime:T<10.1",
                 "bufftime:U<9.8", "bufftime:U<9.9", "bufftime:U>9.9", "bufftime:U<10.1"]
        rules = [{"action": 0, "atoms": [0]}, {"action": 1, "atoms": [1]}]
        values = [True, True, False, False, True, True, False, False, True, True]
        rows = [dict(truth=list(values), executable=executable, allowed=[i], cursor=i,
            time=2.0+i, decision_latest=2.2625+i, wait_allowed=False, state=None)
            for i, executable in enumerate(([True, False], [False, True]))]
        checks = []
        samples = compress.Samples(rows, atoms,
            [{"name": "skill"+str(i), "fcast": False} for i in range(2)], lambda: checks.append(True))
        replay = dict(actual=[{"macro_line": 1}, {"macro_line": 2}],
            comparison={"reproduced": True, "time_tolerance_seconds": 0.0625})
        expected = repair.timing_aliases(rules, samples, replay, limit=2)
        self.assertEqual(len(expected), 2)
        self.assertEqual(repair.timing_aliases(rules, samples, replay, limit=2), expected)
        changed_lines = {next(i for i, rule in enumerate(trial)
            if conditions.condition_key(rule) != conditions.condition_key(rules[i]))
            for _, trial in expected}
        self.assertEqual(changed_lines, {0, 1})
        self.assertTrue(checks)
        self.assertEqual(repair.timing_aliases(rules, samples, replay, limit=0), [])

        def cancelled():
            raise RuntimeError("cancel signal")

        samples.check = cancelled
        with self.assertRaisesRegex(RuntimeError, "cancel signal"):
            repair.timing_aliases(rules, samples, replay)

    def test_predictions_do_not_attach_or_modify_a_replay_certificate(self):
        atoms = ["bufftime:VeryLongClock<9.9", "bufftime:N<9.8", "bufftime:N>9.9", "bufftime:N<10.1"]
        rules, samples, replay = fixture(atoms, [True, False, True, True])
        original_rules, original_replay = copy.deepcopy(rules), copy.deepcopy(replay)
        proposals = repair.timing_aliases(rules, samples, replay)
        self.assertTrue(proposals)
        self.assertEqual(rules, original_rules)
        self.assertEqual(replay, original_replay)
        for _, trial in proposals:
            self.assertTrue(all(set(rule) <= {"action", "atoms", "ops"} for rule in trial))
        replay["comparison"]["reproduced"] = False
        self.assertEqual(repair.timing_aliases(rules, samples, replay), [])


if __name__ == "__main__":
    unittest.main()
