---
name: add-talent
description: Add or update a JX3 talent (奇穴) for a specific version and mount, including talent data, granted skills, buffs, recipes, Rust behavior, UI loading, and selected/unselected tests. Use for concrete talent implementation work.
---

# Add a talent

Talents are defined per version and mount. Resolve both before editing, then inspect adjacent talents in the same tier and search all existing references to the proposed name and ID.

## Model the talent

Confirm its tier and slot, name/icon/description, whether it grants a skill, applies a constant buff, injects a hidden recipe, changes an existing skill, or adds event-driven behavior. Translate the tooltip into explicit simulator rules and record any ambiguity as an assumption.

## Implement

- Follow the target mount's current talent TOML location and schema under `backend/data/{version}/{mount}/`.
- Map simple declared effects through the existing talent effect structures.
- If the talent grants a skill, implement that skill using the repository's normal versioned skill data and script path; do not create a UI-only placeholder.
- If it applies a buff or recipe, use the established target-version registry and ID conventions.
- Put conditional mechanics in `backend/src/scripts/v{version}/` near the affected skill or event hook. Prefer one authoritative implementation over checks duplicated across UI and backend.
- Keep the frontend data-driven. Static HTML edits are exceptional.
- Verify that the unselected path retains baseline behavior and that no other version or mount inherits the talent accidentally.

## Verify

- Test selected and unselected behavior, exact tier/slot selection, granted skill visibility/legality, and every affected damage, cooldown, resource, or buff outcome.
- Test conflicts or replacements with other talents in the same tier when relevant.
- Run `cargo test` from `backend`; run `node --check frontend/app.js` if frontend code changed.
- Report the target version/mount, translated rules, dependent skills/buffs/recipes, assumptions, and test results.

