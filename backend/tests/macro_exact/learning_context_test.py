"""Observed-window features must not invent failures after a legal witness."""
import importlib.util
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location('learning_context_compress', ROOT/'tools/exact_macro_compress.py')
COMPRESS = importlib.util.module_from_spec(spec)
spec.loader.exec_module(COMPRESS)


class LearningContextTests(unittest.TestCase):
    def samples(self, rows):
        return COMPRESS.Samples(rows, ['rage<20', 'rage>10'],
            [{'name':'one'}, {'name':'two'}], lambda:None)

    def row(self, cursor, truth, allowed, wait=False, executable=(True, True)):
        return dict(cursor=cursor, time=cursor, truth=truth, executable=executable,
                    allowed=allowed, wait_allowed=wait)

    def test_legal_earlier_witness_ignores_old_tail_snapshots(self):
        samples = self.samples([self.row(0,[True,False],[0],True),
                                self.row(0,[True,True],[1])])
        context = samples.learning_context([dict(action=0,atoms=[0])])
        self.assertTrue(context['static_compatible'])
        self.assertEqual(1, context['window_coverage'])
        self.assertEqual(0, context['forbidden_hits'])
        self.assertEqual(samples.window_compatible([dict(action=0,atoms=[0])]),
                         context['static_compatible'])

    def test_early_cast_before_window_is_wait_conflict(self):
        samples = self.samples([self.row(0,[True,False],[],True),
                                self.row(0,[True,True],[0])])
        context = samples.learning_context([dict(action=0,atoms=[0])])
        self.assertFalse(context['static_compatible'])
        self.assertEqual(0, context['window_coverage'])
        self.assertEqual(1, context['wait_conflicts'])
        self.assertEqual(1, context['forbidden_hits'])

    def test_wrong_rule_steals_later_correct_hit(self):
        samples = self.samples([self.row(0,[True,True],[1])])
        rules = [dict(action=0,atoms=[0]),dict(action=1,atoms=[1])]
        context = samples.learning_context(rules)
        self.assertEqual(1,context['priority_conflicts'])
        self.assertEqual(1,context['forbidden_hits'])
        correct = samples.learning_context(list(reversed(rules)))
        self.assertTrue(correct['static_compatible'])
        self.assertEqual(0,correct['priority_conflicts'])

    def test_unavailable_wrong_action_is_not_a_failure(self):
        samples = self.samples([self.row(0,[True,True],[1],executable=(False,True))])
        context = samples.learning_context([dict(action=0,atoms=[0]),dict(action=1,atoms=[1])])
        self.assertTrue(context['static_compatible'])
        self.assertEqual(0,context['forbidden_hits'])

    def test_missing_event_and_terminal_extra_are_separate_failures(self):
        samples = self.samples([self.row(0,[False,True],[0]),self.row(1,[True,False],[],True)])
        missing = samples.learning_context([])
        self.assertFalse(missing['static_compatible'])
        self.assertEqual(0,missing['forbidden_hits'])
        extra = samples.learning_context([dict(action=0,atoms=[0])])
        self.assertFalse(extra['static_compatible'])
        self.assertEqual(1,extra['forbidden_hits'])


if __name__ == '__main__':
    unittest.main()
