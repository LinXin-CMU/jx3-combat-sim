"""Stage two: shorten a certified macro against one frozen target, without B.

Sample columns guide search, never certify an edit or prove it impossible.
Every admitted program is parsed and replayed from t=0 by the real oracle.
The finite neighbourhood/beam/local SAT shapes control search breadth only.
"""
import time
import importlib.util
import hashlib
import json
from pathlib import Path

import z3

_PRIORITY_MODULE = None
_CONDITION_MODULE = None
_GLOBAL_MODULE = None
_FAMILY_MODULE = None


def condition_module():
    global _CONDITION_MODULE
    if _CONDITION_MODULE is None:
        spec = importlib.util.spec_from_file_location("exact_macro_conditions", Path(__file__).with_name("exact_macro_conditions.py"))
        _CONDITION_MODULE = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(_CONDITION_MODULE)
    return _CONDITION_MODULE


def complex_guard(rule):
    return bool(rule.get('any_atoms')) or '|' in rule.get('ops', [])


def remove_chain_atom(rule, index):
    value = clone([rule])[0]
    value['atoms'].pop(index)
    if 'ops' in value:
        if value['ops']:
            value['ops'].pop(min(index, len(value['ops'])-1))
        if '|' not in value['ops']:
            value.pop('ops')
    return value


def priority_edits(rules, samples, native=False):
    global _PRIORITY_MODULE
    if _PRIORITY_MODULE is None:
        spec = importlib.util.spec_from_file_location("exact_macro_reorder", Path(__file__).with_name("exact_macro_reorder.py"))
        _PRIORITY_MODULE = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(_PRIORITY_MODULE)
    search = None
    if native:
        search = lambda p,n,s:condition_module().short_guards(p,n,s,max_terms=6,max_candidates=4,branch_limit=12,max_states=192)
    yield from _PRIORITY_MODULE.priority_edits(rules, samples, clone, condition_search=search)


def global_edits(rules, samples, check_solver, diagnostic):
    global _GLOBAL_MODULE
    if _GLOBAL_MODULE is None:
        spec = importlib.util.spec_from_file_location("exact_macro_global", Path(__file__).with_name("exact_macro_global.py"))
        _GLOBAL_MODULE = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(_GLOBAL_MODULE)
    yield from _GLOBAL_MODULE.global_edits(rules, samples, clone, check_solver, diagnostic)


def family_edits(rules, samples, diagnostic):
    global _FAMILY_MODULE
    if _FAMILY_MODULE is None:
        spec = importlib.util.spec_from_file_location("exact_macro_family", Path(__file__).with_name("exact_macro_family.py"))
        _FAMILY_MODULE = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(_FAMILY_MODULE)
    search = lambda p,n,s:condition_module().short_guards(
        p,n,s,max_terms=6,max_candidates=4,branch_limit=12,max_states=192)
    yield from _FAMILY_MODULE.family_edits(rules, samples, clone,
        condition_search=search, diagnostic=diagnostic)


def char_count(text):
    # Same as the macro draft's JavaScript .length: includes brackets, spaces
    # and newlines; no unsafe operator/decimal normalization after validation.
    return len(text.encode("utf-16-le")) // 2


def clone(rules):
    copied = []
    for rule in rules:
        value = {"action": rule["action"], "atoms": list(rule["atoms"])}
        if "any_atoms" in rule and 'ops' not in rule:
            value["any_atoms"] = list(rule["any_atoms"])
        if "ops" in rule:
            value["ops"] = list(rule["ops"])
        copied.append(value)
    return copied


def certified(replay):
    c = replay.get("comparison", {})
    return (replay.get("status") == "ok" and c.get("reproduced") is True
            and c.get("completed_full_replay") is True and not replay.get("truncated", False))


