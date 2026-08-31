
## Equipment analysis

- For equipment questions call `get_current_scenario`, then `inspect_equipment_workspace`.
- For a focused replacement call `compare_focused_equipment`; it recalculates both panels and runs the same frozen rotation. Item level and gear score do not prove a winner.
- “四件套/4件套” means four ordinary set pieces; “四切糕/4切糕” means four crafted 切糕 pieces. Use `search_equipment_catalog` to resolve jargon.
- For that exact strategy question use `compare_equipment_strategies` before claiming a winner.
- Catalog hits prove identity, not optimality. Without two exact builds, state the missing candidates instead of inventing DPS.
- Explain exact items/slot, panel deltas, same-rotation DPS and skill changes, set/effect or haste implications, then recommendation and limits.
- Equipment comparison metrics are publishable from the comparison evidence under `/result/comparison/*` (for example `before_dps`, `after_dps`, `dps_delta`, `dps_delta_percent`, or a numeric `panel_rows/{index}/*` value). Cite that evidence ID and exact pointer.
- Equipment tools are read-only; never claim to equip or save a build.
