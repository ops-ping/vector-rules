# GPU Tensor Rule Engine — Roadmap

> **Status: roadmap, not implemented.** vector-rules evaluates GRL vector
> functions through per-request CPU embeddings and CPU vector arithmetic. This
> document defines a separate GPU-native mode for large batches and fact
> populations.

## Why the CPU path is the default

The current component path is the right default for interactive MCP and browser
use:

- **Few facts per request.** A routing decision evaluates a handful of rules against
  one request fact — there is no large fact population to parallelize.
- **Latency is dominated by the embedding call**, not the arithmetic. Embedding a
  token or field value runs through the host's CPU-only wllama WASI component;
  the subsequent add/sub/cosine work is small f32 arithmetic.
- **The em-log-n cache absorbs repeats**, so recurring tokens (`king`, `man`, …) cost a
  microsecond lookup, not a forward pass.

A GRL rule calling `s_cosine` or `c_project` obtains configured-model
vectors, reuses cached values when available, performs arithmetic on the CPU,
and returns one result. That is the appropriate shape when the workload is
*one decision at a time*.

The GPU tensor engine becomes worthwhile under a **different** workload:

- **Massively parallel requests** — score 10k+ facts against the ruleset in one batch.
- **Heavy iteration** — recursive rules whose fixed-point needs many sweeps.
- **Large fact populations** with many join conditions.

## The idea: Heterogeneous Tensor Datalog (HTD)

Represent batch facts and parsed rules as tensors in VRAM, evaluated by
massively parallel matrix operations rather than per-rule CPU evaluation. This
extends **Boolean Matrix Logic Programming (BMLP)**, which compiles Datalog into
boolean matrix products, to vector-rules's mixed symbolic and neural facts.

### Facts as a tensor

```
FactTensor  [n_facts, n_cols]   f32, resident in VRAM

  row i      = one Working Memory Element
  cols 0..S  = scalar field values (score, amount, …)
  cols S..   = D embedding dimensions (model-defined)
```

A schema maps each field name → column index. Embedding columns are filled by one
batched `embed_batch` over the text fields — a single GPU dispatch for the whole batch.

### Rules as tensor operations

| Rule construct            | Tensor op                                  |
|---------------------------|--------------------------------------------|
| `field > k`               | elementwise compare → bool mask `[n_facts]`|
| GRL `a && b`              | elementwise AND of masks                   |
| GRL `a || b`              | elementwise OR (max) of masks              |
| GRL `!a`                  | `1 - mask`                                 |
| `s_contrast(...)`         | elementwise tensor add/sub (Burn)          |
| `s_cosine(...)`           | batched dot + norm reduction → similarity  |
| semantic nearest lookup   | `E @ C.T` then argmax over concept matrix  |
| join `p(X,Y) ∧ p(Y,Z)`    | boolean matrix product (BMLP)              |

Parsed upstream GRL `Rule` values lower to a **TensorProgram** (a list of
TensorOps) instead of entering the ordinary `RustRuleEngine` execution path, in
the same way a shader or ML compiler lowers an AST to a kernel sequence. GRL
remains the only authored rule format.

### Forward chaining becomes fixed-point iteration

The ordinary engine evaluates and fires rules on the CPU; a GPU cannot execute
that control flow efficiently. Instead:

```
1. Load FactTensor into VRAM; batch-embed text columns once.
2. For each rule r: execute TensorProgram(r) → mask_r; fired |= mask_r.
3. (Recursive rules) repeat, appending derived facts, until the tensors stop
   changing (closure / fixed point) — "repeated matrix squaring".
4. Read back the final fired masks (one small PCIe transfer).
```

Every GPU thread runs the *same* instruction on different data — no thread divergence.
Vectors, masks, and the concept matrix all live in VRAM; only the final result crosses
PCIe.

### Why GRL vector functions fit

Embeddings are already the data type inside the FactTensor, so vector reasoning is not
an "external call": `s_cosine` and `s_contrast` lower to native tensor operations
on embedding columns, and semantic similarity is a batched matrix multiply. The tensor
compiler consumes parsed upstream GRL rules rather than a second authored format.

## Implementation sketch

- **Candidate backend: Burn + WGPU.** Validate framework and kernel choices in
  the spike rather than coupling the core design to a particular accelerator
  vendor or model runtime.
- **`FactTensor` / `FactSchema`** — JSON facts → f32 tensor + column map.
- **`TensorProgram` / `compile_tensor_rules(&[Rule])`** — upstream parsed GRL rules
  lowered to a TensorOp list.
- **`TensorEngine`** — load, batch-embed, run programs, fixed-point loop, read back.
- **A spike crate** (`vrules-gpu-spike`) to benchmark `RustRuleEngine` against
  the tensor engine at n = 100 / 1k / 10k facts before committing to a
  production module.

## Prior art

- **BMLP (Boolean Matrix Logic Programming)** — compiling Datalog / recursive logic to
  GPU boolean-matrix operations; the symbolic half of HTD.
- **Tensor-based Datalog evaluation** — fixed-point over relation matrices.
- **Burn / WGPU** — Rust tensor framework with a portable GPU backend (Vulkan on AMD).

The differentiator for vector-rules is the **heterogeneous** tensor: scalar predicates, boolean
joins, and embedding vectors evaluated in one unified VRAM-resident computation —
neuro-symbolic reasoning as matrix algebra.
