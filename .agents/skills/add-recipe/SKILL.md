---
name: add-recipe
description: Add or update a JX3 recipe or secret-manual modifier in this repository, including versioned recipe data, runtime behavior, skill filtering, UI exposure, and regression tests. Use when implementing a concrete recipe effect.
---

# Add a recipe

Recipes are version-scoped and often skill-scoped. Determine the target `version`, affected skill IDs, selectable/hidden status, and whether the effect is declarative or requires runtime behavior.

## Inspect the current schema

1. Read analogous entries in `backend/data/{version}/recipes.toml`.
2. Search for the affected skill and recipe field names in the target version's Rust scripts.
3. Inspect how the frontend loads and groups recipes before changing UI code.

Do not infer the schema from another version when the target version already has examples.

## Implement

- Add the recipe to `backend/data/{version}/recipes.toml` using the existing key, ID, level, quality, skill-filter, and effect conventions.
- Keep selectable recipes distinct from hidden/runtime-only modifiers according to current data patterns.
- Verify every numeric unit against consumers: damage percent, cooldown frames, duration ticks, resource amount, proc chance, and stack changes are not interchangeable.
- Express simple supported modifiers in data. Put conditional or stateful behavior in the target version's skill/buff scripts or effective-value helpers.
- Ensure recipe selection is unique and deterministic; follow existing set and ordering behavior rather than introducing parallel state.
- The UI is data-driven. Do not hard-code recipe controls unless the existing loader cannot represent the new rule.
- Do not alter other versions unless the requested recipe truly exists there too.

## Verify

- Test the affected skill with and without the recipe and assert the exact changed property.
- Test that unrelated skills and versions remain unchanged.
- Run `cargo test` from `backend`; run `node --check frontend/app.js` if frontend code changed.
- Report the affected skills, units, data/runtime split, assumptions, and test results.

