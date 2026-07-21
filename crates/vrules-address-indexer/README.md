# vrules-address-indexer

`vrules-address-indexer` builds distributable US address indexes from
OpenAddresses-style CSV exports. The artifact is patchable: a full snapshot and
each later patch use the same `manifest.json` + `rows.jsonl` format, with
explicit `upsert` and `delete` operations.

This crate supports the address reference implementation used to prove generic
business-rule execution across hosts. It is not a core framework dependency or
an indication that vector-rules is an address-verification product.

Build a full snapshot:

```bash
cargo run -p vrules-address-indexer -- build \
  --input openaddresses/us/il/demo.csv \
  --source us/il/demo \
  --out dist/us-addresses-v1 \
  --generation 1
```

Build a patch against a previous artifact:

```bash
cargo run -p vrules-address-indexer -- patch \
  --base dist/us-addresses-v1 \
  --input openaddresses/us/il/demo.csv \
  --source us/il/demo \
  --out dist/us-addresses-v2-patch \
  --generation 2
```

Install an artifact into the native em-log-n fjall/usearch shard:

```bash
cargo run -p vrules-address-indexer -- install-native \
  --artifact dist/us-addresses-v1 \
  --db .local/us-addresses
```

Browser clients use `em-log-n-wasm` and IndexedDB:

```js
const shard = await openEmLogNBrowserShard({
  spec: {
    domain: "us-addresses",
    indexes: [{ name: "address", dim: 128, metric: "cosine" }],
  },
});

await shard.applyPatchJsonl(await fetch("/address-index/rows.jsonl").then((r) => r.text()));
const hits = shard.ann("address", queryVector, 10);
```

The vectorizer is `vrules-address-lexical-v1`: a deterministic lexical vector
over canonicalized address tokens and n-grams. It is not labeled as an
embedding. Model-backed vectors can be added as another index without changing
the snapshot/patch operation model; the reference host defaults to
EmbeddingGemma and accepts alternate GGUF embedding models.

The browser example consumes the artifact through IndexedDB. A DataFusion
adapter is the planned analytical counterpart, using the same address facts and
rules to demonstrate PWA/batch conformance.
