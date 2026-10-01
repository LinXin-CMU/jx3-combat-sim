"""Separate reached-branch guidance and bounded feedback contracts."""
import copy
import importlib.util
import json
from pathlib import Path
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[3]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


compress = load('branch_test_compress', ROOT/'tools/exact_macro_compress.py')
whole = load('branch_test_global', ROOT/'tools/exact_macro_global.py')


def row(truth, allowed, cursor, time=0.0, executable=None, wait_allowed=True, state=None):
    return dict(truth=truth, executable=executable or [True], allowed=allowed,
                cursor=cursor, time=time, wait_allowed=wait_allowed, state=state,
                last_skill=None, decision_latest=0.0625, wait_next_time=None)


def failure(rows, cursor=0, fingerprint='failed-path'):
    return dict(status='ok', truncated=False, rows=rows,
                probe_failure=dict(index=cursor, kind='missed_decision_time'),
                actual_fingerprint=fingerprint,
                comparison=dict(reproduced=False, completed_full_replay=False,
                                first_difference=dict(index=cursor)))


def source(rows, atoms=None, actions=None):
    return compress.Samples(rows, atoms or ['buff:X', 'buff:Y'],
                            actions or [dict(name='盾刀', fcast=False)], lambda:None)


class BranchContracts(unittest.TestCase):
    def test_only_reached_prefix_is_used_without_teacher_suffix(self):
        original = source([row([1, 0], [0], 0), row([0, 0], [], 1),
                           row([0, 1], [0], 1), row([1, 1], [0], 2)])
        reached = failure([row([0, 0], [], 0), row([1, 0], [0], 0),
                           row([1, 0], [], 1), row([1, 1], [0], 2)], cursor=1)
        before = copy.deepcopy((original.rows, reached))
        cursor, _, guide = compress.branch_guide(reached, original)
        self.assertEqual(cursor, 1)
        self.assertEqual(guide.rows, reached['rows'][:3])
        self.assertEqual(guide.global_keep_mask, 0b111)
        self.assertNotIn('suffix', guide.guide_kind)
        self.assertEqual((original.rows, reached), before)

    def test_success_projection_keeps_reached_waits_without_importing_source_rows(self):
        original = source([row([0, 0], [], 0), row([1, 1], [0], 0),
                           row([0, 0], [], 1), row([1, 1], [0], 1)])
        reached = failure([row([1, 0], [], 0), row([0, 1], [0], 0)])
        _, _, guide = compress.branch_guide(reached, original)
        projected = whole._WitnessSamples(guide)
        self.assertTrue(projected.all & 1)  # Actual reached WAIT is protected.
        self.assertTrue(projected.all & 2)  # Reached success window remains.
        self.assertEqual(guide.rows, reached['rows'])  # No source window is imported.
        self.assertFalse(projected.compatible([{'action':0, 'atoms':[0]}]))
        self.assertTrue(projected.compatible([{'action':0, 'atoms':[1]}]))

    def test_mutually_exclusive_failed_prefixes_remain_separate(self):
        defs = [dict(name='盾刀', fcast=False), dict(name='盾刀', fcast=True)]
        original = source([row([0, 1], [0, 1], 0, executable=[True, True])], actions=defs)
        first = failure([row([1, 0], [0], 0, executable=[True, True])])
        second = failure([row([1, 0], [1], 0, executable=[True, True])])
        _, first_signature, first_guide = compress.branch_guide(first, original)
        _, second_signature, second_guide = compress.branch_guide(second, original)
        first_program = [{'action':0, 'atoms':[]}]
        second_program = [{'action':1, 'atoms':[0]}, {'action':0, 'atoms':[1]}]
        self.assertNotEqual(first_signature, second_signature)
        self.assertTrue(first_guide.compatible(first_program))
        self.assertFalse(second_guide.compatible(first_program))
        self.assertFalse(first_guide.compatible(second_program))
        self.assertTrue(second_guide.compatible(second_program))
        self.assertEqual(first_guide.rows[0]['allowed'], [0])
        self.assertEqual(second_guide.rows[0]['allowed'], [1])
        self.assertEqual(len(original.rows), 1)

    def test_missing_full_columns_or_an_unaligned_result_is_not_a_branch(self):
        original = source([row([1, 1], [0], 0)])
        self.assertIsNone(compress.branch_guide(None, original))
        self.assertIsNone(compress.branch_guide(failure([row([1], [0], 0)]), original))
        no_failure = failure([row([1, 1], [0], 0)])
        no_failure['probe_failure'] = None
        self.assertIsNone(compress.branch_guide(no_failure, original))
        later_only = failure([row([1, 1], [0], 1)])
        self.assertIsNone(compress.branch_guide(later_only, original))
        terminal = failure([row([1, 1], [], 1)], cursor=1)
        # A reached terminal WAIT is a useful negative-only counterexample,
        # even when the source trajectory has no success window after it.
        cursor, _, guide = compress.branch_guide(terminal, original)
        self.assertEqual(cursor, 1)
        self.assertEqual(guide.rows, terminal['rows'])
        self.assertEqual(guide.groups, {})
        self.assertEqual(guide.required, 0)
        self.assertEqual(guide.global_keep_mask, 1)
        self.assertTrue(guide.compatible([]))
        self.assertFalse(guide.compatible([{'action':0, 'atoms':[]}]))

    def test_known_branch_is_not_rebuilt(self):
        original = source([row([1, 1], [0], 0)])
        reached = failure([row([1, 1], [0], 0)])
        _, signature, _ = compress.branch_guide(reached, original)
        def should_not_construct():
            self.fail('known branch must be skipped before reconstructing bitsets')
        original.check = should_not_construct
        self.assertIsNone(compress.branch_guide(reached, original, {signature}))

    def run_feedback_fixture(self, attempts, thin_factory, full_factory, stage='global'):
        atoms = ['buff:LongCondition']
        actions = [dict(name='盾刀', fcast=False)]
        rules = [{'action':0, 'atoms':[0]*30}]
        baseline_rows = [row([1], [0], 0, state=dict(rage=10))]
        baseline = dict(status='ok', rows=baseline_rows, truncated=False,
                        comparison=dict(reproduced=True, completed_full_replay=True))
        calls, feedback_calls, diagnostics = [], [], []
        def global_candidates(*args, **kwargs):
            for index in range(attempts):
                yield stage + '_rule_peeling', [{'action':0, 'atoms':[0]*(len(calls)+1)}]
        def verify(text, trial, kind):
            calls.append(trial)
            return thin_factory(trial)
        def feedback(text, result, trial, kind):
            feedback_calls.append(trial)
            return full_factory(trial)
        empty = dict(basic_batch=lambda *args:[], feature_batch=lambda *args:[],
                     simple_edits=lambda *args:[], feature_edits=lambda *args:[],
                     or_edits=lambda *args:[], priority_edits=lambda *args, **kwargs:[],
                     local_rewrites=lambda *args:[], family_edits=lambda *args:[],
                     joint_edits=lambda *args, **kwargs:[], global_edits=lambda *args:[])
        empty[stage + '_edits'] = global_candidates
        with patch.multiple(compress, **empty):
            best, replay, records, summary = compress.compress(
                rules, atoms, actions, baseline, lambda value:json.dumps(value), verify,
                lambda:None, lambda solver:self.fail('no solver in this fixture'),
                lambda *args:self.fail('a rejected program cannot be accepted'),
                lambda *args:None, lambda value, smt:diagnostics.append(value),
                feedback=feedback, deep_search=(stage == 'global'), joint_search=(stage == 'joint'))
        self.assertEqual(best, rules)
        self.assertIs(replay, baseline)
        self.assertTrue(all(not record['accepted'] for record in records))
        return calls, feedback_calls, diagnostics, summary

    def test_twelve_branch_budget_stops_with_scope_exhausted(self):
        def thin(trial):
            return failure([row([0], [], 0, time=trial/1000, state=dict(rage=10))])
        def full(trial):
            return failure([row([1], [], 0, time=trial/1000, state=dict(rage=10))])
        calls, _, diagnostics, summary = self.run_feedback_fixture(1, thin, full)
        self.assertEqual(len(calls), 13)  # Initial pass plus twelve branch searches.
        self.assertEqual(summary['global_branches'], 12)
        self.assertEqual(summary['status'], 'scope_exhausted')
        self.assertNotIn('unsat', [record.get('status') for record in diagnostics])

    def test_thin_paths_without_all_states_cannot_share_full_condition_columns(self):
        # Compact native output only exports its LAST row state. Two paths can
        # share all thin observations yet differ on an earlier legal predicate.
        def thin(trial):
            return failure([row([0], [], 0, state=None),
                            row([0], [], 0, time=0.01, state=dict(rage=10))],
                           fingerprint=f'candidate-{trial}')
        def full(trial):
            return failure([row([trial % 2], [], 0, state=None),
                            row([1], [], 0, time=0.01, state=dict(rage=10))],
                           fingerprint=f'candidate-{trial}')
        calls, feedback_calls, _, _ = self.run_feedback_fixture(2, thin, full)
        self.assertGreaterEqual(len(calls), 2)
        self.assertEqual(feedback_calls[:2], [1, 2])

    def test_joint_branches_keep_the_failed_seed_and_incumbent_bound(self):
        atoms = ['buff:LongCondition']
        actions = [dict(name='盾刀', fcast=False)]
        rules = [{'action':0, 'atoms':[0]*30}]
        candidate = [{'action':0, 'atoms':[0]}]
        baseline = dict(status='ok', rows=[row([1], [0], 0)], truncated=False,
                        comparison=dict(reproduced=True, completed_full_replay=True))
        render = json.dumps
        calls = []
        def joint(seed, guide, diagnostic, cost_bound=None):
            calls.append((copy.deepcopy(seed), cost_bound))
            if cost_bound is None:
                yield 'joint_region_2_to_1', candidate
        def verify(text, trial, kind):
            return failure([row([1], [], 0)])
        empty = {name:(lambda *args, **kwargs:[]) for name in ('basic_batch', 'feature_batch',
            'simple_edits', 'feature_edits', 'or_edits', 'priority_edits', 'family_edits',
            'local_rewrites', 'global_edits')}
        with patch.multiple(compress, joint_edits=joint, **empty):
            best, replay, _, summary = compress.compress(rules, atoms, actions, baseline,
                render, verify, lambda:None, lambda solver:solver.check(),
                lambda *args:self.fail('a failed candidate cannot replace the incumbent'),
                lambda *args:None, lambda *args:None,
                feedback=lambda text, result, trial, kind:result, joint_search=True)
        self.assertEqual(calls, [(rules,None), (candidate,compress.char_count(render(rules))+1)])
        self.assertEqual(best, rules)
        self.assertIs(replay, baseline)
        self.assertEqual(summary['joint_branches'], 1)

    def test_same_predecision_states_with_different_verdicts_are_separate_feedback(self):
        # The observed decision can select a wrong action, or cast correctly
        # and miss the following deadline. Identical incoming states alone do
        # not establish the comparison cursor used to cut the reached prefix.
        def thin(trial):
            return failure([row([0], [], 0, state=dict(rage=10))],
                           cursor=(trial-1) % 2, fingerprint=f'candidate-{trial}')
        def full(trial):
            return failure([row([1], [], 0, state=dict(rage=10))],
                           cursor=(trial-1) % 2, fingerprint=f'candidate-{trial}')
        calls, feedback_calls, _, _ = self.run_feedback_fixture(2, thin, full)
        self.assertGreaterEqual(len(calls), 2)
        self.assertEqual(feedback_calls[:2], [1, 2])


if __name__ == '__main__':
    unittest.main()
