# Things That Are Bugs

A bug violates a required outcome or constraint, leaves risk unmanaged,
or adds validation without assurance. Constraints include security and
resource and latency budgets.

| Category | Failure |
| --- | --- |
| **Tautological logic (over-checking)** | Rechecking guaranteed conditions without additional assurance. |
| **Blind execution (under-checking)** | Skipping required validation where failure is possible. |
| **Logic under-implementation** | Omitting a required branch or outcome. |
| **Logic over-permissiveness** | Allowing forbidden inputs, actions, states, or outcomes. |
| **Fault abandonment** | Failing to recover, contain, escalate, or terminate appropriately after failure. |
| **Incorrect logic** | Applying an incorrect condition, calculation, or decision rule. |
| **State inconsistency** | Leaving state contradictory, corrupted, or inconsistent across components. |
| **Fault propagation** | Letting failure spread beyond its permitted scope. |