class Samples:
    """Columns from ONE certified trajectory, including its wait decisions."""
    def __init__(self, rows, atoms, actions, check):
        self.rows, self.atoms, self.actions, self.check = rows, atoms, actions, check
        self.all = (1 << len(rows)) - 1
        self.truth = [0] * len(atoms)
        self.executable = [0] * len(actions)
        self.allowed = [0] * len(actions)
        self.required, self.groups = 0, {}
        for s, row in enumerate(rows):
            check()
            bit = 1 << s
            for k, value in enumerate(row["truth"]):
                if value:
                    self.truth[k] |= bit
            for a, value in enumerate(row["executable"]):
                if value:
                    self.executable[a] |= bit
            for a in row["allowed"]:
                self.allowed[a] |= bit
            if row["allowed"]:
                self.groups[row.get("cursor", s)] = self.groups.get(row.get("cursor", s), 0) | bit
                if not row.get("wait_allowed", False):
                    self.required |= bit
        self.guards = {}
        self.hit_cache = {}
        self.atom_costs = [char_count(atom) for atom in atoms]
        self.action_costs = [char_count(f"/{'fcast' if a.get('fcast') else 'cast'} {a.get('name','')}")
                             for a in actions]
        self.short_features = None

    def hit(self, rule):
        key = rule['action'], tuple(rule['atoms']), tuple(rule.get('any_atoms', ())), tuple(rule.get('ops', ())), 'ops' in rule
        cached = self.hit_cache.get(key)
        if cached is not None:
            return cached
        bits = self.executable[rule["action"]]
        if 'ops' in rule:
            bits &= condition_module().condition_mask(rule, self.truth, self.all)
            self.hit_cache[key] = bits
            return bits
        for k in rule["atoms"]:
            bits &= self.truth[k]
        if rule.get("any_atoms"):
            alternatives = 0
            for k in rule["any_atoms"]:
                alternatives |= self.truth[k]
            bits &= alternatives
        self.hit_cache[key] = bits
        return bits

    def rule_cost(self, rule):
        if 'ops' in rule:
            return self.action_costs[rule['action']] + condition_module().condition_cost(rule,self.atom_costs)
        terms = rule['atoms'] + rule.get('any_atoms', [])
        return self.action_costs[rule['action']] + (2 + sum(self.atom_costs[k]+1 for k in terms) if terms else 0)

    def selections(self, rules):
        remaining, selected = self.all, []
        for r in rules:
            bits = remaining & self.hit(r)
            selected.append(bits)
            remaining &= ~bits
        return selected, remaining

    def compatible(self, rules):
        remaining = self.all
        for rule in rules:
            bits = remaining & self.hit(rule)
            if bits & ~self.allowed[rule['action']]:
                return False
            remaining &= ~bits
        if remaining & self.required:
            return False
        return all(group & ~remaining for group in self.groups.values())

    def covers(self, positives, negatives):
        """Short alternative conjunctions; masks are guidance, not equivalence."""
        key = positives, negatives
        if key in self.guards:
            return self.guards[key]
        if not positives:
            return []
        if not negatives:
            return [[]]
        columns = []
        for k, truth in enumerate(self.truth):
            if k % 64 == 0:
                self.check()
            excluded = negatives & ~truth
            if positives & ~truth == 0 and excluded:
                columns.append((k, excluded))
        alternatives = [[k] for k, excluded in columns if excluded == negatives]
        # Several inexpensive covers with distinct first predicates. A new
        # feature can replace the whole guard, rather than only deleting terms.
        columns.sort(key=lambda p: (self.atom_costs[p[0]], -p[1].bit_count(), p[0]))
        for first in [None] + [k for k, _ in columns[:8]]:
            pending, guard = negatives, []
            if first is not None:
                guard.append(first)
                pending &= self.truth[first]
            while pending:
                self.check()
                viable = [(k, bits & pending) for k, bits in columns if bits & pending and k not in guard]
                if not viable:
                    break
                k, excluded = min(viable, key=lambda p: ((self.atom_costs[p[0]]+1)/p[1].bit_count(), p[0]))
                guard.append(k)
                pending &= ~excluded
            if not pending:
                # Remove sample-redundant terms within the proposed row. The
                # complete replay also checks the wake-up thresholds removed.
                for k in list(guard):
                    covered = 0
                    for other in guard:
                        if other != k:
                            covered |= negatives & ~self.truth[other]
                    if covered == negatives:
                        guard.remove(k)
                alternatives.append(sorted(guard))
        unique = {tuple(g): g for g in alternatives}
        result = sorted(unique.values(), key=lambda g: (sum(self.atom_costs[k]+1 for k in g), g))[:4]
        self.guards[key] = result
        return result


def branch_guide(replay, source, known=()):
    """Only the correctly aligned prefix of ONE reached candidate path.

    The native probe only emits correctly aligned pre-decision observations,
    including the first rejected decision. Never combine two failed paths or
    label a diverged suffix. Do not attach a teacher's unvisited suffix as
    unconditional constraints: even correct source states may be unreachable
    after a different tolerated prefix. The full frozen oracle still decides
    acceptance through the target's complete horizon.
    """
    if not replay:
        return None
    difference = replay.get('comparison', {}).get('first_difference') or {}
    if replay.get('status') != 'ok' or not replay.get('probe_failure') or 'index' not in difference:
        return None
    cursor = difference['index']
    prefix = [r for r in replay.get('rows', []) if r.get('cursor', cursor+1) <= cursor]
    if not prefix or any(len(r.get('truth', ())) != len(source.atoms) for r in prefix):
        return None
    signature = tuple((r['cursor'],round(r['time'],9),bytes(r['truth']),
                       tuple(r['executable']),tuple(r['allowed']),r.get('wait_allowed',False)) for r in prefix)
    if signature in known:
        return None
    guide = Samples(prefix,source.atoms,source.actions,source.check)
    guide.global_keep_mask = (1 << len(prefix))-1
    guide.guide_kind = 'one_aligned_candidate_prefix_only'
    return cursor,signature,guide


