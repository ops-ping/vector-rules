# vrules-wasm

`vrules-wasm` exposes the canonical `vrules-core` GRL evaluator to browser
applications. The binding parses rules with `Ruleset`, builds
`rust_rule_engine::RustRuleEngine`, and evaluates facts with `RuleEvaluator`.

The browser has no stdio MCP transport, so host-backed MCP functions are not
available. Pure GRL, canonicalization functions, vector functions, backward
proof, address standardization, and reference-index functions run in WASM.

## Generic API

- `validate_rule(grl) -> { ok, errors: [{ path, message }] }`
- `prove(grl_rules, query, facts_json) -> { provable, bindings, missing_facts, proof }`
- `class RuleEngine`
  - `new()`
  - `register_rule(grl_source)` appends one GRL rule or source string
  - `register_canon_pattern(label, kind, threshold, examples_json)`
  - `rule_count()`
  - `evaluate(fact_type, fact_data_json, want_trace)`
  - `set_embedding(text, vector, model_name, model_sha256, dimensions)`
  - `clear_embeddings()`

`evaluate` returns `{ fired, decision, facts, trace }`. It creates an empty
`Decision` object for generic decisions, and GRL `then` actions write effects
directly into the returned facts.

`register_canon_pattern` configures the router used by `s_canon_match`,
`b_canon_matches`, and `m_canon_label` during evaluation. `kind` is `auto`, `log`, or `json`;
`threshold` is a Hamming distance from 0 through 64; and `examples_json` is a
JSON array containing only strings. Registered patterns remain available for
subsequent evaluations by that `RuleEngine`.

Vector evaluation accepts real vectors supplied by the browser host.
`set_embedding` requires the real model name, a 64-character SHA-256 model
revision, and the model dimensions. It rejects inconsistent metadata,
dimension mismatches, non-finite vectors, and all-zero placeholder vectors.
Vector functions are registered only when a validated embedder is present.
The upstream evaluator reports missing vector functions, embedding failures,
and missing prefetched vectors directly.

## Address reference API

- `address_standardize(mode, input_json, index_json)`
- `verify_address(mode, input_json, grl, index_json, reference_json)`

`address_standardize` performs policy-neutral canonicalization and optional
local-index matching. `verify_address` evaluates an `AddressDecision` fact and
reads the post-action object from the evaluator's returned facts. Address and
reference index lookups are ordinary `RustRuleEngine` custom functions:

- `c_addr_index_score`, `b_addr_index_match`, `m_addr_index_match_id`
- `c_ref_match_count`, `m_ref_match_name`, `m_ref_match_id`
- `c_ref_exact_score`, `c_ref_lexical_score`, `c_ref_match_score`, `b_ref_match`

## Sequential browser inputs

The WASM package provides one-input/one-output `RuleEngine` evaluation. A
browser can feed records sequentially, but the package does not claim stream
windows or persistent stream state.

## Build

From the workspace root:

```sh
cargo install wasm-pack --locked
wasm-pack build crates/vrules-wasm --target web --out-name vrules_wasm
cargo test -p vrules-wasm
```

The console under `apps/console/` runs the WASM build from its `npm run build`
and `npm run dev` scripts.

## License

MIT OR Apache-2.0.
