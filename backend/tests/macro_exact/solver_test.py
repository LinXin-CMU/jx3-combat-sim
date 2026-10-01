"""Contract tests for ordered selection, waits and finite-language claims."""
import importlib.util
from pathlib import Path
import unittest

path = Path(__file__).resolve().parents[3] / "tools/exact-macro-synth.py"
spec = importlib.util.spec_from_file_location("exact_synth", path)
synth = importlib.util.module_from_spec(spec)
spec.loader.exec_module(synth)


def row(truth, executable, allowed):
    return {"truth": truth, "executable": executable, "allowed": allowed}


class OrderedSelection(unittest.TestCase):
    def test_alternative_paths_replace_teacher_prefix_instead_of_accumulating(self):
        teacher = [dict(row([1, 0], [1, 1], [0]), cursor=i) for i in range(3)]
        bank = synth.PathSamples(teacher)
        def replay(action):
            return {"comparison": {"acceptance_prefix":2, "exact_prefix":2, "state_prefix":0,
                    "order_prefix":2, "max_time_error_on_order_prefix":0},
                    "rows":[dict(row([1, 0], [1, 1], [action]), cursor=i) for i in range(2)]}
        bank.add('/cast A', replay(0))
        bank.add('/cast B', replay(1))
        _, selected, _ = bank.select(2)
        self.assertEqual([r['allowed'] for r in selected], [[1],[1],[0]])
        self.assertEqual([r['cursor'] for r in selected], [0,1,2])
        self.assertEqual(bank.select(0)[1], teacher)

    def test_local_search_yields_to_full_construction_after_stale_prefix(self):
        scheduler = synth.SearchScheduler()
        scheduler.improved([{'rules':[n]} for n in range(200)])
        self.assertEqual(len(scheduler.frontier), synth.LOCAL_FRONTIER_LIMIT)
        seen = set()
        for _ in range(synth.LOCAL_REPLAYS_PER_PREFIX):
            trial = scheduler.next(seen, str)
            self.assertIsNotNone(trial)
            seen.add(str(trial['rules']))
            scheduler.extend([{'rules':[1000 + _]}], urgent=True)
        self.assertIsNone(scheduler.next(seen, str))
        self.assertTrue(scheduler.exhausted)
        self.assertEqual(scheduler.frontier, [])
        scheduler.extend([{'rules':[9999]}])
        self.assertIsNone(scheduler.next(seen, str))
        scheduler.improved([{'rules':[42]}])
        self.assertEqual(scheduler.next(seen, str)['rules'], [42])

    def test_failure_type_selects_the_local_method(self):
        atoms = ['bufftime:A<15.3', 'bufftime:A<15.2', 'bufftime:A<15.4', 'buff:B']
        rules = [{'action':0, 'atoms':[0]}]
        missed = {'comparison':{'first_difference':{'index':0}}, 'actual':[],
                  'probe_failure':{'kind':'missed_decision_time'},
                  'rows':[dict(row([0,0,1,1], [1], [0]), cursor=0)]}
        self.assertTrue(synth.repair_trials(rules, atoms, missed))
        self.assertTrue(all('to' in trial or trial.get('repair') == 'missing_action_guard'
                            for trial in synth.repair_trials(rules, atoms, missed)))
        self.assertEqual(synth.guard_trials(rules, atoms, missed), [])

    def test_missing_rule_addition_respects_priorities_and_executable_negatives(self):
        atoms = ['buff:A', 'rage=40']
        rules = [{'action':0, 'atoms':[0]}]
        rows = [dict(row([1,1], [1,1], [0]), cursor=0),
                dict(row([0,0], [1,1], []), cursor=1),
                dict(row([0,1], [1,1], [1]), cursor=1)]
        replay = {'comparison':{'first_difference':{'index':1}}, 'actual':[{}],
                  'probe_failure':{'kind':'missed_decision_time'}, 'rows':rows}
        trial = synth.coverage_trials(rules, atoms, replay)[0]
        self.assertEqual(trial['rules'], rules + [{'action':1, 'atoms':[1]}])
        self.assertEqual(rules, [{'action':0, 'atoms':[0]}])

    def test_threshold_expansion_keeps_existing_ids_and_legal_fields(self):
        atoms = ['bufftime:A<2.0', 'rage=40']
        extended = synth.expand_thresholds(atoms, [{'buffs':[
            {'name':'A','remaining':2.36}, {'name':'unknown','remaining':5}], 'target_buffs':[]}])
        self.assertEqual(extended[:len(atoms)], atoms)
        self.assertIn('bufftime:A<2.4', extended)
        self.assertFalse(any('unknown' in a for a in extended))

    def test_unused_early_rule_can_wait_for_a_legal_timer_instead_of_only_being_dropped(self):
        atoms = ['buff:A', 'bufftime:B<9.9', 'buff:C']
        rules = [{'action':0, 'atoms':[0]}]
        last = dict(row([1,0,0], [1], []), cursor=0, time=2, rejected_actions=[0],
                    state={'buffs':[{'name':'B','remaining':10}], 'target_buffs':[]})
        replay = {'comparison':{'time_tolerance_seconds':0.125, 'first_difference':{
            'index':0, 'expected':{'skill_id':1,'time':2.2},
            'actual':{'skill_id':1,'time':2}}},
            'actual':[{'macro_line':1}], 'rows':[last]}
        trial = synth.guard_trials(rules, atoms, replay)[0]
        self.assertEqual(trial['repair'], 'early_cast_clock_gate')
        self.assertEqual(trial['rules'][0]['atoms'], [1,0])
        self.assertEqual(rules[0]['atoms'], [0])

    def test_missed_cast_uses_only_aligned_executable_target_rules(self):
        atoms = ['bufftime:A<15.3', 'bufftime:A<15.2', 'bufftime:A<15.4', 'buff:B']
        rules = [{'action': 0, 'atoms': [0]}, {'action': 1, 'atoms': [0]}, {'action': 2, 'atoms': [0]}]
        trace = {'comparison': {'first_difference': {'index': 1, 'expected': {'time': 2}}},
                 'actual': [{}], 'probe_failure': {'kind': 'missed_decision_time'},
                 'rows': [dict(row([False,False,True,True], [True,True,False], [1,2]), cursor=1)]}
        trials = synth.threshold_trials(rules, atoms, trace)
        self.assertEqual([t['line'] for t in trials], [2,2])
        self.assertEqual(trials[0]['to'], atoms[2])
        self.assertEqual(rules[1]['atoms'], [0])
        trace['rows'][-1]['cursor'] = 2
        self.assertEqual(synth.threshold_trials(rules, atoms, trace), [])

    def test_preempting_rule_can_split_across_different_correct_contexts(self):
        atoms = ['bufftime:A<1.0','buff:B','buff:C']
        rules = [{'action':0, 'atoms':[0]}, {'action':1, 'atoms':[]}]
        trace = {'comparison': {'first_difference': {'index':2}},
                 'actual':[{'macro_line':1}]*3,
                 'rows':[dict(row([1,1,0],[1,1],[0]),cursor=0),
                         dict(row([1,0,1],[1,1],[0]),cursor=1),
                         dict(row([1,0,0],[1,1],[]),cursor=2,rejected_actions=[0])]}
        trials = synth.guard_trials(rules, atoms, trace)
        self.assertTrue(trials)
        repaired = trials[0]['rules']
        self.assertEqual(len(repaired),3)
        self.assertEqual(repaired[-1],rules[-1])
        self.assertTrue(all(0 in rule['atoms'] for rule in repaired[:-1]))
        for sample in trace['rows'][:2]:
            self.assertTrue(any(all(sample['truth'][a] for a in r['atoms']) for r in repaired[:-1]))
        self.assertFalse(any(all(trace['rows'][-1]['truth'][a] for a in r['atoms']) for r in repaired[:-1]))
        self.assertEqual(rules[0]['atoms'],[0])
        before = synth.repair_signature(trace)
        self.assertNotEqual(synth.repair_signature(trace, rules), synth.repair_signature(trace, repaired))
        trace['actual'] = [{ 'macro_line': 1 }, { 'macro_line': 1 }, { 'macro_line': 2 }]
        self.assertNotEqual(synth.repair_signature(trace),before)

    def test_packed_trace_truth_is_decoded_for_local_repair(self):
        self.assertEqual(synth.row_truth({'truth_hex':'8501','truth_count':9}), bytes([1,0,1,0,0,0,0,1,1]))

    def test_local_threshold_trials_are_hypotheses_in_the_existing_language(self):
        atoms = ["bufftime:盾飞<15.3", "bufftime:盾飞<15.2", "bufftime:盾飞<15.4", "bufftime:盾飞<15.5"]
        rules = [{"action": 0, "atoms": [0]}]
        trace = {"comparison": {"first_difference": {"index": 0, "actual": {"time": 2}, "expected": {"time": 1}}},
                 "actual": [{"macro_line": 1, "macro_page": 1}],
                 "rows": [dict(row([1,1,0,0],[1],[]),cursor=0,rejected_actions=[0])]}
        trials = synth.threshold_trials(rules, atoms, trace)
        self.assertEqual([trial["to"] for trial in trials], [atoms[2], atoms[1], atoms[3]])
        self.assertEqual(rules[0]["atoms"], [0])
        self.assertTrue(all("status" not in trial for trial in trials))
        trace["actual"] = []
        self.assertEqual(synth.threshold_trials(rules, atoms, trace), [])

    def assert_threshold_metadata_after_prefix(self, prefix):
        atoms = ['buff:A', 'buff:B', 'buff:C', 'bufftime:D<15.3',
                 'bufftime:D<15.2', 'bufftime:D<15.4', 'bufftime:D<15.5']
        rules = [prefix, {'action':1, 'atoms':[3]}]
        trace = {'comparison':{'first_difference':{'index':0,
                    'actual':{'time':2}, 'expected':{'time':1}}},
                 'actual':[{'macro_line':2, 'macro_page':1}],
                 'rows':[dict(row([0,0,0,1,1,0,0], [1,1], []),
                              cursor=0, rejected_actions=[1])]}
        trials = synth.threshold_trials(rules, atoms, trace)
        self.assertTrue(trials)
        self.assertEqual([trial['line'] for trial in trials], [2]*len(trials))
        self.assertEqual([trial['from'] for trial in trials], [atoms[3]]*len(trials))
        self.assertEqual([trial['to'] for trial in trials], [atoms[5], atoms[4], atoms[6]])
        normalize = synth.repair_module().conditions().normalize
        for trial in trials:
            self.assertEqual(normalize(trial['rules'][0]), normalize(prefix))
        self.assertEqual(rules, [prefix, {'action':1, 'atoms':[3]}])

    def test_threshold_metadata_ignores_unrelated_explicit_and_normalization(self):
        self.assert_threshold_metadata_after_prefix({'action':0, 'atoms':[0,1], 'ops':['&']})

    def test_threshold_metadata_ignores_unrelated_legacy_or_normalization(self):
        self.assert_threshold_metadata_after_prefix({'action':0, 'atoms':[0], 'any_atoms':[1,2]})

    def test_conflicting_seed_rule_can_split_without_limit_or_overlap(self):
        rows = [row([True, False], [True, True], [0]),
                row([False, True], [True, True], [0]),
                row([False, False], [True, True], [1])]
        result = synth.construct(rows, ["buff:A", "buff:B"], [{}, {}],
                                 seed=[{"action": 0, "atoms": []}, {"action": 1, "atoms": []}])
        self.assertEqual(result["status"], "sat")
        self.assertEqual(result["repaired_rules"], 2)
        self.assertEqual(result["rules"], [{"action": 0, "atoms": [0]},
                                          {"action": 0, "atoms": [1]}, {"action": 1, "atoms": []}])

    def test_repair_removes_weaker_old_bound_and_can_change_priority(self):
        result = synth.construct([row([True, True], [True], [0]), row([True, False], [True], [])],
                                 ["bufftime:A<9.0", "bufftime:A<5.0"], [{}],
                                 seed=[{"action": 0, "atoms": [0]}])
        self.assertEqual(result["rules"], [{"action": 0, "atoms": [1]}])
        result = synth.construct([row([], [True, False], [0]), row([], [True, True], [1])],
                                 [], [{}, {}], seed=[{"action": 0, "atoms": []}, {"action": 1, "atoms": []}])
        self.assertEqual([r["action"] for r in result["rules"]], [1, 0])

    def test_rejected_last_action_at_deadline_is_a_conflict_not_assertion_failure(self):
        sample = dict(row([True], [True], []), wait_allowed=False, rejected_actions=[0])
        self.assertEqual(synth.conflict([sample])["states"], [sample])

    def test_seed_rules_are_repaired_not_frozen(self):
        rows = [row([True], [True, True], [0]), row([False], [True, True], [1])]
        bad_seed = [{"action": 0, "atoms": []}]
        result = synth.construct(rows, ["buff:X"], [{}, {}], seed=bad_seed)
        self.assertEqual(result["status"], "sat")
        self.assertEqual(result["rules"], [{"action": 0, "atoms": [0]}, {"action": 1, "atoms": []}])

    def test_cast_now_is_reversible_branch_not_a_permanent_wait_label(self):
        rows = [dict(row([False], [True], []), cursor=0, wait_allowed=True),
                dict(row([False], [True], [0]), cursor=0, wait_allowed=True),
                dict(row([True], [True], [0]), cursor=0, wait_allowed=True)]
        atoms, actions = ["buff:X"], [{}]
        cache = synth.SampleIndex()
        branch = synth.construct(rows, atoms, actions, obligations=[1], cache=cache)
        self.assertEqual(branch["status"], "unsat")
        retry = synth.construct(rows, atoms, actions, cache=cache)
        self.assertEqual(retry["status"], "sat")
        self.assertTrue(rows[1]["wait_allowed"])

    def test_cached_columns_split_after_new_counterexample(self):
        rows = [row([True, True], [True], [0])]
        cache, atoms, actions = synth.SampleIndex(), ["rage>0", "buff:X"], [{}]
        self.assertEqual(synth.construct(rows, atoms, actions, cache=cache)["rules"][0]["atoms"], [])
        rows.append(row([True, False], [True], []))
        self.assertEqual(synth.construct(rows, atoms, actions, cache=cache)["rules"][0]["atoms"], [1])

    def test_wait_hint_uses_only_last_aligned_state(self):
        r = dict(row([True], [True], [0]), cursor=2, time=2.1,
                 wait_allowed=True, wait_next_time=2.4, decision_latest=2.27, wake_atoms=[0])
        replay = {"rows": [r], "probe_failure": {"kind": "missed_decision_time", "index": 2, "time": 2.4}}
        indices = {synth.row_key(r): 0}
        hint = synth.wait_witness(replay, indices)
        self.assertEqual((hint["row"], hint["next_time"], hint["wake_atoms"]), (0, 2.4, [0]))
        replay["probe_failure"]["index"] = 3
        self.assertIsNone(synth.wait_witness(replay, indices))

    def test_time_window_allows_wait_and_only_requires_one_hit_per_target(self):
        rows = [dict(row([False], [True], []), cursor=0, wait_allowed=True),
                dict(row([False], [True], [0]), cursor=0, wait_allowed=True),
                dict(row([True], [True], [0]), cursor=0, wait_allowed=True)]
        self.assertIsNone(synth.conflict(rows))
        result = synth.construct(rows, ["buff:X"], [{}])
        self.assertEqual(result["status"], "sat")
        self.assertEqual(result["rules"], [{"action": 0, "atoms": [0]}])

    def test_prototype_has_no_rule_slot_ceiling(self):
        count = 40
        rows = [row([i == j for j in range(count)], [True] * count, [i]) for i in range(count)]
        result = synth.construct(rows, [f"rage={i}" for i in range(count)], [{}] * count)
        self.assertEqual(result["status"], "sat")
        self.assertEqual(len(result["rules"]), count)
        self.assertIsNone(result["slots"])

    def test_prototype_uses_all_needed_terms_and_drops_weaker_overlap(self):
        # Six independent conditions are necessary; the seventh is implied by
        # the first over these observations and adds no exclusion evidence.
        rows = [row([True] * 7, [True], [0])]
        for i in range(6):
            truth = [j != i for j in range(6)] + [True]
            rows.append(row(truth, [True], []))
        result = synth.construct(rows, [f"buff:{i}" for i in range(7)], [{}])
        self.assertEqual(result["status"], "sat")
        self.assertEqual(set(result["rules"][0]["atoms"]), set(range(6)))
        self.assertIsNone(result["terms"])

    def test_prototype_respects_priority_and_unavailable_negatives(self):
        for rows, atoms in [
            ([row([], [True, True], [0]), row([], [False, True], [1])], []),
            ([row([True], [True, True], [0]), row([False], [True, True], [1])], ["buff:X"]),
        ]:
            result = synth.construct(rows, atoms, [{}, {}])
            self.assertEqual(result["status"], "sat")
            self.assertEqual([r["action"] for r in result["rules"]], [0, 1])
            self.assertEqual(result["rules"][1]["atoms"], [])

    def test_prototype_rebuilds_atom_equivalence_after_counterexample(self):
        rows = [row([True, True], [True], [0])]
        first = synth.construct(rows, ["rage>0", "buff:X"], [{}])
        self.assertEqual(first["rules"][0]["atoms"], [])
        rows.append(row([True, False], [True], []))
        second = synth.construct(rows, ["rage>0", "buff:X"], [{}])
        self.assertEqual(second["rules"][0]["atoms"], [1])

    def test_prototype_reports_conjunction_obstruction_without_slot_unsat(self):
        rows = [row([False], [True], [0]), row([True], [True], [])]
        result = synth.construct(rows, ["buff:X"], [{}])
        self.assertEqual(result["status"], "unsat")
        self.assertEqual(result["conflict"]["blockers"], [{"anchor": 0, "action": 0, "wrong_row": 1}])

    def test_unavailable_action_does_not_need_false_condition(self):
        rows = [row([], [True, True], [0]), row([], [False, True], [1])]
        result, _ = synth.solve(rows, [], [{}, {}], 2, 1, 3000)
        self.assertEqual(result["status"], "sat")
        self.assertEqual([r["action"] for r in result["rules"]], [0, 1])

    def test_lower_priority_wrong_rule_can_still_be_true(self):
        rows = [row([True], [True, True], [0]), row([False], [True, True], [1])]
        result, _ = synth.solve(rows, ["buff:X"], [{}, {}], 2, 1, 3000)
        self.assertEqual(result["status"], "sat")
        self.assertEqual(result["rules"], [{"action": 0, "atoms": [0]}, {"action": 1, "atoms": []}])

    def test_wait_at_same_observation_is_real_conflict(self):
        rows = [row([True], [True], [0]), row([True], [True], [])]
        self.assertIsNotNone(synth.conflict(rows))
        result, _ = synth.solve(rows, ["rage>0"], [{}], 4, 2, 3000)
        self.assertEqual(result["status"], "unsat")

    def test_new_counterexample_splits_previously_equivalent_atoms(self):
        rows = [row([True, True], [True], [0])]
        first, _ = synth.solve(rows, ["rage>0", "buff:X"], [{}], 1, 1, 3000)
        self.assertEqual(first["distinct_atoms"], 0)
        rows.append(row([True, False], [True], []))
        second, _ = synth.solve(rows, ["rage>0", "buff:X"], [{}], 1, 1, 3000)
        self.assertEqual(second["status"], "sat")
        self.assertEqual(second["rules"][0]["atoms"], [1])

    def test_slot_unsat_is_distinct_from_language_conflict(self):
        rows = [row([], [True, True], [0]), row([], [False, True], [1])]
        self.assertIsNone(synth.conflict(rows))
        result, _ = synth.solve(rows, [], [{}, {}], 1, 1, 3000)
        self.assertEqual(result["status"], "unsat")

    def test_encoding_budget_is_unknown_not_unsat(self):
        rows = [row([True], [True], [0])]
        result, _ = synth.solve(rows, ["rage>0"], [{}], 1, 1, 3000, deadline=0)
        self.assertEqual(result["status"], "unknown")
        self.assertEqual(result["reason"], "wall_budget_during_encoding")


if __name__ == "__main__":
    unittest.main()