class BatchPlan:
    """Stable original row IDs let runtime failures split without index drift."""
    def __init__(self, rules, samples):
        self.original = clone(rules)
        self.current = {i:r for i,r in enumerate(clone(rules))}
        self.samples = samples
        self.units = sum(samples.rule_cost(r)+1 for r in rules)

    def rules(self):
        return [r for _,r in sorted(self.current.items())]

    def change(self, patches):
        self.samples.check()
        current, units = dict(self.current), self.units
        for i,replacement in patches.items():
            if i in current:
                units -= self.samples.rule_cost(current.pop(i))+1
            if replacement is not None:
                current[i] = replacement
                units += self.samples.rule_cost(replacement)+1
        if units >= self.units or not self.samples.compatible([r for _,r in sorted(current.items())]):
            return False
        self.current, self.units = current, units
        return True

    def patches(self):
        return [(i,self.current.get(i)) for i,r in enumerate(self.original) if self.current.get(i) != r]

    def groups(self):
        groups = {}
        for i,r in sorted(self.current.items()):
            groups.setdefault(r['action'],[]).append(i)
        return groups.values()


def clusters(ids):
    """Whole action region first, then smaller disjoint chunks, not all pairs."""
    if len(ids) < 2:
        return
    yield ids
    for width in (8,4,2):
        if width < len(ids):
            for start in range(0,len(ids),width):
                if len(ids[start:start+width]) > 1:
                    yield ids[start:start+width]


def basic_batch(rules, samples, actions):
    plan = BatchPlan(rules,samples)
    chosen,_ = samples.selections(rules)
    # No observed selection is not a deletion proof: these rows can supply
    # wake-up thresholds. The combined proposal still needs full replay.
    plan.change({i:None for i,bits in enumerate(chosen) if not bits})
    for i in list(plan.current):
        r = plan.current.get(i)
        if r and actions[r['action']]['fcast']:
            normal = next((a for a,v in enumerate(actions) if not v['fcast'] and v['name']==actions[r['action']]['name']),None)
            if normal is not None:
                replacement = clone([r])[0]
                replacement['action'] = normal
                plan.change({i:replacement})
    for i in sorted(plan.current,key=lambda i:-samples.rule_cost(plan.current[i])):
        plan.change({i:None})
    for i in list(plan.current):
        if '|' in plan.current[i].get('ops',[]):
            continue
        for atom in sorted(plan.current[i]['atoms'],key=lambda k:-samples.atom_costs[k]):
            r = plan.current[i]
            replacement = clone([r])[0]
            replacement['atoms'] = [k for k in r['atoms'] if k!=atom]
            replacement.pop('ops',None)
            plan.change({i:replacement})
    for ids in list(plan.groups()):
        for group in clusters(ids):
            live = [i for i in group if i in plan.current]
            if len(live) < 2 or any(complex_guard(plan.current[i]) for i in live):
                continue
            guard = set(plan.current[live[0]]['atoms'])
            for i in live[1:]:
                guard.intersection_update(plan.current[i]['atoms'])
            patches = {i:None for i in live[1:]}
            patches[live[0]] = {'action':plan.current[live[0]]['action'],'atoms':sorted(guard)}
            plan.change(patches)
    return plan.patches()


def feature_batch(rules, samples):
    plan = BatchPlan(rules,samples)
    for ids in list(plan.groups()):
        for group in clusters(ids):
            live = [i for i in group if i in plan.current]
            if len(live) < 2 or any(complex_guard(plan.current[i]) for i in live):
                continue
            keys = sorted(plan.current)
            chosen,_ = samples.selections(plan.rules())
            selected = dict(zip(keys,chosen))
            positives = 0
            for i in live:
                positives |= selected[i]
            a = plan.current[live[0]]['action']
            for position in (live[0],live[-1]):
                earlier = 0
                for i,bits in selected.items():
                    if i < position:
                        earlier |= bits
                negatives = samples.all & ~earlier & samples.executable[a] & ~samples.allowed[a]
                for guard in samples.covers(positives,negatives):
                    patches = {i:None for i in live if i!=position}
                    patches[position] = {'action':a,'atoms':guard}
                    if plan.change(patches):
                        break
                if len([i for i in live if i in plan.current]) < len(live):
                    break
    # Apply short replacements across many independent action rows in one
    # proposal. Cost checks precede the selection check and avoid rendering.
    for i in list(sorted(plan.current)):
        if complex_guard(plan.current[i]):
            continue
        keys = sorted(plan.current)
        chosen,_ = samples.selections(plan.rules())
        selected = dict(zip(keys,chosen))
        earlier = 0
        for key,bits in selected.items():
            if key < i:
                earlier |= bits
        a = plan.current[i]['action']
        negatives = samples.all & ~earlier & samples.executable[a] & ~samples.allowed[a]
        for guard in samples.covers(selected[i],negatives):
            if plan.change({i:{'action':a,'atoms':guard}}):
                break
    return plan.patches()


