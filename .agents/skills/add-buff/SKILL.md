---
name: add-buff
description: Add or update a buff, debuff, periodic effect, team aura, or stackable combat state in this repository with correct versioning, registration, units, hooks, and tests. Use for concrete buff implementation work, not for general design discussion.
---

# Add a buff or debuff

Resolve the target game `version` and `mount` first. Inspect analogous effects in that exact ruleset before choosing a data structure or ID.

## Specify the effect

Confirm the ID, name, icon, duration, tick interval, maximum stacks, refresh/replace rules, dispel/debuff status, timeline visibility, source skills, talent or recipe dependencies, attribute deltas, and lifecycle hooks. Distinguish personal buffs, target debuffs, and team-wide effects.

## Implement safely

- Search current definitions and registries before assigning an ID. Follow the target version's established ID segment; never assume the next free value.
- Put shared engine primitives in shared modules only when every supported version has identical semantics.
- Put version-specific definitions and behavior under `backend/src/scripts/v{version}/`, following the local `buffs`, `defs`, or `team_buffs` layout.
- Register every new definition through the target version's existing registry. An unregistered definition is incomplete.
- Use the existing `AttribField` and effect structures. Verify the expected unit for every value—flat amount, percent, basis points, frames, milliseconds, or ticks—against nearby code.
- Apply reversible stat changes as deltas and ensure removal exactly undoes application. Avoid accumulating permanent drift on refresh or stack changes.
- Use established add, refresh, stack, tick, expire, and remove hooks. Keep deterministic ordering and event emission intact.
- Add player setters or effective-value helpers only if runtime state needs them; avoid duplicating derived-stat logic.
- Preserve unrelated versions and mounts.

## Verify

- Test application, refresh, stack cap, expiry/removal, periodic ticks, and the relevant damage/stat consequence.
- Test both enabled and disabled talent/recipe paths where applicable.
- Run `cargo test` from `backend`; run frontend syntax checks only if frontend code changed.
- Report IDs, units, lifecycle semantics, registration points, assumptions, and test results.

