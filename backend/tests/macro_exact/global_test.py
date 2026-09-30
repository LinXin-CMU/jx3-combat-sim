"""Whole-program candidate contracts, independent of any reference macro."""
import importlib.util
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[3]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


compress = load('global_test_compress', ROOT/'tools/exact_macro_compress.py')
whole = load('global_test_search', ROOT/'tools/exact_macro_global.py')


def row(truth, executable, allowed, cursor, wait_allowed=False):
    return dict(truth=truth, executable=executable, allowed=allowed,
                cursor=cursor, wait_allowed=wait_allowed or not allowed)


def actions(*names):
    return [{'name':name,'fcast':False} for name in names]


class WholeProgramContracts(unittest.TestCase):
    def candidates(self, source, samples):
        records = []
        candidates = list(whole.global_edits(source, samples, compress.clone,
            lambda solver:self.fail('bitset search must not issue solver checks'),
            lambda info,smt:records.append(info)))
        bound = sum(samples.rule_cost(rule)+1 for rule in source)
        for kind, candidate in candidates:
            guide = whole._WitnessSamples(samples) if kind.startswith('global_witness_peeling') else samples
            self.assertTrue(guide.compatible(candidate))
            self.assertLess(sum(samples.rule_cost(rule)+1 for rule in candidate), bound)
        self.assertNotIn('unsat', [record.get('status') for record in records])
        return candidates, records

    def test_whole_macro_can_replace_guards_and_reorder_jointly(self):
        atoms = ['buff:First','buff:Redundant','buff:Second','buff:AlsoRedundant']
        rows = [row([1,1,0,0],[1,1],[0],0), row([0,0,1,1],[1,1],[1],1),
                row([0,0,0,0],[1,1],[],2)]
        samples = compress.Samples(rows, atoms, actions('盾刀','绝刀'), lambda:None)
        source = [{'action':1,'atoms':[2,3]}, {'action':0,'atoms':[0,1]}]
        candidates, records = self.candidates(source, samples)
        self.assertTrue(candidates)
        self.assertTrue(any(all(len(rule['atoms']) == 1 for rule in candidate)
                            for _,candidate in candidates))
        self.assertTrue(any(candidate[0]['action'] == 0 for _,candidate in candidates))
        self.assertEqual(records[-1]['initial_chars'], sum(samples.rule_cost(rule)+1 for rule in source)-1)

    def test_new_action_can_replace_original_action_when_sample_allows_it(self):
        defs = [{'name':'盾刀','fcast':True},{'name':'盾刀','fcast':False}]
        samples = compress.Samples([row([1],[1,1],[0,1],0),row([0],[1,1],[],1)],
                                  ['buff:X'], defs, lambda:None)
        source = [{'action':0,'atoms':[0]}]
        candidates, _ = self.candidates(source, samples)
        self.assertTrue(any(candidate[0]['action'] == 1 for _,candidate in candidates))

    def test_wait_rows_and_cursor_witnesses_are_preserved(self):
        atoms = ['buff:X','buff:Y']
        samples = compress.Samples([row([0,1],[1],[],0), row([1,1],[1],[0],0,True),
                                   row([0,0],[1],[],1)], atoms, actions('盾刀'), lambda:None)
        source = [{'action':0,'atoms':[0,1]}]
        candidates, _ = self.candidates(source, samples)
        self.assertTrue(candidates)
        for _,candidate in candidates:
            hits, remaining = samples.selections(candidate)
            self.assertFalse(hits[0] & 1)  # Never cast at the preceding WAIT.
            self.assertTrue(any(hit & 2 for hit in hits))  # Cursor witness survives.
            self.assertTrue(remaining & 4)  # End WAIT remains a WAIT.

    def test_unavailable_actions_do_not_block_lower_priority_rules(self):
        atoms = ['buff:X','buff:Y']
        samples = compress.Samples([row([1,1],[1,1],[0],0), row([1,1],[0,1],[1],1)],
                                  atoms, actions('盾刀','绝刀'), lambda:None)
        source = [{'action':0,'atoms':[0,1]}, {'action':1,'atoms':[0,1]}]
        candidates, _ = self.candidates(source, samples)
        self.assertTrue(candidates)
        self.assertTrue(any(candidate == [{'action':0,'atoms':[]},{'action':1,'atoms':[]}]
                            for _,candidate in candidates))

    def test_chain_and_optional_or_inputs_are_not_mutated(self):
        atoms = ['buff:A','buff:B','buff:C','buff:Extra']
        rows = [row([1,0,0,1],[1,1],[0],0), row([0,1,1,1],[1,1],[0],1),
                row([0,1,0,1],[1,1],[1],2)]
        samples = compress.Samples(rows, atoms, actions('盾刀','绝刀'), lambda:None)
        source = [{'action':0,'atoms':[0,1,2],'ops':['|','&']},
                  {'action':1,'atoms':[3],'any_atoms':[0,1]}]
        before = compress.clone(source)
        candidates, _ = self.candidates(source, samples)
        self.assertEqual(source, before)
        self.assertTrue(candidates)
        # An explicit AND chain and an old AND rule have sortable identities.
        self.assertNotEqual(whole._key({'action':0,'atoms':[0]}),
                            whole._key({'action':0,'atoms':[0],'ops':[]}))

    def test_new_branch_can_rebuild_a_seed_that_only_supplies_cost_bound(self):
        samples = compress.Samples([row([],[1,1],[0],0)], [], actions('盾刀','另一条轨迹上的昂贵技能'), lambda:None)
        source = [{'action':1,'atoms':[]}]
        self.assertFalse(samples.compatible(source))
        candidates, records = self.candidates(source, samples)
        self.assertTrue(candidates)
        self.assertEqual(candidates[0][1], [{'action':0,'atoms':[]}])
        self.assertFalse(records[-1]['seed_sample_compatible'])

    def test_pause_or_cancel_check_is_not_swallowed(self):
        samples = compress.Samples([row([1],[1],[0],0)], ['buff:X'], actions('盾刀'), lambda:None)
        def cancelled():
            raise RuntimeError('test cancellation')
        samples.check = cancelled
        with self.assertRaisesRegex(RuntimeError, 'test cancellation'):
            list(whole.global_edits([{'action':0,'atoms':[0]}], samples,
                                   compress.clone, lambda solver:None, lambda *value:None))

    def test_zero_observations_does_not_invent_an_empty_solution(self):
        samples = compress.Samples([], [], actions('盾刀'), lambda:None)
        candidates, records = self.candidates([{'action':0,'atoms':[]}], samples)
        self.assertEqual(candidates, [])
        self.assertEqual(records[-1]['status'], 'scope_exhausted')

    def test_success_guide_is_separate_and_does_not_claim_wait_compatibility(self):
        samples = compress.Samples([row([1,0],[1],[],0), row([0,1],[1],[],0),
                                   row([1,1],[1],[0],0,True)],
                                  ['buff:X','buff:Y'], actions('盾刀'), lambda:None)
        source = [{'action':0,'atoms':[0,1]}]
        before = samples.all, list(samples.truth), list(samples.allowed), dict(samples.groups)
        candidates, records = self.candidates(source, samples)
        self.assertTrue(candidates)
        self.assertTrue(all(kind.startswith('global_witness_peeling') for kind,_ in candidates))
        self.assertTrue(any(not samples.compatible(candidate) for _,candidate in candidates))
        self.assertEqual((samples.all, samples.truth, samples.allowed, samples.groups), before)
        self.assertEqual(records[-1]['guide_kind'], 'success_witness_projection')

    def test_projected_branch_keeps_its_reached_wait_counterexamples(self):
        samples = compress.Samples([row([1,0],[1],[],0), row([0,1],[1],[],0),
                                   row([1,1],[1],[0],0,True)],
                                  ['buff:X','buff:Y'], actions('盾刀'), lambda:None)
        samples.global_keep_mask = 0b001
        projected = whole._WitnessSamples(samples)
        self.assertEqual(projected.all, 0b101)
        self.assertFalse(projected.compatible([{'action':0,'atoms':[0]}]))
        self.assertTrue(projected.compatible([{'action':0,'atoms':[1]}]))

    def test_clock_carriers_are_an_explicit_experiment(self):
        atoms = ['rage<0','bufftime:Clock>1',
                 'buff:VeryLongRedundantStateNameXXXXXXXXXXXXXXXX','buff:X']
        samples = compress.Samples([row([0,1,1,1],[1],[0],0),
                                   row([0,0,0,0],[1],[],1)],
                                  atoms, actions('盾刀'), lambda:None)
        source = [{'action':0,'atoms':[1,2,3]}]
        plain, records = self.candidates(source, samples)
        self.assertTrue(plain)
        self.assertFalse(any('preserve_clocks' in kind for kind,_ in plain))
        self.assertEqual(records[-1]['experimental_clock_carriers'], 0)
        samples.global_clock_carriers = True
        timed, _ = self.candidates(source, samples)
        carriers = [program[-1] for kind,program in timed if 'preserve_clocks' in kind]
        self.assertTrue(carriers)
        self.assertTrue(all(samples.hit(carrier) == 0 for carrier in carriers))
        self.assertTrue(all(set(carrier['atoms']) == {0,1} for carrier in carriers))

    def test_branch_separators_survive_a_dense_primitive_library(self):
        atoms = ['buff:A','buff:B','nobuff:A','nobuff:B',
                 'buff:'+'VeryLongSourceCondition'*8]
        rows = [row([0,0,1,1,1],[1],[0],0), row([1,1,0,0,1],[1],[0],1),
                row([0,1,1,0,0],[1],[],2), row([1,0,0,1,0],[1],[],2)]
        samples = compress.Samples(rows, atoms, actions('盾刀'), lambda:None)
        samples.global_keep_mask = 0b1111
        source = [{'action':0,'atoms':[4]}]
        candidates = list(whole._search(source, samples, compress.clone,
                                       lambda *values:None, column_limit=1))
        self.assertTrue(candidates)
        self.assertTrue(all(samples.compatible(program) for _,program in candidates))
        self.assertTrue(any(len(program) == 2 and
                            all(4 not in rule['atoms'] for rule in program)
                            for _,program in candidates))


if __name__ == '__main__':
    unittest.main()