def simple_edits(rules, actions, check):
    for i, rule in enumerate(rules):
        check()
        yield "remove_rule", rules[:i] + rules[i+1:]
        if actions[rule["action"]]["fcast"]:
            normal = next((a for a, v in enumerate(actions) if not v["fcast"]
                           and v["name"] == actions[rule["action"]]["name"]), None)
            if normal is not None:
                trial = clone(rules)
                trial[i]["action"] = normal
                yield "fcast_to_cast", trial
        for k in range(len(rule["atoms"])):
            trial = clone(rules)
            trial[i] = remove_chain_atom(rule,k) if 'ops' in rule else clone([rule])[0]
            if 'ops' not in rule:
                trial[i]['atoms'].pop(k)
            yield "remove_atom", trial
        # Removing an OR alternative narrows this guard. The final alternative
        # is retained; deleting the line handles an always-false guard instead.
        if len(rule.get("any_atoms", [])) > 1:
            for k in range(len(rule["any_atoms"])):
                check()
                trial = clone(rules)
                trial[i]["any_atoms"].pop(k)
                if len(trial[i]["any_atoms"]) == 1:
                    trial[i]["atoms"].extend(trial[i].pop("any_atoms"))
                yield "remove_or_atom", trial
        for j in range(i+1, len(rules)):
            if (rules[j]["action"] == rule["action"]
                    and not complex_guard(rule) and not complex_guard(rules[j])):
                trial = clone(rules)
                trial[i] = {"action":rule["action"],"atoms":sorted(set(rule["atoms"]) & set(rules[j]["atoms"]))}
                trial.pop(j)
                yield "merge_same_action", trial


def feature_edits(rules, samples):
    selections, _ = samples.selections(rules)
    remaining = samples.all
    for i, rule in enumerate(rules):
        samples.check()
        a = rule["action"]
        negatives = remaining & samples.executable[a] & ~samples.allowed[a]
        if not complex_guard(rule):
            for guard in samples.covers(selections[i], negatives):
                trial = clone(rules)
                trial[i] = {'action':a,'atoms':guard}
                yield "replace_guard", trial
        remaining &= ~selections[i]
    # Joint replacement + deletion + relocation. Intermediate standalone edits
    # need not pass: the whole group is handed to the executor atomically.
    groups = {}
    for i, rule in enumerate(rules):
        groups.setdefault(rule["action"], []).append(i)
    for a, positions in groups.items():
        if any(complex_guard(rules[i]) for i in positions):
            continue
        for left, i in enumerate(positions):
            for j in positions[left+1:]:
                samples.check()
                positives = selections[i] | selections[j]
                for position in (i, j):
                    earlier = 0
                    for bits in selections[:position]:
                        earlier |= bits
                    negatives = samples.all & ~earlier & samples.executable[a] & ~samples.allowed[a]
                    for guard in samples.covers(positives, negatives):
                        trial = clone(rules)
                        trial[position] = {'action':a,'atoms':guard}
                        trial.pop(j if position == i else i)
                        yield "group_replace_and_move", trial


