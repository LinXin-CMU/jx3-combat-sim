---
name: add-skill
description: Add or update a JX3 combat skill in this repository, including versioned TOML data, Rust behavior scripts, registration, UI grouping, and regression checks. Use when implementing a skill, active ability, channel, stance switch, or skill-specific combat behavior; do not use for analysis-only requests.
---

# Add a skill

Treat each game version and mount as an independent ruleset. Determine the target `version` and `mount` from the request or the currently selected project context before editing. If either choice would materially change the result and cannot be inferred, ask for it.

## Inspect before editing

1. Read an analogous skill under `backend/data/{version}/{mount}/skills/`.
2. Read the target mount's `school.toml`, especially `[ui.skill_groups]`.
3. Search `backend/src/scripts/v{version}/skills/` for related mechanics and registration patterns.
4. Confirm the skill ID, display name, icon ID, weapon or stance restriction, damage kind and coefficients, cooldown/GCD/haste behavior, rage cost or gain, charges, channel or cast time, talent interactions, recipes, and combo state.

Do not copy coefficients or behavior across versions without evidence from the target version.

## Implement

- Put declarative data in `backend/data/{version}/{mount}/skills/{id}_{name}.toml`, following the target version's schema.
- Put non-trivial behavior in one focused Rust module under `backend/src/scripts/v{version}/skills/` and register it in the local `mod.rs` and dispatcher used by that version.
- Keep the deterministic simulator authoritative. Route casts through the existing skill-cast path so cooldown, GCD, resource, buffs, events, and damage remain consistent.
- If the skill interrupts or changes an ongoing state, emit the same event/state transitions as analogous skills. Do not bypass the event system.
- Add the skill to `school.toml` UI groups when it should be directly selectable. The frontend is data-driven; do not hard-code a new button in HTML unless the existing architecture truly requires it.
- Reuse existing icon and naming conventions. Never reuse an existing skill ID for a different mechanic.
- Preserve other versions and mounts. Shared helpers are appropriate only when their semantics are genuinely identical.

## Verify

- Add or update focused Rust tests for cast legality, resource/cooldown changes, damage or buff outcome, and version isolation.
- Run `cargo test` from `backend`.
- Run `node --check frontend/app.js` if frontend code changed.
- When a deterministic output changes intentionally, update the relevant regression fingerprint or golden expectation and document why.
- Report the target version/mount, files changed, rules implemented, assumptions, and tests run.

