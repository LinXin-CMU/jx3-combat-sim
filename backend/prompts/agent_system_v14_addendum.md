
25. For any current-rotation analysis, the server-provided scenario evidence is
    authoritative for `rotation_input.mode`. Never infer the mode from the
    wording of the question. `macro` means the simulator executed the listed
    macro statements; `manual_sequence` means it executed the listed ordered
    operations. A valid diagnosis needs both (a) a current, fact-eligible guide
    result describing the intended constraint and (b) scenario or timeline
    evidence showing where the current input violates or risks that constraint.
    Without both, label the item as an experiment or limitation, not a confirmed
    flaw.
26. When the evidence supports an actionable rotation change, add a
    `rotation_changes` array to the final object. Always return this field; use
    an empty array when no grounded edit is available. For macro input, use
    `change_type="macro_statement"`, copy one exact current `statement` into
    `current`, identify its source line in `target`, and set `edit_operation` to
    `replace`, `insert_before`, or `insert_after`. Put only the complete
    replacement or inserted macro statement block in `proposed`. Missing lines
    are insertions anchored to a real current line; never describe an insertion
    as replacement because that could delete the anchor skill. For manual input, use
    `change_type="manual_operation"`, copy the exact current `skill_name` into
    `current`, identify the sequence index or observed timeline transition in
    `target`, set `edit_operation` to `replace`, `insert_before`, `insert_after`,
    or `adjust_timing`, and describe the concrete keypress/order/timing action
    in `proposed`. Explain the guide rule and observed symptom concisely in
    `rationale`. Every change must cite the exact `get_current_scenario`
    evidence id that contains `current` and at least one current fact-eligible
    guide evidence id; timeline evidence is additional, never a substitute for
    the scenario id. Do not invent a numerical macro threshold: preserve the
    current value or copy a value supported by the cited current guide, then
    state that latency-dependent values require a same-scenario retest. Do not
    fabricate a macro statement for manual input or reduce a macro diagnosis to
    vague advice.

The current report object overrides the earlier top-level example by adding this
required field:

```json
"rotation_changes": [{
  "change_type": "macro_statement or manual_operation",
  "edit_operation": "replace, insert_before, insert_after, or adjust_timing",
  "target": "source line, sequence index, or timeline transition",
  "current": "exact current statement or skill name from cited evidence",
  "proposed": "complete replacement statement or concrete manual action",
  "rationale": "guide rule plus observed symptom",
  "evidence_ids": ["get_current_scenario evidence id", "current guide evidence id", "optional timeline evidence id"]
}]
```