def or_edits(rules, samples, pair_limit=128):
    """Exact guard unions in the native right-associative macro language.

    P&A | P&B is written P&A|B, meaning P AND (A OR B). Each remainder
    must be one literal; arbitrary ORs of conjunctions need grouping the game
    grammar cannot express. The pair bound controls this search pass only.
    """
    groups = {}
    for i, rule in enumerate(rules):
        groups.setdefault(rule['action'], []).append(i)
    pairs = 0
    for action, positions in groups.items():
        for offset, i in enumerate(positions):
            for j in positions[offset+1:]:
                samples.check()
                if pairs >= pair_limit:
                    return
                pairs += 1
                left, right = rules[i], rules[j]
                if '|' in left.get('ops',[]) or '|' in right.get('ops',[]):
                    continue
                left_prefix, right_prefix = set(left['atoms']), set(right['atoms'])
                left_any, right_any = left.get('any_atoms'), right.get('any_atoms')
                if left_any or right_any:
                    prefix = left_prefix if left_any else right_prefix
                    if ((left_any and left_prefix != prefix)
                            or (right_any and right_prefix != prefix)):
                        continue
                    left_tail = list(left_any) if left_any else sorted(left_prefix - prefix)
                    right_tail = list(right_any) if right_any else sorted(right_prefix - prefix)
                    if ((not left_any and (not prefix <= left_prefix or len(left_tail) != 1))
                            or (not right_any and (not prefix <= right_prefix or len(right_tail) != 1))):
                        continue
                else:
                    prefix = left_prefix & right_prefix
                    left_tail, right_tail = sorted(left_prefix-prefix), sorted(right_prefix-prefix)
                    if len(left_tail) != 1 or len(right_tail) != 1:
                        continue
                suffix = sorted(set(left_tail) | set(right_tail))
                if len(suffix) < 2:
                    continue
                merged = {'action':action, 'atoms':sorted(prefix), 'any_atoms':suffix}
                if samples.rule_cost(merged)+1 >= samples.rule_cost(left)+samples.rule_cost(right)+2:
                    continue
                for position in (i, j):
                    samples.check()
                    trial = clone(rules)
                    trial[position] = clone([merged])[0]
                    trial.pop(j if position == i else i)
                    # Relocating an exact union can preempt an intervening skill.
                    # Check the actual ordered program, then require real replay.
                    if samples.compatible(trial):
                        yield 'merge_or_suffix', trial


