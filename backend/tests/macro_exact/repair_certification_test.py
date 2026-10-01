"""Compression-driver contracts for repairing an independently failed seed.

The fixture target has three casts and only a one-cast matching prefix. Same-
skill timing failure, full observation provenance, incumbent cost and complete
native certification must all be respected by the repair frontier.
"""
import copy
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[3]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


compress = load("repair_driver_compress", ROOT/"tools/exact_macro_compress.py")
conditions = load("repair_driver_conditions", ROOT/"tools/exact_macro_conditions.py")


ATOMS = ["buff:LongCondition", "buff:Short", "buff:ClockGate"]
ACTIONS = [{"name": "盾刀", "fcast": False}]
INCUMBENT = [{"action": 0, "atoms": [0]*20}]
SHORT_SEED = [{"action": 0, "atoms": [1]}, {"action": 0, "atoms": []}]
REPAIRED = [{"action": 0, "atoms": [2, 1], "ops": ["&"]}, {"action": 0, "atoms": []}]


def render(rules):
    lines = []
    for rule in rules:
        guard = conditions.condition_text(rule, ATOMS)
        lines.append("/cast " + ("["+guard+"] " if guard else "") + ACTIONS[rule["action"]]["name"])
    return "\n".join(lines)


def row(values, cursor, state=None):
    return dict(truth=values, cursor=cursor, time=float(cursor), executable=[True],
        allowed=[0], wait_allowed=False, state=state, last_skill=None,
        decision_latest=cursor+0.2625, wait_next_time=cursor+0.2,
        rejected_actions=[0] if cursor == 1 else [])


def passed(complete=True, truncated=False, status="ok"):
    return dict(status=status, truncated=truncated,
        actual_fingerprint="independent-certified-replay",
        rows=[row([True, True, True], 0), row([True, True, True], 1), row([True, True, True], 2)],
        comparison=dict(reproduced=True, completed_full_replay=complete,
            acceptance_prefix=3, exact_prefix=3, target_count=3, actual_count=3))


def failure(seed, full=False, fingerprint="failed-seed", same_skill=True, values=None):
    values = values if values is not None else [False, True, False]
    rows = [row([True, True, False] if full else [False], 0),
            row(values if full else [False], 1, state={"rage": 17})]
    return dict(status="ok", truncated=False, rows=rows,
        actual_fingerprint=fingerprint,
        actual=[{"macro_line": 1, "macro_page": 1},
                {"macro_line": len(seed), "macro_page": 1}],
        probe_failure={"index": 1, "kind": "cast_mismatch"},
        comparison=dict(reproduced=False, completed_full_replay=False,
            acceptance_prefix=1, exact_prefix=1, target_count=3, actual_count=2,
            first_difference={"index": 1,
                "expected": {"skill_id": 100, "time": 1.2},
                "actual": {"skill_id": 100 if same_skill else 101, "time": 1.0}}))


