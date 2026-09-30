"""Native right-associated macro conditions and bounded short-guard search.

Every emitted chain is legal macro text: ``A&B|C`` means ``A&(B|C)``.
Truth masks guide candidates on one current trajectory; they neither prove
runtime equivalence nor replace the caller's independent native replay.
"""


def normalize(rule):
    """Return a copied atoms/ops condition; explicit ops take precedence."""
    atoms = list(rule.get("atoms", []))
    if "ops" in rule:
        ops = list(rule["ops"])
    else:
        tail = list(rule.get("any_atoms", []))
        ops = ["&"] * max(0, len(atoms) - 1)
        if tail:
            if atoms:
                ops.append("&")
            atoms.extend(tail)
            ops.extend(["|"] * (len(tail) - 1))
    if len(ops) != max(0, len(atoms) - 1) or any(op not in ("&", "|") for op in ops):
        raise ValueError("condition ops must contain one native & or | between each leaf")
    return {"atoms": atoms, "ops": ops}


def condition_key(rule):
    condition = normalize(rule)
    return tuple(condition["atoms"]), tuple(condition["ops"])


def is_and(rule):
    return all(op == "&" for op in normalize(rule)["ops"])


def condition_mask(rule, truth, all_mask):
    condition = normalize(rule)
    atoms, ops = condition["atoms"], condition["ops"]
    if not atoms:
        return all_mask
    mask = truth[atoms[-1]] & all_mask
    for index in range(len(ops) - 1, -1, -1):
        leaf = truth[atoms[index]] & all_mask
        mask = leaf & mask if ops[index] == "&" else leaf | mask
    return mask & all_mask


def condition_cost(rule, atom_costs):
    """Additional UTF-16 characters for a rendered ``[condition] ``."""
    condition = normalize(rule)
    if not condition["atoms"]:
        return 0
    return 3 + sum(atom_costs[index] for index in condition["atoms"]) + len(condition["ops"])


def condition_text(rule, atomtexts):
    """Render the exact native chain, without invented grouping parentheses."""
    condition = normalize(rule)
    atoms, ops = condition["atoms"], condition["ops"]
    if not atoms:
        return ""
    return atomtexts[atoms[0]] + "".join(op + atomtexts[index] for op, index in zip(ops, atoms[1:]))


