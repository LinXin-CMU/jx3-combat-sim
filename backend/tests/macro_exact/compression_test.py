"""Focused stage-two contracts: priority, grouped rewrites and certification."""
import importlib.util
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("compress", ROOT / "tools/exact_macro_compress.py")
compress = importlib.util.module_from_spec(spec)
spec.loader.exec_module(compress)


def sample(truth, executable, allowed, cursor):
    return dict(truth=truth, executable=executable, allowed=allowed, cursor=cursor, wait_allowed=not allowed)


def replay(rows, passed=True):
    return dict(status="ok", rows=rows, truncated=False,
                comparison=dict(reproduced=passed, completed_full_replay=passed))


def native_render(rules, atoms, actions):
    lines = []
    for rule in rules:
        if 'ops' in rule:
            guard = atoms[rule['atoms'][0]] + ''.join(op+atoms[k] for op,k in zip(rule['ops'],rule['atoms'][1:])) if rule['atoms'] else ''
        else:
            prefix = '&'.join(atoms[k] for k in rule['atoms'])
            suffix = '|'.join(atoms[k] for k in rule.get('any_atoms', []))
            guard = prefix + ('&' if prefix and suffix else '') + suffix
        action = actions[rule['action']]
        lines.append(('/fcast ' if action['fcast'] else '/cast ')
                     + ('['+guard+'] ' if guard else '') + action['name'])
    return '\n'.join(lines) or '/cast [rage<0] 盾刀'


def native_matches(text, atoms, actions, rows):
    """Independent right-associative text evaluation for these finite fixtures."""
    parsed = []
    for line in text.splitlines():
        command, rest = line.split(' ', 1)
        guard, name = rest[1:].split('] ', 1) if rest.startswith('[') else ('', rest)
        action = next(a for a,value in enumerate(actions)
                      if value['name'] == name and value['fcast'] == (command == '/fcast'))
        parsed.append((action, re.split(r'([&|])', guard) if guard else []))
    for row in rows:
        values = dict(zip(atoms, row['truth']))
        selected = None
        for action, parts in parsed:
            condition = bool(values[parts[-1]]) if parts else True
            for k in range(len(parts)-3, -1, -2):
                left = bool(values[parts[k]])
                condition = left and condition if parts[k+1] == '&' else left or condition
            if condition and row['executable'][action]:
                selected = action
                break
        if (selected is None and row['allowed']) or (selected is not None and selected not in row['allowed']):
            return False
    return True


