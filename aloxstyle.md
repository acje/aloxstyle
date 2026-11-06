# AloxStyle

AloxStyle is a Rust style for unattended services without an async runtime. It
composes explicit ownership, bounded execution, and typed invariants to keep
services understandable and predictable.

## Priorities

Evaluate design tradeoffs in this order:

1. **Maintainability.** Favor simple designs and small, reversible, deployable
   changes.
2. **Correctness by design.** Encode invariants in types. Validate untrusted
   input at boundaries; trust validated values inward.
3. **Response times.** Bound work and keep progress, timers, and shutdown
   responsive to service requirements.
4. **Energy efficiency.** Minimize redundant polling, unnecessary allocation,
   and idle CPU or network activity.
5. **Features.** Build only capabilities justified by requirements using the
   simplest design that satisfies the required behavior, resource limits, and
   security constraints.

## Design deep modules

- Give each format, configuration rule, and state transition one owner. Expose
  meaningful operations over validated values.
- Put specialized logic and bookkeeping behind a small interface. Hide
  representations and algorithms; expose resource costs, failure behavior,
  policy, and caller obligations.
- Merge tightly coupled components and remove redundant forwarding layers.
  Preserve boundaries that enforce validation, authorization, ownership,
  supervision, or distinct responsibilities.
- Keep connection and DNS transitions and buffer layout inside their owning
  modules. Keep work budgets, progress, timers, and shutdown checks visible to
  the event loop.
- Generalize across related, current uses. Group inputs that travel together
  into cohesive request, configuration, or context types with enforced
  invariants. Preserve independent values as independent values.
- Where startup validation and preallocation are required, construct a validated
  runtime plan once. Preserve allocation limits and avoid unnecessary buffer
  copies when reorganizing state.

## Model valid states

Use enums, newtypes, private fields, and state machines to exclude real invalid
states. Separate untrusted input representations from validated domain values.
Represent independent flags as booleans and genuine optionality as `Option`.
Choose integer types that fit the domain and Rust APIs.

For every changed invariant-bearing type:

1. State the invariant, caller boundary, and module visibility.
2. Inventory all construction and mutation routes: public fields and struct
   literals, constructors and builders, `Default`, `From`, `TryFrom`,
   deserialization, setters, mutable references, and `DerefMut`. Record absent
   routes explicitly.
3. Show how each route preserves the invariant, or provide a constructible
   counterexample and remedy. Include code inside the defining module, where
   private fields remain accessible.
4. Record valid and invalid boundary cases, an accept/reject verdict, and named
   compile-fail or runtime tests. Distinguish inspected tests from executed
   tests and identify evidence gaps.

Validate the parser and domain model as well as the types enforcing them.

## Give each decision one owner

Express each decision in one place. Replace invalid sentinel states with valid
domain types before simplifying branches. Keep a decision together instead of
splitting it between leading return guards and a `match` over the same cases.

Use guards for genuine preconditions and early short-circuits. Use `?`,
`let else`, and loop `continue` where they make control flow clearer.

## Make failure explicit

- Preserve meaningful `Option`, `Result`, and error distinctions. Empty results
  represent successful queries with no matches; idempotent success represents
  an operation whose required outcome is already established. Keep malformed
  input, operating failures, capacity exhaustion, and unknown outcomes explicit.
- Keep public error enums closed: omit `#[non_exhaustive]` and keep variant sets
  complete within a major semver line.
- Handle malformed external input and expected operating failures as errors.
  Use fail-stop behavior for unexpected panics. Reusing owner state after a
  caught panic requires a demonstrated recovery contract.
- Deny production `unwrap()`. Permit `expect()` only for a construction-proven
  invariant with a justified per-site lint expectation.
- Encode structural facts in types or compile-time assertions first. Keep input
  validation and correctness checks active in release builds. Reserve
  `debug_assert!` for removable diagnostics and measure hot-path assertion costs.

## Bound execution

Use explicit scheduling without an async runtime. The application owns
scheduling, fairness, timer progress, and shutdown. Give tunnel and DNS state a
single owner and explicit transitions.

- Work in bounded increments between reachable timer and shutdown checks.
  Apply the same discipline to synchronous helpers and drain loops.
