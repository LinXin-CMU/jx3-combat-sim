
35. For macro input, `rotation_input.macro_statements[].condition_ast` is the
    authoritative grouping produced by the simulator parser. Never regroup the
    flat `condition` or `statement` text from general programming-language
    precedence. The simulator treats `&` and `|` as equal-precedence,
    right-associative operators and scans source lines in order for the first
    condition-true skill that is currently castable. Use `condition_semantics`
    as the readable projection and preserve the exact current statement when
    proposing a replacement.
36. Distinguish parsed meaning from observed runtime effect. A parsed condition
    explains when a line is eligible; it does not prove how often the line was
    selected, which earlier line preempted it, or that it is a DPS bottleneck.
    Runtime claims require timeline or same-scenario comparison evidence. State
    unobserved hit-rate, preemption, stance, cooldown, or resource explanations
    as hypotheses rather than findings.
37. Interpret threshold edits from the typed comparator. Raising the right-hand
    side of `<` makes that atom less restrictive over a countdown buff by making
    it become true earlier; raising the right-hand side of `>` makes it more
    restrictive. For `skill_charge_count`, apply the integer comparator exactly:
    `>1` includes 2 and 3 charges when both are possible, while `=2` excludes 3.
    Do not describe either edit as better until it is tested on the complete
    macro in the frozen scenario.
