"""Contextual leaf proposals preserve native chains and certification gates."""
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location('chain_compression_fixtures',
    Path(__file__).with_name('compression_test.py'))
FIX = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(FIX)
COMP = FIX.compress
COND = COMP.condition_module()


class ChainLeafTests(unittest.TestCase):
    def fixture(self):
        atoms = ['buff:OriginalLong', 'buff:B', 'buff:C', 'buff:X']
        actions = [dict(name='skill', fcast=False)]
        rows = [FIX.sample(values, [1], [0] if allowed else [], cursor)
                for cursor, (values, allowed) in enumerate([
                    ([1, 1, 0, 1], True), ([0, 1, 0, 0], False),
                    ([1, 0, 0, 0], False), ([1, 0, 1, 1], True),
                    ([0, 0, 1, 0], False)])]
        rules = [dict(action=0, atoms=[0, 1, 2], ops=['&', '|'])]
        samples = COMP.Samples(rows, atoms, actions, lambda: None)
        return atoms, actions, rows, rules, samples

    def test_mixed_chain_replaces_contextual_not_global_equivalent_leaf(self):
        atoms, actions, rows, rules, samples = self.fixture()
        self.assertNotEqual(samples.truth[0], samples.truth[3])
        proposals = list(COMP.chain_leaf_edits(rules, samples))
        replacement = dict(action=0, atoms=[3, 1, 2], ops=['&', '|'])
        self.assertIn(('replace_chain_leaf', [replacement]), proposals)
        for _, trial in proposals:
            text = FIX.native_render(trial, atoms, actions)
            self.assertTrue(FIX.native_matches(text, atoms, actions, rows))
            self.assertLess(COMP.char_count(text), COMP.char_count(FIX.native_render(rules, atoms, actions)))
        self.assertEqual(rules[0]['atoms'], [0, 1, 2])

    def test_legacy_or_tail_keeps_action_and_native_branching(self):
        atoms, actions, rows, rules, samples = self.fixture()
        legacy = [dict(action=0, atoms=[0], any_atoms=[1, 2])]
        proposals = list(COMP.chain_leaf_edits(legacy, samples))
        self.assertTrue(proposals)
        self.assertTrue(all(FIX.native_matches(FIX.native_render(trial, atoms, actions), atoms, actions, rows)
                            for _, trial in proposals))
        self.assertEqual(legacy[0]['any_atoms'], [1, 2])

    def test_prior_correct_actions_and_unavailable_actions_are_dont_cares(self):
        atoms, actions, rows, rules, _ = self.fixture()
        atoms += ['buff:Earlier']
        for row in rows:
            row['truth'].append(0)
            row['executable'].append(0)
        actions += [dict(name='earlier', fcast=False)]
        rows += [FIX.sample([1, 1, 0, 0, 1], [1, 1], [1], len(rows)),
                 FIX.sample([1, 1, 0, 0, 0], [0, 0], [], len(rows)+1)]
        rules.insert(0, dict(action=1, atoms=[4]))
        samples = COMP.Samples(rows, atoms, actions, lambda: None)
        trial = next(trial for _, trial in COMP.chain_leaf_edits(rules, samples)
                     if trial[1]['atoms'][0] == 3)
        self.assertTrue(FIX.native_matches(FIX.native_render(trial, atoms, actions), atoms, actions, rows))

    def test_distinct_clocks_with_same_sample_truth_remain_distinct(self):
        _, actions, rows, rules, _ = self.fixture()
        atoms = ['bufftime:OriginalLong<4.0', 'buff:B', 'buff:C', 'bufftime:X<4.0', 'bufftime:Y<3.0']
        for row in rows:
            row['truth'].append(row['truth'][3])
        samples = COMP.Samples(rows, atoms, actions, lambda: None)
        choices = {trial[0]['atoms'][0] for _, trial in COMP.chain_leaf_edits(rules, samples)}
        self.assertTrue({3, 4} <= choices)

    def test_false_replacement_cannot_steal_an_earlier_wait(self):
        _, actions, rows, rules, _ = self.fixture()
        atoms = ['buff:OriginalLong', 'buff:B', 'buff:C', 'buff:X']
        rows[1]['truth'][3] = 1
        samples = COMP.Samples(rows, atoms, actions, lambda: None)
        self.assertFalse(any(trial[0]['atoms'][0] == 3
                             for _, trial in COMP.chain_leaf_edits(rules, samples)))

    def test_static_proposal_is_not_a_native_certificate(self):
        atoms, actions, rows, rules, _ = self.fixture()
        certified_source = FIX.replay(rows)
        attempted, accepted = [], []
        def verify(text, trial, kind):
            attempted.append(kind)
            return FIX.replay(rows, False)
        best, result, _, _ = COMP.compress(rules, atoms, actions, certified_source,
            lambda proposal: FIX.native_render(proposal, atoms, actions), verify,
            lambda: None, lambda solver: solver.check(),
            lambda *args: accepted.append(args), lambda *args: None, lambda *args: None)
        self.assertIn('replace_chain_leaf', attempted)
        self.assertEqual(best, rules)
        self.assertIs(result, certified_source)
        self.assertFalse(accepted)

    def test_cancellation_interrupts_catalog_scan(self):
        _, _, _, rules, samples = self.fixture()
        def cancelled():
            raise InterruptedError('paused/stopped')
        samples.check = cancelled
        with self.assertRaises(InterruptedError):
            list(COMP.chain_leaf_edits(rules, samples))

    def bridge_fixture(self, reject_bridge=False):
        atoms = ['bufftime:T<2.0', 'buff:B', 'buff:C', 'skill_energy:S']
        self.assertEqual(len(atoms[0]), len(atoms[3]))
        actions = [dict(name='a', fcast=False), dict(name='b', fcast=False)]
        rows = [FIX.sample([1, 1, 0, 1], [1, 1], [0], 0),
                FIX.sample([1, 0, 0, 1], [1, 1], [1], 1),
                FIX.sample([1, 0, 1, 1], [1, 1], [0], 2)]
        alternative_rows = [dict(row, truth=list(row['truth'])) for row in rows]
        alternative_rows[1]['truth'][3] = 0
        source = [dict(action=0, atoms=[0, 1, 2], ops=['&', '|']),
                  dict(action=1, atoms=[])]
        proposals, calls, accepted = {}, [], []
        def render(rules):
            text = FIX.native_render(rules, atoms, actions)
            proposals[text] = rules
            return text
        def verify(text, n, kind):
            trial = proposals[text]
            leaf_ids = {atom for rule in trial for atom in rule['atoms']}
            # A new scheduling dependency changes the next observed state.
            # Removing both dependencies misses a required scan altogether.
            used_rows = alternative_rows if 3 in leaf_ids else rows
            passed = bool(leaf_ids & {0, 3}) and FIX.native_matches(text, atoms, actions, used_rows)
            result = FIX.replay(used_rows, passed)
            if reject_bridge and kind == 'bridge_chain_leaf':
                result['truncated'] = True
            calls.append((kind, COMP.certified(result)))
            return result
        best, result, _, _ = COMP.compress(source, atoms, actions, FIX.replay(rows),
            render, verify, lambda: None, lambda solver: solver.check(),
            lambda *args: accepted.append(args), lambda *args: None, lambda *args: None)
        return source, best, result, calls, accepted

    def test_equal_bridge_uses_its_own_certified_path_for_shorter_descendant(self):
        source, best, result, calls, accepted = self.bridge_fixture()
        self.assertIn(('bridge_chain_leaf', True), calls)
        self.assertTrue(any(kind.startswith('bridge_') and kind != 'bridge_chain_leaf' and passed
                            for kind, passed in calls))
        self.assertEqual(best[0]['atoms'], [3])
        self.assertTrue(COMP.certified(result))
        self.assertTrue(accepted)
        self.assertTrue(all(args[2]['saved_chars'] > 0 for args in accepted))

    def test_truncated_bridge_never_supplies_descendant_states(self):
        source, best, _, calls, accepted = self.bridge_fixture(reject_bridge=True)
        self.assertIn(('bridge_chain_leaf', False), calls)
        self.assertFalse(any(kind.startswith('bridge_') and kind != 'bridge_chain_leaf'
                             for kind, _ in calls))
        self.assertEqual(best, source)
        self.assertFalse(accepted)


if __name__ == '__main__':
    unittest.main()
