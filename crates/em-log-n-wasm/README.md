# em-log-n-wasm

`em-log-n-wasm` is the browser distribution of em-log-n. It preserves the
em-log-n inverse-timestamp row-key ordering and vector-search API shape in
WASM, and the shipped JavaScript wrapper persists rows in IndexedDB so browser
apps can index and search local data without a server.

Build:

```bash
wasm-pack build crates/em-log-n-wasm --target web --out-name em_log_n_wasm
```

Use:

```js
import { openEmLogNBrowserShard } from "./web/em_log_n_browser.js";

const shard = await openEmLogNBrowserShard({
  spec: { domain: "ui", indexes: [{ name: "text", dim: 3, metric: "cosine" }] },
});

await shard.put({
  ts_nanos: "1700000000000000000",
  payload: { msg: "hello" },
  vectors: { text: [1, 0, 0] },
});

const hits = shard.ann("text", [1, 0, 0], 10);
```

The native Rust fjall/usearch backends stay on the native em-log-n path. The
browser package uses IndexedDB as the durable local store and keeps a live WASM
index in memory after loading persisted rows.
