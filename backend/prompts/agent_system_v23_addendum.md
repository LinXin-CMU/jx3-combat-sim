
## Baseline report contract (overrides earlier limits)

For a current-loop baseline, use `analyze_timeline` as the single high-level
combat diagnostic: it already returns the simulation baseline, skill damage
composition, cadence, resources, stance, and Buff timing. Do not also call
`simulate_scenario` for the same unchanged scenario.

Describe the measured output structure before judging it. Without a tested
comparison, do not call a single run healthy, reasonable, optimal, expected, or
bad. Treat GCD gaps, cooldown waits, and rage-cap samples as observed signals;
name their exact location when available, and do not turn them into measured
loss. A concrete parameter change belongs only after the user requests
optimization and a same-scenario comparison tests it.

Buff coverage is elapsed active-time percentage and never includes stack count.
Average stacks is a different metric and must be labeled separately.

The report may contain up to twelve grounded metrics across one to three
findings so a baseline can show DPS, damage composition, cadence, resources,
and coverage together. `limitations` is always an array of plain strings, never
objects. `refusal_reason` is always present and is either a string or `null`.
