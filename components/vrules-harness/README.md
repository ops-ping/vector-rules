# vrules-harness

`vrules-harness` is a WASI component that acts as the execution harness for `vrules-core`.

## Responsibility

- Loads Git-governed GRL rule packs (`shared-rules/`) from the filesystem or Git repository history using the embedded `gix` crate.
- Instantiates the `vrules-core` RETE rule engine and canonical router.
- Executes forward-chaining rule evaluation with execution traces.
- Evaluates backward-chaining goals to generate verifiable mathematical proof trees.
- Manages Git branch evaluation, rule comparison, diffs, and signed-off fast-forward promotions.

## Interfaces

Exported WASI World: `plugin-component` (`wit/vrules.wit`)
- Exports interface `plugin` (`initialize`, `invoke`).
- Imports interface `host` (`invoke`, `get-embedding-info`, `embed`, `http`, `log`).

## Supported Operations

Invocations through `plugin.invoke(operation, payload)`:
- `evaluate`: Evaluates working facts against a loaded ruleset or Git revision.
- `prove`: Runs backward-chaining goal proof over the active ruleset.
- `branches`: Lists branches and commit revisions in the rules Git repository.
- `compare`: Compares rule sets between two Git revisions or candidate GRL text.
- `promote`: Fast-forwards the active rules revision after explicit sign-off.