def short_guards(positives, negatives, samples, max_terms=8, max_candidates=8,
                 branch_limit=24, max_states=384, max_atoms=256):
    """Find short legal chains covering P and avoiding N on current samples.

    AND prefixes must cover all remaining positives and shrink negatives.
    OR prefixes must avoid all remaining negatives and shrink positives.
    Iterative deepening, memoized residuals, balanced operator branches, and
    small beams bound work. Limits never constitute global UNSAT.
    Same truth signatures retain short stable leaves and distinct clocks;
    whole chains are deduplicated by truth AND their clock-leaf signature.
    Required sample fields: all, truth, atom_costs, check; atoms is optional.
    """
    samples.check()
    positives &= samples.all
    negatives &= samples.all
    if not positives or positives & negatives or min(max_terms, max_candidates, branch_limit, max_states, max_atoms) <= 0:
        return []
    truth, costs, all_mask = samples.truth, samples.atom_costs, samples.all
    texts = getattr(samples, "atoms", None)
    # A Samples instance owns one immutable truth matrix. New trajectories or
    # atom catalogs must create/replace their arrays, invalidating this stamp.
    stamp = (id(truth), len(truth), id(costs), len(costs), id(texts), all_mask)
    cache = getattr(samples, "_native_condition_cache", None)
    if cache is None or cache["stamp"] != stamp:
        clock = set()
        if texts is not None:
            for index, text in enumerate(texts):
                samples.check()
                if "bufftime:" in text:
                    clock.add(index)
        signatures = {}
        for index, mask in enumerate(truth):
            samples.check()
            signatures.setdefault(mask & all_mask, []).append(index)
        pool = []
        for indices in signatures.values():
            samples.check()
            ordered = sorted(indices, key=lambda index: (costs[index], index))
            stable = [index for index in ordered if index not in clock]
            timed = [index for index in ordered if index in clock]
            pool.extend(stable[:1] + timed[:2])
        cache = {"stamp": stamp, "sources": (truth, costs, texts), "clock": clock, "pool": pool, "queries": {}}
        try:
            samples._native_condition_cache = cache
        except AttributeError:
            pass
    clock, pool = cache["clock"], cache["pool"]
    query = (positives, negatives, max_terms, max_candidates, branch_limit, max_states, max_atoms)
    if query in cache["queries"]:
        return [{"atoms": list(atoms), "ops": list(ops)} for atoms, ops in cache["queries"][query]]

    if len(pool) > max_atoms:
        # Include cheap inner suffix leaves even when they cannot be the root.
        # Separating features and legal root candidates provide the remainder.
        quarter = max(1, max_atoms // 4)
        ranked = [
            sorted(pool, key=lambda index: (costs[index], index))[:quarter],
            sorted(pool, key=lambda index: (-((positives & truth[index]).bit_count()
                                             + (negatives & ~truth[index]).bit_count()) / (costs[index] + 1),
                                            costs[index], index)),
            sorted((index for index in pool if not positives & ~truth[index]),
                   key=lambda index: (-(negatives & ~truth[index]).bit_count() / (costs[index] + 1), costs[index], index)),
            sorted((index for index in pool if not negatives & truth[index]),
                   key=lambda index: (-(positives & truth[index]).bit_count() / (costs[index] + 1), costs[index], index)),
        ]
        selected = set()
        # Round-robin avoids spending the variable budget on one operator.
        for offset in range(max(len(items) for items in ranked)):
            samples.check()
            for items in ranked:
                if offset < len(items):
                    selected.add(items[offset])
                    if len(selected) >= max_atoms:
                        break
            if len(selected) >= max_atoms:
                break
        pool = sorted(selected, key=lambda index: (costs[index], index))
    else:
        pool.sort(key=lambda index: (costs[index], index))

    memo, roots_cache, states, budget_misses = {}, {}, 0, 0
    iteration_ceiling = max_states

    def prune(guards):
        unique = {}
        for atoms, ops in guards:
            samples.check()
            rule = {"atoms": atoms, "ops": ops}
            mask = condition_mask(rule, truth, all_mask)
            wake = tuple(sorted({index for index in atoms if index in clock}))
            semantic = mask, wake
            entry = (condition_cost(rule, costs), tuple(atoms), tuple(ops), mask, wake)
            if semantic not in unique or entry[:3] < unique[semantic][:3]:
                unique[semantic] = entry
        ranked = sorted(unique.values(), key=lambda entry: entry[:3])
        chosen = ranked[:max_candidates]
        # Keep a clock alternative of the cheapest semantic mask where one
        # exists, even when many cheaper stable expressions fill the beam.
        if chosen and max_candidates > 1:
            first = chosen[0]
            alternate = next((entry for entry in ranked if entry[3] == first[3]
                              and entry[4] != first[4]), None)
            if alternate is not None and alternate not in chosen:
                chosen[-1] = alternate
                chosen.sort(key=lambda entry: entry[:3])
        return [(entry[1], entry[2]) for entry in chosen]

    def roots(p, n):
        samples.check()
        key = p, n
        if key in roots_cache:
            return roots_cache[key]
        leaves = [((), ())] if not n else []
        and_roots, or_roots = [], []
        for index in pool:
            samples.check()
            mask = truth[index] & all_mask
            missing = p & ~mask
            intersecting = n & mask
            if not missing and not intersecting:
                leaves.append(((index,), ()))
            if not missing and intersecting != n:
                and_roots.append((-(n & ~mask).bit_count() / (costs[index] + 1), costs[index], index, intersecting))
            if not intersecting and missing and missing != p:
                or_roots.append((-(p & mask).bit_count() / (costs[index] + 1), costs[index], index, missing))
        ordered_and, ordered_or = sorted(and_roots), sorted(or_roots)
        branches = []
        for offset in range(max(len(ordered_and), len(ordered_or))):
            samples.check()
            for operator, options in (("&", ordered_and), ("|", ordered_or)):
                if offset < len(options):
                    branches.append((operator, options[offset]))
                    if len(branches) >= branch_limit:
                        break
            if len(branches) >= branch_limit:
                break
        result = prune(leaves), branches
        roots_cache[key] = result
        return result

    def search(p, n, depth, ceiling):
        nonlocal states, budget_misses
        samples.check()
        key = p, n, depth
        if key in memo:
            return memo[key]
        if states >= ceiling:
            budget_misses += 1
            return []
        states += 1
        misses_before = budget_misses
        leaves, branches = roots(p, n)
        guards = list(leaves)
        if depth > 1:
            # Each operator receives a share, so native A|B&C is reachable
            # even when AND branches are attractive at another residual.
            # Give distinct root choices enough cells for a short chain before
            # allowing the first subtree to consume the entire depth pass.
            fair_width = min(len(branches), max(1, (ceiling - states) // (depth - 1)))
            if fair_width < len(branches):
                budget_misses += 1
            active = branches[:fair_width]
            for position, (operator, (_, _, index, residual)) in enumerate(active):
                samples.check()
                share = max(1, (ceiling - states) // (len(active) - position))
                child_ceiling = min(ceiling, states + share)
                suffixes = (search(p, residual, depth - 1, child_ceiling) if operator == "&"
                            else search(residual, n, depth - 1, child_ceiling))
                for suffix_atoms, suffix_ops in suffixes:
                    samples.check()
                    if index in suffix_atoms:
                        continue
                    if not suffix_atoms:
                        # AND true is the root leaf; OR true is unconditional.
                        guards.append(((index,), ()) if operator == "&" else ((), ()))
                    else:
                        guards.append(((index,) + suffix_atoms, (operator,) + suffix_ops))
        result = prune(guards)
        # A depth pass may hit its reserved budget. Do not cache that partial
        # exploration as complete and block a later, better-funded pass.
        if misses_before == budget_misses:
            memo[key] = result
        return result

    result = []
    best = []
    for depth in range(1, max_terms + 1):
        samples.check()
        if states >= max_states:
            break
        # Shallow 3/4-leaf solutions are explored before long-chain guesses.
        # Unused capacity rolls forward, while later depths retain a share.
        reserve = max(1, (max_states - states) // (max_terms - depth + 1))
        iteration_ceiling = min(max_states, states + reserve)
        best = prune(best + search(positives, negatives, depth, iteration_ceiling))
    for atoms, ops in best:
        samples.check()
        rule = {"atoms": list(atoms), "ops": list(ops)}
        mask = condition_mask(rule, truth, all_mask)
        if not positives & ~mask and not negatives & mask:
            result.append(rule)
    if len(cache["queries"]) >= 256:
        cache["queries"].pop(next(iter(cache["queries"])))
    cache["queries"][query] = [condition_key(rule) for rule in result]
    return result
