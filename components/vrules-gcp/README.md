# vrules-gcp

`vrules-gcp` is a dual-mode WASI component providing Vertex AI web grounding and zero-trust cloud credential management.

## Responsibility

- **Provider Mode (`id: "ai.vrules.grounding"`):** Invokes Vertex AI (Gemini) with Google Search grounding for assistant tools like `web_ground`.
- **Auth Mode (`id: "gcp-auth"`):** Generates `Authorization` headers using Application Default Credentials (ADC) or explicit tokens, encapsulating cloud credentials entirely within the WebAssembly guest.

## Interfaces

Exported WASI World: `plugin-component` (`wit/vrules.wit`)
- Exports interface `plugin` (`initialize`, `invoke`).
- Imports interface `host` (`invoke`, `get-embedding-info`, `embed`, `http`, `log`).

## Configuration

In `release/vrules-components.json`:
- `mode`: `"auth"` for cloud cache tier authentication, or omitted/provider for Vertex grounding.
- `project`: Google Cloud project ID.
- `location`: Vertex AI region (e.g. `"us-central1"` or `"global"`).
- `access_token` / ADC configuration.
