# Things That Are Bugs

A bug violates a required outcome or constraint or leaves risk unmanaged.

Categories may overlap to some degree.

## Under-implementation

Omitting required behavior or guarantees.

| Category | Failure |
| --- | --- |
| **Blind execution (under-checking)** | Skipping required validation where failure is possible. |
| **Fault abandonment** | Failing to recover, contain, escalate, or terminate appropriately after failure. |

## Over-permissiveness

Allowing forbidden behavior, states, or resource consumption.

| Category | Failure |
| --- | --- |
| **State inconsistency** | Leaving state contradictory, corrupted, or inconsistent across components. |
| **Fault propagation** | Letting failure spread beyond its permitted scope. |

## Incorrect Implementation

Using invalid code or realizing requirements incorrectly.

| Category | Failure |
| --- | --- |
| **Over-checking** | Rechecking guaranteed conditions without additional assurance. |
| **Incorrect logic** | Applying an incorrect condition, calculation, or decision rule. |
| **Invalid code structure** | Violating syntax, typing, or build constraints. |
| **Incorrect coordination** | Using incorrect ordering, synchronization, or communication between operations or components. |
