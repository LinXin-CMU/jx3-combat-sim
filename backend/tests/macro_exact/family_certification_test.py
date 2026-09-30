"""Family proposals must pass the common native certification acceptance gate.

Synthetic snapshots deliberately hide a wake-up threshold's runtime role. A
shorter, statically compatible 3-to-2 proposal may never replace the complete
baseline when replay timing fails or terminal replay is unfinished.
"""
from contextlib import ExitStack
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location("family_certification_compress",
    ROOT/"tools/exact_macro_compress.py")
compress = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(compress)


def render(rules, atoms, actions):
    lines = []
    for rule in rules:
        action = actions[rule["action"]]
        guard = "&".join(atoms[k] for k in rule["atoms"])
        lines.append("/cast " + ("["+guard+"] " if guard else "") + action["name"])
    return "\n".join(lines)


class FamilyCertificationContracts(unittest.TestCase):
    def run_rejected_family(self, reproduced, completed_full_replay):
        atoms = ["bufftime:Wake<3.0", "buff:A", "buff:B", "buff:C", "buff:AB"]
        actions = [{"name": "盾刀", "fcast": False}]
        rows = [dict(truth=truth, executable=[1], allowed=allowed, cursor=n,
            wait_allowed=not allowed) for n, (truth, allowed) in enumerate([
                ([1, 1, 0, 0, 1], [0]), ([1, 0, 1, 0, 1], [0]),
                ([1, 0, 0, 1, 0], [0]), ([0, 0, 0, 0, 0], [])])]
        rules = [{"action": 0, "atoms": [0, k]} for k in (1, 2, 3)]
        candidate = [{"action": 0, "atoms": [4]}, {"action": 0, "atoms": [3]}]
        baseline = dict(status="ok", rows=rows, truncated=False,
            comparison=dict(reproduced=True, completed_full_replay=True))
        original = compress.clone(rules)
        calls, accepted = [], []
        columns = compress.Samples(rows, atoms, actions, lambda: None)
        self.assertTrue(columns.compatible(rules))
        self.assertTrue(columns.compatible(candidate))
        self.assertLess(compress.char_count(render(candidate, atoms, actions)),
            compress.char_count(render(rules, atoms, actions)))

        def family(source, samples, diagnostic):
            self.assertEqual(source, original)
            self.assertTrue(samples.compatible(candidate))
            yield "family_rebuild_3_to_2", compress.clone(candidate)

        def verify(text, trial, kind):
            calls.append((text, trial, kind))
            return dict(status="ok", rows=rows, truncated=False,
                comparison=dict(reproduced=reproduced,
                    completed_full_replay=completed_full_replay,
                    first_difference=None if reproduced else {"index": 1, "kind": "cast_mismatch"}))

        with ExitStack() as mocks:
            mocks.enter_context(patch.object(compress, "family_edits", family))
            for name in ("simple_edits", "feature_edits", "or_edits", "priority_edits",
                    "local_rewrites", "global_edits", "basic_batch", "feature_batch"):
                mocks.enter_context(patch.object(compress, name, return_value=[]))
            best, replay, records, summary = compress.compress(rules, atoms, actions,
                baseline, lambda value: render(value, atoms, actions), verify,
                lambda: None, lambda solver: solver.check(),
                lambda *values: accepted.append(values), lambda summary: None,
                lambda info, smt: None)

        family_calls = [call for call in calls if call[2].startswith("family_")]
        family_records = [record for record in records if record["kind"].startswith("family_")]
        self.assertEqual(len(family_calls), 1)
        self.assertEqual(family_calls[0][0], render(candidate, atoms, actions))
        self.assertEqual(summary["family_trials"], 1)
        self.assertEqual(len(family_records), 1)
        self.assertFalse(family_records[0]["accepted"])
        self.assertEqual(best, original)
        self.assertEqual(rules, original)
        self.assertIs(replay, baseline)
        self.assertTrue(compress.certified(replay))
        self.assertEqual(summary["saved_chars"], 0)
        self.assertEqual(accepted, [])

    def test_lost_wake_timing_retains_certified_baseline(self):
        self.run_rejected_family(reproduced=False, completed_full_replay=True)

    def test_incomplete_terminal_replay_retains_certified_baseline(self):
        self.run_rejected_family(reproduced=True, completed_full_replay=False)


if __name__ == "__main__":
    unittest.main()