def local_rewrites(rules, samples, render, check_solver, diagnostic):
    """Finite, weighted SAT over small coupled windows; never global UNSAT.

    Choose actions, guards and order jointly, minimizing copyable characters
    by tightening a cost bound. Fixed outside rules participate in selection.
    Per-check timeouts yield UNKNOWN and let other windows run.
    """
    chosen, _ = samples.selections(rules)
    for start in range(0, len(rules)-1):
        samples.check()
        window = rules[start:start+3]
        if any(complex_guard(rule) for rule in window):
            continue
        region = 0
        for bits in chosen[start:start+len(window)]:
            region |= bits
        if not region:
            continue
        action_ids = sorted({r["action"] for r in window})
        for a in list(action_ids):
            v = samples.actions[a]
            if v["fcast"]:
                action_ids.extend(k for k, other in enumerate(samples.actions)
                                  if not other["fcast"] and other["name"] == v["name"] and k not in action_ids)
        # Preserve existing literals, and add short distinguishing features.
        old = {k for r in window for k in r["atoms"]}
        if samples.short_features is None:
            signatures = {}
            for k,bits in enumerate(samples.truth):
                if bits != 0 and bits != samples.all:
                    previous = signatures.get(bits)
                    if previous is None or samples.atom_costs[k] < samples.atom_costs[previous]:
                        signatures[bits] = k
            samples.short_features = sorted(signatures.values(),key=lambda k:(samples.atom_costs[k],k))[:24]
        pool = samples.short_features
        pool = sorted(old | set(pool))
        if len(pool) > 64:
            diagnostic({"kind":"local_sat", "start":start, "status":"skipped_large_window"}, None)
            continue
        solver = z3.Solver()
        solver.set(timeout=1000, random_seed=0)
        slots, variables, weights = [], [], []
        for s in range(len(window)):
            acts = {a:z3.Bool(f"a_{s}_{a}") for a in action_ids}
            cond = {k:z3.Bool(f"c_{s}_{k}") for k in pool}
            enabled = z3.Bool(f"on_{s}")
            has_guard = z3.Bool(f"guard_{s}")
            solver.add(z3.PbEq([(v,1) for v in acts.values()],1))
            solver.add(has_guard == z3.Or(list(cond.values())))
            solver.add(z3.Implies(z3.Not(enabled), z3.Not(has_guard)))
            if s:
                solver.add(z3.Implies(enabled, slots[-1][2]))
            if cond:
                solver.add(z3.PbLe([(v,1) for v in cond.values()],3))
            for a, v in acts.items():
                weight = char_count(f"/{'fcast' if samples.actions[a]['fcast'] else 'cast'} {samples.actions[a]['name']}")+1
                weights.append((z3.And(enabled,v),weight))
            weights.extend((v,char_count(samples.atoms[k])+1) for k,v in cond.items())
            weights.append((has_guard,2))
            variables.extend([enabled] + list(acts.values()) + list(cond.values()))
            slots.append((acts,cond,enabled))
        # Each enabled line pays its newline, including the last. This additive
        # metric differs by a fixed one character for every nonempty program.
        ceiling = char_count(render(window))+1
        solver.add(z3.Or([slot[2] for slot in slots]))
        solver.add(z3.PbLe(weights, ceiling-1))
        prefix = rules[:start]
        suffix = rules[start+len(window):]
        _, prefix_remaining = samples.selections(prefix)
        suffix_chosen, _ = samples.selections(suffix)
        suffix_masks = [(r["action"],bits) for r,bits in zip(suffix,suffix_chosen) if bits]
        # Earlier fixed rules already settle these observations correctly.
        # Do not re-encode their local slots, nor repeatedly recompute guards
        # for every row/window. Keep their cursor witnesses as constants.
        satisfied = {cursor for cursor,bits in samples.groups.items() if bits & ~prefix_remaining}
        hits_by_cursor, encoded = {}, {}
        for index, row in enumerate(samples.rows):
            samples.check()
            bit = 1 << index
            if not prefix_remaining & bit:
                continue
            suffix_hit = next((a for a,bits in suffix_masks if bits&bit), None)
            key = (tuple(bool(samples.truth[k]&bit) for k in pool),
                   tuple(bool(samples.executable[a]&bit) for a in action_ids),
                   tuple(row["allowed"]), row.get("wait_allowed",not row["allowed"]), suffix_hit)
            if key not in encoded:
                earlier = z3.BoolVal(False)
                for acts,cond,on in slots:
                    executable = z3.Or([v for a,v in acts.items() if samples.executable[a]&bit])
                    false_terms = [v for k,v in cond.items() if not samples.truth[k]&bit]
                    hit = z3.And(on,executable,z3.Not(z3.Or(false_terms)))
                    selected = z3.And(hit,z3.Not(earlier))
                    solver.add(z3.Implies(selected,z3.Or([v for a,v in acts.items() if a in row["allowed"]])))
                    earlier = z3.Or(earlier,hit)
                valid_suffix = suffix_hit is not None and suffix_hit in row["allowed"]
                if suffix_hit is not None and not valid_suffix:
                    solver.add(earlier)
                if row["allowed"] and not row.get("wait_allowed",False) and not valid_suffix:
                    solver.add(earlier)
                encoded[key] = z3.Or(earlier,z3.BoolVal(valid_suffix))
            if row["allowed"] and row.get("cursor",index) not in satisfied:
                hits_by_cursor.setdefault(row.get("cursor",index),[]).append(encoded[key])
        for hits in hits_by_cursor.values():
            solver.add(z3.Or(hits))
        # A few different models form a local beam; runtime rejects models
        # which changed scheduling. Their rows never become universal labels.
        for model_index in range(3):
            started = time.perf_counter()
            status = check_solver(solver)
            info = {"kind":"local_sat", "start":start, "model":model_index,
                    "slots":len(slots), "atoms":len(pool), "status":str(status),
                    "solve_ms":(time.perf_counter()-started)*1000}
            if status == z3.unknown:
                info["reason"] = solver.reason_unknown()
            diagnostic(info, solver.to_smt2())
            if status != z3.sat:
                break
            model = solver.model()
            replacement = []
            for acts,cond,on in slots:
                if z3.is_true(model.eval(on)):
                    a = next(a for a,v in acts.items() if z3.is_true(model.eval(v)))
                    replacement.append({"action":a,"atoms":[k for k,v in cond.items() if z3.is_true(model.eval(v))]})
            yield "local_sat_rewrite", clone(prefix) + replacement + clone(suffix)
            solver.add(z3.Or([v != model.eval(v,model_completion=True) for v in variables]))