class RepairDriverCertification(unittest.TestCase):
    def run_driver(self, seeds, verify, repair, baseline=None, feedback=None):
        baseline = passed() if baseline is None else baseline
        accepted, diagnostics = [], []
        empty = {name: (lambda *args, **kwargs: []) for name in (
            "basic_batch", "feature_batch", "simple_edits", "feature_edits", "or_edits",
            "family_edits", "joint_edits", "window_edits", "event_window_edits", "timing_edits", "local_rewrites", "global_edits")}
        empty["priority_edits"] = lambda *args, **kwargs: (
            ("priority_short_seed", copy.deepcopy(seed)) for seed in seeds)
        empty["repair_edits"] = repair
        incumbent = copy.deepcopy(INCUMBENT)
        with patch.multiple(compress, **empty):
            result = compress.compress(incumbent, ATOMS, ACTIONS, baseline, render, verify,
                lambda: None, lambda solver: self.fail("no solver is needed in this contract"),
                lambda *args: accepted.append(args), lambda summary: None,
                lambda info, smt: diagnostics.append(info), feedback=feedback)
        self.assertEqual(incumbent, INCUMBENT)
        return result, accepted, diagnostics, baseline

    def test_short_failed_seed_may_grow_below_incumbent_and_then_be_certified(self):
        self.assertLess(compress.char_count(render(SHORT_SEED)), compress.char_count(render(REPAIRED)))
        self.assertLess(compress.char_count(render(REPAIRED)), compress.char_count(render(INCUMBENT)))
        failed, certified = failure(SHORT_SEED), passed()
        observed = failure(SHORT_SEED, full=True)
        verify_calls, feedback_calls, repair_calls = [], [], []
        oversized = [{"action": 0, "atoms": [0]*21}]

        def verify(text, trial, kind):
            verify_calls.append((text, kind))
            return certified if text == render(REPAIRED) else failed

        def feedback(text, replay, trial, kind):
            feedback_calls.append((text, replay))
            self.assertEqual(text, render(SHORT_SEED))
            self.assertIs(replay, failed)
            return observed

        def repair(seed, atoms, replay, check):
            repair_calls.append((seed, replay))
            self.assertEqual(seed, SHORT_SEED)
            self.assertEqual(atoms, ATOMS)
            self.assertIs(replay, observed)
            self.assertEqual(replay["actual"][1]["macro_line"], 2)
            self.assertTrue(all(len(row["truth"]) == len(ATOMS) for row in replay["rows"]))
            return [("repair_over_incumbent", oversized), ("repair_clock_gate", copy.deepcopy(REPAIRED))]

        (best, replay, records, summary), accepted, _, _ = self.run_driver(
            [SHORT_SEED], verify, repair, feedback=feedback)
        self.assertEqual(best, REPAIRED)
        self.assertIs(replay, certified)
        self.assertEqual(len(feedback_calls), 1)
        self.assertEqual(len(repair_calls), 1)
        self.assertEqual([kind for _, kind in verify_calls if not kind.startswith("beam_")],
            ["priority_short_seed", "repair_clock_gate"])
        self.assertNotIn(render(oversized), [text for text, _ in verify_calls])
        self.assertEqual(summary["repair_trials"], 1)
        self.assertEqual(summary["repair_expansions"], 1)
        self.assertEqual(summary["accepted_count"], 1)
        self.assertEqual(len(accepted), 1)
        self.assertTrue(compress.certified(replay))
        self.assertEqual([record["accepted"] for record in records
            if not record["kind"].startswith("beam_")], [False, True])

    def test_incomplete_or_truncated_repair_keeps_original_certified_best(self):
        for rejected in (passed(complete=False), passed(truncated=True), passed(status="probe_budget")):
            with self.subTest(complete=rejected["comparison"]["completed_full_replay"],
                    truncated=rejected["truncated"], status=rejected["status"]):
                failed = failure(SHORT_SEED, full=True)
                calls = []

                def verify(text, trial, kind):
                    calls.append(kind)
                    return rejected if kind.startswith("repair_") else failed

                def repair(seed, atoms, replay, check):
                    self.assertEqual(seed, SHORT_SEED)
                    self.assertIs(replay, failed)
                    return [("repair_clock_gate", copy.deepcopy(REPAIRED))]

                (best, replay, records, summary), accepted, _, baseline = self.run_driver(
                    [SHORT_SEED], verify, repair)
                self.assertIn("repair_clock_gate", calls)
                self.assertEqual(summary["repair_trials"], 1)
                self.assertEqual(best, INCUMBENT)
                self.assertIs(replay, baseline)
                self.assertTrue(compress.certified(replay))
                self.assertEqual(accepted, [])
                self.assertTrue(all(not record["accepted"] for record in records))

    def test_thin_failure_without_feedback_cannot_invent_repair_columns(self):
        failed = failure(SHORT_SEED)
        repair_calls = []
        (best, replay, _, summary), accepted, _, baseline = self.run_driver([SHORT_SEED],
            lambda *args: failed, lambda *args: repair_calls.append(args) or [])
        self.assertEqual(repair_calls, [])
        self.assertEqual(summary["repair_expansions"], 0)
        self.assertEqual(best, INCUMBENT)
        self.assertIs(replay, baseline)
        self.assertEqual(accepted, [])

    def test_nonmatching_skill_with_short_prefix_does_not_enter_blind_repair(self):
        failed = failure(SHORT_SEED, full=True, same_skill=False)
        calls = []
        (best, replay, _, summary), _, _, baseline = self.run_driver([SHORT_SEED],
            lambda *args: failed, lambda *args: calls.append("repair") or [],
            feedback=lambda *args: calls.append("feedback") or failed)
        self.assertEqual(calls, [])
        self.assertEqual(summary["repair_expansions"], 0)
        self.assertEqual(best, INCUMBENT)
        self.assertIs(replay, baseline)

    def test_failed_seeds_keep_their_own_macro_lines_and_full_paths(self):
        second_seed = [{"action": 0, "atoms": [2]}, {"action": 0, "atoms": [1]},
                       {"action": 0, "atoms": []}]
        seeds = [SHORT_SEED, second_seed]
        # Thin observations, fingerprints and comparisons intentionally match.
        # Missing earlier states forbid sharing their full observations.
        thin = {render(seed): failure(seed) for seed in seeds}
        observed = {render(SHORT_SEED): failure(SHORT_SEED, full=True, values=[True, False, True]),
                    render(second_seed): failure(second_seed, full=True, values=[False, True, False])}
        feedback_calls, repair_calls = [], []

        def feedback(text, replay, trial, kind):
            feedback_calls.append(text)
            self.assertIs(replay, thin[text])
            return observed[text]

        def repair(seed, atoms, replay, check):
            text = render(seed)
            repair_calls.append(text)
            self.assertNotEqual(seed, INCUMBENT)
            self.assertIs(replay, observed[text])
            self.assertEqual(replay["actual"][1]["macro_line"], len(seed))
            self.assertEqual(len(replay["rows"]), 2)
            self.assertEqual(replay["rows"][-1]["truth"], observed[text]["rows"][-1]["truth"])
            return []

        (best, replay, _, summary), accepted, _, baseline = self.run_driver(seeds,
            lambda text, *args: thin[text], repair, feedback=feedback)
        self.assertCountEqual(feedback_calls, [render(seed) for seed in seeds])
        self.assertCountEqual(repair_calls, [render(seed) for seed in seeds])
        self.assertEqual(summary["repair_expansions"], 2)
        self.assertEqual(best, INCUMBENT)
        self.assertIs(replay, baseline)
        self.assertEqual(accepted, [])

    def test_complete_thin_states_cannot_share_another_seeds_macro_line(self):
        second_seed = [{"action": 0, "atoms": [2]}, {"action": 0, "atoms": [1]},
                       {"action": 0, "atoms": []}]
        seeds = [SHORT_SEED, second_seed]
        thin = {render(seed): failure(seed) for seed in seeds}
        # Equal reached snapshots can come from different selected macro lines.
        # Reusing one complete replay would attach the wrong source line to the
        # second seed even when its states and eventual cast fingerprint match.
        for replay in thin.values():
            for observed_row in replay["rows"]:
                observed_row["state"] = {"rage": 17}
        observed = {render(seed): failure(seed, full=True) for seed in seeds}
        feedback_calls, repair_calls = [], []

        def feedback(text, replay, trial, kind):
            feedback_calls.append(text)
            return observed[text]

        def repair(seed, atoms, replay, check):
            text = render(seed)
            repair_calls.append(text)
            self.assertIs(replay, observed[text])
            self.assertEqual(replay["actual"][1]["macro_line"], len(seed))
            return []

        (best, replay, _, _), accepted, _, baseline = self.run_driver(seeds,
            lambda text, *args: thin[text], repair, feedback=feedback)
        self.assertCountEqual(feedback_calls, [render(seed) for seed in seeds])
        self.assertCountEqual(repair_calls, [render(seed) for seed in seeds])
        self.assertEqual(best, INCUMBENT)
        self.assertIs(replay, baseline)
        self.assertEqual(accepted, [])

    def test_uncertified_equal_cost_timing_alternate_cannot_start_window_search(self):
        atoms = ["bufftime:Source<9.8", "bufftime:Target<9.8"]
        source = [{"action": 0, "atoms": [0]}]
        alternate = [{"action": 0, "atoms": [1]}]
        baseline = dict(status="ok", truncated=False,
            rows=[dict(truth=[True, False], executable=[True], allowed=[0], cursor=0,
                time=0.0, wait_allowed=False, state=None)],
            comparison=dict(reproduced=True, completed_full_replay=True,
                acceptance_prefix=1, target_count=1, actual_count=1))

        def local_render(rules):
            return "/cast ["+conditions.condition_text(rules[0], atoms)+"] 盾刀"

        self.assertEqual(compress.char_count(local_render(source)), compress.char_count(local_render(alternate)))
        for reproduced, completed in ((False, True), (True, False)):
            with self.subTest(reproduced=reproduced, completed=completed):
                window_calls, verified, accepted = [], [], []
                rejected = dict(status="ok", truncated=False, rows=[],
                    comparison=dict(reproduced=reproduced, completed_full_replay=completed))
                empty = {name: (lambda *args, **kwargs: []) for name in (
                    "basic_batch", "feature_batch", "simple_edits", "feature_edits", "or_edits",
                    "family_edits", "joint_edits", "priority_edits", "repair_edits",
                    "local_rewrites", "global_edits", "event_window_edits")}

                def windows(rules, samples):
                    window_calls.append(copy.deepcopy(rules))
                    self.assertEqual(rules, source, "an uncertified alternate cannot supply window samples")
                    return []

                def verify(text, trial, kind):
                    verified.append((text, kind))
                    return rejected

                empty["window_edits"] = windows
                empty["timing_edits"] = lambda *args: [("beam_timing_alias", copy.deepcopy(alternate))]
                with patch.multiple(compress, **empty):
                    best, replay, records, summary = compress.compress(source, atoms, ACTIONS,
                        baseline, local_render, verify, lambda: None,
                        lambda solver: self.fail("no solver is needed"),
                        lambda *args: accepted.append(args), lambda summary: None, lambda *args: None)
                self.assertEqual(window_calls, [source])  # The ordinary incumbent stage still runs.
                self.assertEqual(verified, [(local_render(alternate), "beam_timing_alias")])
                self.assertEqual(best, source)
                self.assertIs(replay, baseline)
                self.assertEqual(summary["timing_trials"], 1)
                self.assertEqual(accepted, [])
                self.assertFalse(records[0]["accepted"])


if __name__ == "__main__":
    unittest.main()