class CompressionContracts(unittest.TestCase):
    def test_explicit_and_chains_can_merge_into_native_or_suffix(self):
        atoms = ['buff:A','buff:B','buff:C']
        actions = [dict(name='盾刀',fcast=False)]
        rows = [sample([1,1,0],[1],[0],0),sample([1,0,1],[1],[0],1),
                sample([1,0,0],[1],[],2)]
        rules = [dict(action=0,atoms=[0,1],ops=['&']),dict(action=0,atoms=[0,2],ops=['&'])]
        columns = compress.Samples(rows,atoms,actions,lambda:None)
        candidates = list(compress.or_edits(rules,columns))
        self.assertTrue(candidates)
        self.assertTrue(all(native_matches(native_render(trial,atoms,actions),atoms,actions,rows)
                            for _,trial in candidates))
        self.assertTrue(any(len(trial)==1 for _,trial in candidates))

    def batch_fixture(self, critical_clock=False):
        atoms = ['buff:X','bufftime:Wake<3.0']
        actions = [{'name':'盾刀','fcast':False},{'name':'绝刀','fcast':False}]
        rows = [sample([1,0],[1,1],[0],0),sample([0,0],[1,1],[1],1)]
        rules = [{'action':0,'atoms':[0]} for _ in range(40)] + [{'action':1,'atoms':[]}]
        if critical_clock:
            rules[1] = {'action':0,'atoms':[1]}
        programs,calls = {},[]
        def render(candidate):
            text = native_render(candidate, atoms, actions)
            programs[text] = compress.clone(candidate)
            return text
        columns = compress.Samples(rows,atoms,actions,lambda:None)
        def verify(text,n,kind):
            candidate = programs[text]
            passed = columns.compatible(candidate) and (not critical_clock or any(1 in r['atoms'] for r in candidate))
            calls.append((kind,passed))
            return replay(rows,passed)
        best,result,records,summary = compress.compress(rules,atoms,actions,replay(rows),render,verify,
            lambda:None,lambda solver:solver.check(),lambda *v:None,lambda _:None,lambda *v:None)
        return best,result,calls,records,summary

    def test_one_real_validation_can_remove_many_rows(self):
        best,result,calls,_,summary = self.batch_fixture()
        self.assertTrue(compress.certified(result))
        self.assertEqual(len(best),2)
        self.assertEqual(calls,[('batch_basic',True)])
        self.assertEqual(summary['max_batch_rules'],39)

    def test_failed_batch_splits_with_stable_ids_and_keeps_critical_wake_rule(self):
        best,result,calls,records,summary = self.batch_fixture(critical_clock=True)
        self.assertTrue(compress.certified(result))
        # Broader searches may fold the retained clock into another guard.
        # The contract is preservation and certified shortening, not 3 rows.
        self.assertLessEqual(len(best),3)
        self.assertTrue(any(1 in r['atoms'] for r in best))
        self.assertFalse(calls[0][1])
        self.assertTrue(any(passed for _,passed in calls))
        self.assertLess(len(calls),39)
        self.assertGreater(summary['max_batch_rules'],1)
        self.assertTrue(all(not r['accepted'] for r in records if not r['comparison']['reproduced']))

    def test_copyable_utf16_count_and_full_certificate_required(self):
        self.assertEqual(compress.char_count("/cast [buff:X] 盾刀\n😀"), 20)
        self.assertTrue(compress.certified(replay([])))
        self.assertFalse(compress.certified(dict(replay([]), truncated=True)))
        self.assertFalse(compress.certified(dict(replay([]), status="probe_budget")))
        self.assertFalse(compress.certified(dict(status="ok", comparison={"reproduced":True})))

    def test_unavailable_and_lower_priority_wrong_actions_are_legal(self):
        rows = [sample([], [1,1], [0], 0), sample([], [0,1], [1], 1)]
        columns = compress.Samples(rows, [], [{},{}], lambda:None)
        self.assertTrue(columns.compatible([{"action":0,"atoms":[]},{"action":1,"atoms":[]}]))
        self.assertFalse(columns.compatible([{"action":1,"atoms":[]},{"action":0,"atoms":[]}]))

    def test_group_can_replace_features_and_delete_rows_in_one_verified_edit(self):
        atoms = ["rage=10","rage=20","bufftime:X<3.0"]
        actions = [{"name":"盾刀","fcast":False},{"name":"绝刀","fcast":False}]
        rows = [sample([1,0,1],[1,1],[0],0),sample([0,1,1],[1,1],[0],1),sample([0,0,0],[1,1],[1],2)]
        rules = [{"action":0,"atoms":[0]},{"action":0,"atoms":[1]},{"action":1,"atoms":[]}]
        programs, tested, accepted, diagnostics = {},[],[],[]
        def render(candidate):
            text = native_render(candidate, atoms, actions)
            programs[text] = compress.clone(candidate)
            return text or "/cast [rage<0] 盾刀"
        guide = compress.Samples(rows,atoms,actions,lambda:None)
        def verify(text,n,kind):
            tested.append((text,kind))
            return replay(rows,guide.compatible(programs[text]))
        best,result,records,summary = compress.compress(rules,atoms,actions,replay(rows),render,verify,
            lambda:None,lambda s:s.check(),lambda *v:accepted.append(v),lambda _:None,
            lambda *v:diagnostics.append(v))
        self.assertTrue(compress.certified(result))
        self.assertEqual(len(best),2)
        self.assertEqual(best[0]["atoms"],[2])
        self.assertTrue(any(kind in ("group_replace_and_move","batch_features") for _,kind in tested))
        self.assertEqual(summary["status"],"scope_exhausted")
        self.assertLess(summary["best_chars"],summary["initial_chars"])
        self.assertTrue(all(accepted[i][2]["best_chars"] > accepted[i+1][2]["best_chars"] for i in range(len(accepted)-1)))

    def test_wait_threshold_changes_must_pass_runtime_even_with_equal_truth(self):
        atoms = ["bufftime:X<3.0","buff:Y"]
        actions = [{"name":"盾刀","fcast":False}]
        rows = [sample([1,1],[1],[0],0),sample([0,0],[1],[],1)]
        rules = [{"action":0,"atoms":[0,1]}]
        original = "0,1"
        attempts = []
        def render(candidate):
            return ";".join(",".join(str(k) for k in r["atoms"]) for r in candidate) or "none"
        def verify(text,n,kind):
            attempts.append(text)
            return replay(rows,False)  # Lost wake-up event despite matching columns.
        best,_,_,summary = compress.compress(rules,atoms,actions,replay(rows),render,verify,lambda:None,
            lambda s:s.check(),lambda *v:self.fail("uncertified edit accepted"),lambda _:None,lambda *v:None)
        self.assertEqual(render(best),original)
        self.assertTrue(attempts)
        self.assertEqual(summary["saved_chars"],0)

    def test_clone_keeps_or_lists_independent_and_plain_schema_unchanged(self):
        rules = [{'action':0,'atoms':[0]}, {'action':1,'atoms':[1],'any_atoms':[2,3]}]
        copied = compress.clone(rules)
        self.assertEqual(copied, rules)
        self.assertNotIn('any_atoms', copied[0])
        copied[1]['atoms'].append(4)
        copied[1]['any_atoms'].pop()
        self.assertEqual(rules[1], {'action':1,'atoms':[1],'any_atoms':[2,3]})

    def test_or_hits_cost_and_native_right_association(self):
        atoms = ['buff:P','rage=10','rage=20']
        actions = [{'name':'盾刀','fcast':False}]
        rows = [sample([1,1,0],[1],[0],0), sample([1,0,1],[1],[0],1),
                sample([0,0,1],[1],[],2), sample([1,0,0],[1],[],3)]
        columns = compress.Samples(rows, atoms, actions, lambda:None)
        rule = {'action':0,'atoms':[0],'any_atoms':[1,2]}
        text = native_render([rule], atoms, actions)
        self.assertEqual(text, '/cast [buff:P&rage=10|rage=20] 盾刀')
        self.assertTrue(native_matches(text, atoms, actions, rows))
        self.assertEqual(columns.hit(rule), 0b0011)
        self.assertEqual(columns.hit({'action':0,'atoms':[0],'any_atoms':[1]}), 0b0001)
        self.assertEqual(columns.hit({'action':0,'atoms':[0],'any_atoms':[2]}), 0b0010)
        self.assertEqual(columns.hit(rule), 0b0011)  # Cache includes OR alternatives.
        self.assertEqual(columns.rule_cost(rule), compress.char_count(text))

    def test_shared_prefix_or_merge_needs_one_validation(self):
        atoms = ['buff:P','rage=10','rage=20']
        actions = [{'name':'盾刀','fcast':False},{'name':'绝刀','fcast':False}]
        rows = [sample([1,1,0],[1,1],[0],0), sample([1,0,1],[1,1],[0],1),
                sample([0,1,0],[1,1],[1],2), sample([1,0,0],[1,1],[1],3),
                sample([0,0,1],[1,1],[1],4)]
        rules = [{'action':0,'atoms':[0,1]}, {'action':0,'atoms':[0,2]}, {'action':1,'atoms':[]}]
        calls, accepted = [], []
        def verify(text, n, kind):
            calls.append((kind, text))
            return replay(rows, native_matches(text, atoms, actions, rows))
        best, result, records, summary = compress.compress(rules, atoms, actions, replay(rows),
            lambda candidate:native_render(candidate, atoms, actions), verify, lambda:None,
            lambda solver:solver.check(), lambda *value:accepted.append(value), lambda _:None, lambda *value:None)
        self.assertTrue(compress.certified(result))
        self.assertEqual(best, [{'action':0,'atoms':[0],'any_atoms':[1,2]}, {'action':1,'atoms':[]}])
        self.assertEqual([kind for kind,_ in calls], ['merge_or_suffix'])
        self.assertEqual(len(accepted), 1)
        self.assertEqual(summary['accepted_count'], 1)
        self.assertTrue(records[0]['accepted'])

    def test_or_merge_cannot_preempt_an_intervening_priority_rule(self):
        atoms = ['rage=10','rage=20','buff:X']
        actions = [{'name':'盾刀','fcast':False},{'name':'绝刀','fcast':False}]
        rows = [sample([1,0,0],[1,1],[0],0), sample([0,1,1],[1,1],[1],1),
                sample([1,0,1],[1,1],[0],2), sample([0,1,0],[1,1],[0],3)]
        rules = [{'action':0,'atoms':[0]}, {'action':1,'atoms':[2]}, {'action':0,'atoms':[1]}]
        columns = compress.Samples(rows, atoms, actions, lambda:None)
        self.assertTrue(columns.compatible(rules))
        self.assertEqual(list(compress.or_edits(rules, columns)), [])
        merged = {'action':0,'atoms':[],'any_atoms':[0,1]}
        self.assertFalse(columns.compatible([merged, rules[1]]))
        self.assertFalse(columns.compatible([rules[1], merged]))

    def test_existing_or_suffixes_combine_with_same_prefix(self):
        atoms = ['buff:P','rage=10','rage=20','rage=30','rage=40']
        actions = [{'name':'盾刀','fcast':False},{'name':'绝刀','fcast':False}]
        rows = [sample([1]+[int(k==i) for k in range(4)],[1,1],[0],i) for i in range(4)]
        rows.append(sample([0,0,0,0,1],[1,1],[1],4))
        rules = [{'action':0,'atoms':[0],'any_atoms':[1,2]},
                 {'action':0,'atoms':[0],'any_atoms':[3,4]}, {'action':1,'atoms':[]}]
        columns = compress.Samples(rows, atoms, actions, lambda:None)
        proposals = list(compress.or_edits(rules, columns))
        self.assertTrue(proposals)
        candidate = proposals[0][1]
        self.assertEqual(candidate[0], {'action':0,'atoms':[0],'any_atoms':[1,2,3,4]})
        self.assertTrue(native_matches(native_render(candidate, atoms, actions), atoms, actions, rows))
        mixed = [rules[0], {'action':0,'atoms':[0,3]}, rules[2]]
        mixed_rows = rows[:3] + rows[4:]
        mixed_columns = compress.Samples(mixed_rows, atoms, actions, lambda:None)
        self.assertEqual(list(compress.or_edits(mixed, mixed_columns))[0][1][0]['any_atoms'], [1,2,3])

    def test_pure_or_union_keeps_required_disjoint_states(self):
        atoms = ['rage=10','rage=20']
        actions = [{'name':'盾刀','fcast':False},{'name':'绝刀','fcast':False}]
        rows = [sample([1,0],[1,1],[0],0), sample([0,1],[1,1],[0],1),
                sample([0,0],[1,1],[1],2)]
        rules = [{'action':0,'atoms':[0]}, {'action':0,'atoms':[1]}, {'action':1,'atoms':[]}]
        columns = compress.Samples(rows, atoms, actions, lambda:None)
        candidate = list(compress.or_edits(rules, columns))[0][1]
        self.assertEqual(candidate[0], {'action':0,'atoms':[],'any_atoms':[0,1]})
        self.assertEqual(native_render(candidate, atoms, actions), '/cast [rage=10|rage=20] 盾刀\n/cast 绝刀')
        self.assertTrue(native_matches(native_render(candidate, atoms, actions), atoms, actions, rows))

    def test_or_branch_deletion_still_requires_complete_runtime_certificate(self):
        atoms = ['buff:P','rage=10','bufftime:X<3.0']
        actions = [{'name':'盾刀','fcast':False}]
        rows = [sample([1,1,0],[1],[0],0), sample([0,0,0],[1],[],1)]
        rules = [{'action':0,'atoms':[0],'any_atoms':[1,2]}]
        attempts = []
        def verify(text, n, kind):
            attempts.append(kind)
            return dict(status='ok', rows=rows, truncated=False,
                        comparison=dict(reproduced=True, completed_full_replay=False))
        best, result, records, summary = compress.compress(rules, atoms, actions, replay(rows),
            lambda candidate:native_render(candidate, atoms, actions), verify, lambda:None,
            lambda solver:solver.check(), lambda *value:self.fail('uncertified OR accepted'),
            lambda _:None, lambda *value:None)
        self.assertIn('remove_or_atom', attempts)
        self.assertEqual(best, rules)
        self.assertTrue(compress.certified(result))
        self.assertEqual(summary['saved_chars'], 0)
        self.assertTrue(all(not record['accepted'] for record in records))

    def test_legacy_edits_never_drop_an_existing_or_suffix(self):
        atoms = ['buff:P','rage=10','rage=20']
        actions = [{'name':'盾刀','fcast':True},{'name':'盾刀','fcast':False}]
        rows = [sample([1,1,0],[1,1],[0],0), sample([1,0,1],[1,1],[0],1)]
        rules = [{'action':0,'atoms':[0],'any_atoms':[1,2]}]
        for kind, candidate in compress.simple_edits(rules, actions, lambda:None):
            if kind != 'remove_rule':
                if kind == 'remove_or_atom':
                    self.assertNotIn('any_atoms', candidate[0])
                    self.assertEqual(candidate[0]['atoms'][0], 0)
                    self.assertIn(candidate[0]['atoms'][1], [1,2])
                else:
                    self.assertIn('any_atoms', candidate[0])
                    self.assertEqual(candidate[0]['any_atoms'], [1,2])
        columns = compress.Samples(rows, atoms, actions, lambda:None)
        self.assertEqual(list(compress.feature_edits(rules, columns)), [])

    def test_batch_fcast_conversion_preserves_or_and_and_only_sat_skips_it(self):
        atoms = ['buff:P','rage=10','rage=20']
        actions = [{'name':'盾刀','fcast':True},{'name':'盾刀','fcast':False}]
        rows = [sample([1,1,0],[1,1],[0,1],0), sample([1,0,1],[1,1],[0,1],1),
                sample([0,1,0],[1,1],[],2)]
        rules = [{'action':0,'atoms':[0],'any_atoms':[1,2]}]
        columns = compress.Samples(rows, atoms, actions, lambda:None)
        patches = compress.basic_batch(rules, columns, actions)
        self.assertEqual(patches, [(0, {'action':1,'atoms':[0],'any_atoms':[1,2]})])
        self.assertEqual(compress.feature_batch(rules, columns), [])
        with_fallback = rules + [{'action':1,'atoms':[]}]
        self.assertEqual(list(compress.local_rewrites(with_fallback, columns,
            lambda candidate:native_render(candidate, atoms, actions),
            lambda solver:self.fail('AND-only SAT saw an OR window'), lambda *value:None)), [])


if __name__ == "__main__":
    unittest.main()