def compress(rules, atoms, actions, baseline, render, verify, check,
             check_solver, accepted, progress, diagnostic, budget=lambda: True,
             feedback=None, deep_search=False):
    if not certified(baseline):
        raise ValueError("compression requires a full certified baseline")
    best, best_replay = clone(rules), baseline
    initial = char_count(render(best))
    seen, records = {render(best)}, []
    branches, branch_seen, feedback_cache = [], set(), {}
    feedback_source = None
    summary = {"status":"running", "initial_chars":initial, "best_chars":initial,
               "saved_chars":0, "trial_count":0, "accepted_count":0,
               "batch_trials":0, "max_batch_rules":0,
               "family_trials":0,
               "global_branches":0,
               "deep_search":deep_search,
               "scope":"single page; adaptive batches, native right-associated AND/OR guards, same-action subset rebuilds to 2/3 rules, global insertion rebuilds (24 candidates/pass), beam width 4/depth 2, SAT windows 3x3" + ("; experimental whole-program beam with up to 12 separate counterexample branches" if deep_search else "")}
    progress(dict(summary))

    def evaluate(kind, trial, allow_equal=False, batch_rules=0):
        nonlocal best,best_replay
        check()
        text = render(trial)
        cost = char_count(text)
        if text in seen or cost > summary["best_chars"] or (cost == summary["best_chars"] and not allow_equal):
            return None
        seen.add(text)
        if not budget():
            summary["status"] = "budget_exhausted"
            return None
        summary["trial_count"] += 1
        if kind.startswith('family_'):
            summary['family_trials'] += 1
        summary["method"] = kind
        summary['last_batch_rules'] = batch_rules
        if batch_rules:
            summary['batch_trials'] += 1
        progress(dict(summary))
        replay = verify(text, summary["trial_count"], kind)
        passed = certified(replay)
        records.append({"trial":summary["trial_count"],"kind":kind,"chars":cost,
                        "batch_rules":batch_rules,
                        "accepted":passed and cost < summary["best_chars"],
                        "comparison":replay.get("comparison"),"timings_ms":replay.get("timings_ms",{})})
        if not passed and feedback is not None and kind.startswith('global_') and feedback_source is not None:
            # Thin failure verdicts stay cheap. Observe full atom columns only
            # for whole-program candidates that supply a new search branch.
            # Cache only an identical complete aligned observation path,
            # including full exported states, target labels and eligibility.
            # Equal atom signatures alone never justify sharing trajectories.
            rows = replay.get('rows',[])
            observations = [{k:r.get(k) for k in ('state','last_skill','time','cursor',
                            'allowed','executable','wait_allowed','decision_latest','wait_next_time')}
                            for r in rows]
            # Compact native replies usually export only the last full state.
            # Missing earlier states cannot establish trajectory identity.
            observation_path = {'rows':observations,'comparison':replay.get('comparison'),
                                'probe_failure':replay.get('probe_failure'),
                                'actual_fingerprint':replay.get('actual_fingerprint')}
            cache_key = hashlib.sha256(json.dumps(observation_path,sort_keys=True,separators=(',',':')).encode()).digest() if rows and all(r.get('state') is not None for r in rows) else None
            reached = feedback_cache.get(cache_key) if cache_key is not None else None
            if reached is None:
                reached = feedback(text,replay,summary['trial_count'],kind)
                if cache_key is not None:
                    if len(feedback_cache) >= 8:
                        feedback_cache.pop(next(iter(feedback_cache)))
                    feedback_cache[cache_key] = reached
            proposal = branch_guide(reached,feedback_source,branch_seen)
            if proposal is not None:
                cursor,signature,guide = proposal
                if signature not in branch_seen:
                    branch_seen.add(signature)
                    branches.append((cursor,signature,guide))
                    branches.sort(key=lambda item:-item[0])
                    del branches[4:]
        if passed and cost < summary["best_chars"]:
            best,best_replay = clone(trial),replay
            summary.update(best_chars=cost,saved_chars=initial-cost,accepted_count=summary["accepted_count"]+1)
            summary['max_batch_rules'] = max(summary['max_batch_rules'],batch_rules)
            accepted(text,replay,dict(summary))
            progress(dict(summary))
        return replay if passed else None

    def ordered(candidates):
        unique = {}
        for kind,trial in candidates:
            check()
            text = render(trial)
            if text not in seen and char_count(text) < summary["best_chars"]:
                unique.setdefault(text,(char_count(text),text,kind,trial))
        return [(kind,trial) for _,_,kind,trial in sorted(unique.values(),key=lambda p:(p[0],p[1]))]

    def run_batch(kind, source, samples, patches):
        # Independent static edits are only a plan. Execute the whole plan
        # once; failed groups are split against the original stable row IDs.
        # Only passed subsets are retained; an unsuccessful remainder never
        # rolls back an accepted macro or borrows its certificate.
        if len(patches) < 2:
            return
        admitted, failures = {}, 0
        def visit(part):
            nonlocal failures
            check()
            if failures >= 12 or not part:
                return
            changed = dict(admitted)
            changed.update(part)
            trial = [changed.get(i,r) for i,r in enumerate(source) if changed.get(i,r) is not None]
            if not samples.compatible(trial):
                if len(part) > 1:
                    middle = len(part)//2
                    visit(part[:middle])
                    visit(part[middle:])
                return
            result = evaluate(kind,trial,batch_rules=len(part))
            if result:
                admitted.update(part)
                return
            failures += 1
            if len(part) > 1:
                middle = len(part)//2
                visit(part[:middle])
                visit(part[middle:])
        visit(patches)

    while budget():
        check()
        old_cost = summary["best_chars"]
        # This cheap first-match check routes expensive replays toward plausible
        # edits. Rejected sample shapes are not a language-level UNSAT claim.
        samples = Samples(best_replay["rows"],atoms,actions,check)
        run_batch('batch_basic',clone(best),samples,basic_batch(best,samples,actions))
        if summary['best_chars'] < old_cost:
            continue
        for kind,trial in ordered(simple_edits(best,actions,check)):
            if samples.compatible(trial):
                evaluate(kind,trial)
            if summary["best_chars"] < old_cost:
                break
        if summary["best_chars"] < old_cost:
            continue
        run_batch('batch_features',clone(best),samples,feature_batch(best,samples))
        if summary['best_chars'] < old_cost:
            continue
        for kind,trial in ordered(feature_edits(best,samples)):
            if samples.compatible(trial):
                evaluate(kind,trial)
                if summary["best_chars"] < old_cost:
                    break
        if summary["best_chars"] < old_cost:
            continue
        for kind,trial in ordered(or_edits(best,samples)):
            evaluate(kind,trial)
            if summary['best_chars'] < old_cost:
                break
        if summary['best_chars'] < old_cost:
            continue
        # A repeated action may need several priority regions. Rebuild a
        # donor subset into multiple guards together; intermediate programs
        # need not reproduce the target. Keep action/shape diversity instead
        # of letting one cheapest family consume every replay slot.
        for kind,trial in family_edits(best,samples,diagnostic):
            evaluate(kind,trial)
            if summary['best_chars'] < old_cost:
                break
        if summary['best_chars'] < old_cost:
            continue
        # Native guard rebuilding includes joint replacement and priority;
        # try the productive broader neighbourhood before local SAT/beam.
        for kind,trial in priority_edits(best,samples,native=True):
            evaluate(kind,trial)
            if summary['best_chars'] < old_cost:
                break
        if summary['best_chars'] < old_cost:
            continue
        # Cross-region placement, replacement and deletion form ONE candidate.
        # Its intermediate programs need not be valid. The static filter is
        # only guidance; every admitted result still gets native certification.
        for kind,trial in priority_edits(best,samples):
            evaluate(kind,trial)
            if summary['best_chars'] < old_cost:
                break
        if summary['best_chars'] < old_cost:
            continue
        for kind,trial in local_rewrites(best,samples,render,check_solver,diagnostic):
            evaluate(kind,trial)
            if summary["best_chars"] < old_cost:
                break
        if summary["best_chars"] < old_cost:
            continue
        # Equal-cost priority moves can unlock deletions. Retain a small beam
        # of certified alternative orders; only strictly shorter results replace
        # the user's best. All paths keep their OWN samples, never pooled.
        frontier = [(clone(best),best_replay)]
        for depth in range(2):
            next_frontier = []
            for base,base_replay in frontier:
                guide = Samples(base_replay["rows"],atoms,actions,check)
                for i in range(len(base)-1):
                    check()
                    trial = clone(base)
                    trial[i],trial[i+1] = trial[i+1],trial[i]
                    if guide.compatible(trial):
                        replay = evaluate("beam_priority_move",trial,allow_equal=True)
                        if replay:
                            next_frontier.append((trial,replay))
                    if len(next_frontier) >= 4:
                        break
                if len(next_frontier) >= 4:
                    break
            for base,base_replay in next_frontier:
                guide = Samples(base_replay["rows"],atoms,actions,check)
                proposals = (list(simple_edits(base,actions,check)) + list(or_edits(base,guide))
                             + list(feature_edits(base,guide)))
                for kind,trial in ordered(proposals):
                    if guide.compatible(trial):
                        evaluate("beam_"+kind,trial)
                        if summary["best_chars"] < old_cost:
                            break
                if summary["best_chars"] < old_cost:
                    break
            if summary["best_chars"] < old_cost or not next_frontier:
                break
            frontier = next_frontier
        if summary["best_chars"] == old_cost:
            if not deep_search:
                break
            feedback_source = samples
            branches.clear()
            for kind,trial in global_edits(best,samples,check_solver,diagnostic):
                evaluate(kind,trial)
                if summary['best_chars'] < old_cost:
                    break
            # Rebuild all guards/actions/priorities after each counterexample.
            # Each branch retains its own WAIT observations; a previous path
            # never becomes an unconditional constraint on the next path.
            for branch in range(12):
                if not branches or summary['best_chars'] < old_cost:
                    break
                cursor,_,guide = branches.pop(0)
                summary['global_branches'] += 1
                diagnostic({'kind':'global_counterexample_branch','branch':branch+1,
                            'cursor':cursor,'guide_rows':len(guide.rows)},None)
                progress(dict(summary))
                for kind,trial in global_edits(best,guide,check_solver,diagnostic):
                    evaluate(kind,trial)
                    if summary['best_chars'] < old_cost:
                        break
            if summary['best_chars'] == old_cost:
                break
    summary["status"] = "scope_exhausted" if budget() else "budget_exhausted"
    progress(dict(summary))
    return best,best_replay,records,summary
