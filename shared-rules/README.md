# vrules shared rules

Reusable Git-governed GRL policy packs for vrules.

```text
vrules-rules.toml        # product pack definitions
proxy/*.grl              # routing, classification, exposure, runtime policy
address/*.grl            # reference address workflow
shared/*.grl             # cross-product rules
proxy/tools.json         # tool metadata, not rule semantics
schemas/facts.json       # shared transport-neutral fact shapes
```

GRL is the only authored rule format. Fact-type targeting comes from
`Type.field` references, functions use normal GRL calls, and effects are
explicit `then` actions:

```grl
rule "ExposeReasoningRules" no-loop {
    when
        Exposure.context == "reasoning" &&
        Exposure.tool_group == "rules"
    then
        Decision.exposed = true;
}
```

The same files load into native, WASI, browser, batch, and stream hosts through
rust-rule-engine's parser and rule model. JSON remains the transport format for
facts, results, schemas, and tool metadata.

## Governance

Rules remain ordinary reviewed source files. The rules component can load the
working tree or a Git revision, validate GRL, compare rule names and content,
evaluate candidate facts, and promote a fast-forward revision after explicit
sign-off.

LLMs can draft rules, examples, tests, explanations, and replay facts. Git
review, validation, deterministic evaluation, and append-only audit records
remain the control points.

## Packs

`proxy/exposure.grl` evaluates one `Exposure` fact per candidate tool. A firing
rule sets `Decision.exposed`.

The address pack is a reference business workflow rather than framework
semantics. It combines native standardization, local indexes, reference
evidence, vector field detection, and organizational rules. King Cola bill-to
addresses are invalid; the Tristate Cola rule carries the 111 East Cola Lane
policy exception.

## License

MIT OR Apache-2.0.
