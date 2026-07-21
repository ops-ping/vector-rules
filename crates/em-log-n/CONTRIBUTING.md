# Contributing

Thank you for your interest in em-log-n. This project is small and early;
the bar for contributions is high signal, not high volume.

## Before you write code

Please open an issue first if your change touches any of the following.
None of these are gatekept by reviewers being grumpy — they exist to
save you wasted time on a PR that won't be merged.

- Anything in [docs/hard-constraints.md](docs/hard-constraints.md) — the
  latency-first, in-process, no-SQL-on-hot-path rules.
- Any new dependency. Every dep is pinned to an exact version; new ones
  need a rationale.
- The on-disk format (row key layout, protobuf schema, manifest format).
  These are versioned at the crate boundary and have backwards-compat
  obligations.
- The public API of `Shard`, `Store`, `Codec`, `Embedder`,
  `RetentionPolicy`, or `ObjectStore`. Stable surfaces aren't easily
  walked back.

For everything else — tests, docs, examples, fixes to comments,
performance improvements that don't move APIs — just open a PR.

## Style and policies

- **Pinned versions.** All deps in `Cargo.toml` use `=x.y.z`. A version
  bump is a deliberate code-review change, not a transitive update.
- **One `cargo build --release --features full` invariant.** The repo must
  build the tested backend stack with one cargo command (no ad hoc prep
  steps). `full` enables fjall, usearch, codecs, and cloud cold-tier; the
  default build stays light.
- **Tests are first-class.** New behaviour comes with a test. Hard
  invariants (key ordering, codec determinism, GC-race tolerance) get
  proptest property assertions, not just spot checks.
- **`#![deny(unsafe_code)]`** at the crate root. The one exception is
  the usearch custom-metric bridge in `src/shard.rs`, where a raw-
  pointer ABI is unavoidable; it is the only `unsafe` in the crate and
  is documented inline.
- **Documentation lives next to the change.** A module-level doc
  comment that explains *why* is more valuable than a wiki page no one
  reads.

## Workflow

1. Fork → branch → PR. Branch names prefixed with your GitHub handle.
2. Run `cargo test` locally. CI runs the same.
3. Commit messages: imperative subject ≤ 72 chars, blank line, body
   that explains the why. If the change reflects user feedback, include
   a `Co-authored-by:` trailer crediting the source.
4. Squash if asked.

## Code of conduct

Be kind. Assume good faith. If something feels off, say so.

## License

By contributing, you agree your contribution is licensed under
Apache-2.0, the same license as the project. See [LICENSE](LICENSE).
