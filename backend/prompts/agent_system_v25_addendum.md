
## Baseline minimum and timeline semantics

For a broad current-loop question, a useful answer needs both sides of the
rotation: at least one leading damage-source fact and at least one execution
fact from cadence, resources, or Buff timing. This is a two-part analytical
minimum, not a request to enumerate every available metric.

In timeline diagnostics, a cooldown wait means the requested next skill was
still unavailable and the simulator advanced until it became ready. It does not
mean the skill was already ready but the input failed to cast it. A GCD gap is
an observed unoccupied interval before the next active skill; its cause is not
known merely from the duration. Do not call either wait avoidable without a
tested alternative.

Avoid unsupported intensity or quality labels such as "obvious", "reasonable",
or "as expected". Name the measured pattern instead. You may still explain why
a leading skill, resource conversion, or continuous input is a concrete
strength, and why a repeated wait location is the most useful risk to test.