- Assess aggregate consumption separately from per-item or per-visit limits.
- Bound recursive work and depth wherever execution guarantees depend on them.
- Preserve fair progress across active work. Evaluate batching and hot-path
  optimizations against measured or modeled work and latency budgets.
- Distinguish readiness and no-progress outcomes from EOF, protocol errors,
  TLS errors, and capacity rejection.
- Reject work when fixed capacity is exhausted. Keep retries within the same
  storage limits and validation rules.

Record each resource budget in the owning design or configuration contract:

| Field | Record |
| --- | --- |
| Owner | Component responsible for enforcement. |
| Scope | Resource, execution phase, and local or aggregate coverage. |
| Units | Items, bytes, operations per visit, or time. |
| Limit | Workload-justified maximum or deadline. |
| Exhaustion | Rejection, yielding, or timeout behavior at the limit. |
| Evidence | Named test or measurement of enforcement and exhaustion behavior. |

## Budget memory across lifecycles

Plan storage alongside work limits. When the service contract requires startup
preallocation, reserve fixed connection, buffer, and cache capacity at startup.
Account separately for future socket creation and kernel resource exhaustion.

For each allocation or deallocation restriction, specify:

- The execution phase and first-party scope.
- Dependency behavior and authorized exceptions.
- Ownership and retained capacity across initialization, serving, failure,
  reconnect, teardown, and final shutdown.

Define an exception across its complete lifecycle, including caller-owned
allocations. Preserve resource and security contracts when merging helpers or
removing wrappers. Revalidate affected evidence after dependency changes and
resolve implementation deviations through correction or explicit approval.

## Bound observability

Keep logging and metrics within the service's work, latency, and allocation
budgets, including during overload and sink failure.

- Bound event size, emission rate, buffering, and metric label cardinality.
  Use stable labels rather than per-request or per-connection identifiers.
- Rate-limit or aggregate repeated diagnostics. Account for suppressed events
  with bounded counters.
- Define behavior for full buffers and unavailable sinks. Keep emission and
  flush work within timer and shutdown budgets, including final shutdown.
- Respect phase-specific allocation restrictions in formatting, aggregation,
  and export. Verify worst-case telemetry load alongside serving work.

## Improve designs deliberately

Before structural refactoring, establish passing tests for affected observable
behavior. Add missing characterization or regression coverage first. Prefer
contract-level assertions that survive changes to private structure.

Before a major rewrite, compare at least two materially different designs.
Assess caller knowledge, information leakage, failure paths, ownership,
per-visit work, and allocation costs. Record the choice and rationale in a
design decision, commit message, or issue.

Keep improvements relevant, small, reversible, and separate from behavior
changes. Track larger work for later.

## Explain through structure and contracts

Make Rust code understandable through names, types, and structure. Restructure
code that needs explanatory commentary. Keep rationale in design decisions,
commit messages, or issues.

Use doc comments only for code contracts: public APIs, unsafe functions and
traits, and in-scope doctests. Include `# Errors`, `# Panics`, and `# Safety`
where required; unsafe functions and traits require `# Safety`. Follow the
project's restrictions on unsafe code.

Plain line and block comments are prohibited, including safety notes, TODOs,
lint justifications, and commented-out code.

## Match verification to the claim

- Start with type-level exclusion and deterministic tests. Test valid and
  invalid boundaries and the affected interactions among state transitions,
  capacity, work budgets, partial I/O, stalled peers, fairness, timers, and
  shutdown.
- Add fuzzing, simulation, model checking, or independent oracles to close a
  specific evidence gap.
- Run focused checks during each small change. Verify changed components and
  affected dependents before claiming completion; use broader verification for
  release claims.
- Preserve required allocation evidence. Report exercised paths, measured
  threads and phases, and dependency attribution. Global allocator counts
  combine first-party and dependency costs.
- Limit conclusions to the evidence: measured runs establish observed behavior;
  exhaustive checks establish properties within their stated assumptions.
  Establish aggregate memory bounds separately from local allocation counts.

## Choose dependencies and tools deliberately

Evaluate dependencies by total maintenance risk: implementation complexity,
supply-chain exposure, allocation behavior, and update cost. Include the cost
of maintaining equivalent code in-house. Honor project dependency restrictions.

Use the project's formatter and lint configuration. Give lint exceptions
specific reasons and preserve coherent logic when satisfying size limits.
