use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{DirPerms, FilePerms, WasiCtx, WasiCtxView, WasiView, p2::add_to_linker_sync};

use crate::cache_key::{self, CacheKey, DEFAULT_CANON_NS};
use crate::manifest::{ComponentManifest, ComponentSpec};

mod runtime_bindings {
    wasmtime::component::bindgen!({
        path: "../../wit",
        world: "runtime-component",
    });
}

mod plugin_bindings {
    wasmtime::component::bindgen!({
        path: "../../wit",
        world: "plugin-component",
    });
}

mod embedding_bindings {
    wasmtime::component::bindgen!({
        path: "../../wit",
        world: "embedding-component",
    });
}

pub use runtime_bindings::ai::vrules::types::Identity;

pub struct RuntimeHost {
    runtime: Mutex<RuntimeInstance>,
    services: Arc<Services>,
    admin_plugin: String,
}

struct Services {
    embedding: OnceLock<Arc<Mutex<EmbeddingInstance>>>,
    embedding_info: OnceLock<EmbeddingMetadata>,
    plugins: RwLock<HashMap<String, Arc<Mutex<PluginInstance>>>>,
    output: ComponentOutput,
    http: reqwest::blocking::Client,
    cache_plugin: Option<String>,
    upstream: Option<Upstream>,
}

/// Parent vrules-rest tier: pulled through on a local miss, written up after a
/// local compute. Requests run while the daemon's global host mutex is held, so
/// the client carries explicit timeouts — an unbounded stall would freeze every
/// transport.
struct Upstream {
    authority: String,
    client: reqwest::blocking::Client,
}

type HttpResponse = (u16, Vec<(String, String)>, Vec<u8>);

/// Failure modes of the cache-tier operations exposed to the REST transport.
#[derive(Debug)]
pub enum CacheError {
    /// No cache plugin is configured in the component manifest.
    Unavailable,
    /// The request itself is malformed (wrong dimensions, bad hash).
    Invalid(String),
    /// The cache component rejected or failed the operation.
    Failed(String),
}

/// Failure modes of compute-on-miss resolution.
#[derive(Debug)]
pub enum ResolveError {
    /// The `{model}` path segment does not match the active embedding model;
    /// computing under a foreign model segment would poison the tier's
    /// content-addressed keys.
    ModelMismatch {
        active: String,
    },
    Failed(String),
}

#[derive(Clone)]
struct EmbeddingMetadata {
    id: String,
    version: String,
    model: String,
    revision: String,
    dimensions: u32,
}

struct HostState {
    wasi: WasiCtx,
    table: ResourceTable,
    services: Arc<Services>,
    allowed_http_hosts: HashSet<String>,
}

struct RuntimeInstance {
    store: Store<HostState>,
    bindings: runtime_bindings::RuntimeComponent,
}

struct PluginInstance {
    store: Store<HostState>,
    bindings: plugin_bindings::PluginComponent,
}

struct EmbeddingInstance {
    store: Store<HostState>,
    bindings: embedding_bindings::EmbeddingComponent,
}

#[derive(Clone, Copy, Debug)]
pub enum ComponentOutput {
    Stdio,
    Daemon,
}

impl RuntimeHost {
    pub fn load(
        manifest: ComponentManifest,
        output: ComponentOutput,
        upstream: Option<String>,
    ) -> Result<Self> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.wasm_exceptions(true);
        let engine = Engine::new(&config)?;
        let upstream = upstream
            .map(|authority| {
                Ok::<_, anyhow::Error>(Upstream {
                    authority,
                    client: reqwest::blocking::Client::builder()
                        .connect_timeout(Duration::from_millis(500))
                        .timeout(Duration::from_secs(2))
                        .build()
                        .context("build upstream cache-tier HTTP client")?,
                })
            })
            .transpose()?;
        let services = Arc::new(Services {
            embedding: OnceLock::new(),
            embedding_info: OnceLock::new(),
            plugins: RwLock::new(HashMap::new()),
            output,
            http: reqwest::blocking::Client::builder()
                .build()
                .context("build component HTTP client")?,
            cache_plugin: manifest.cache_plugin.clone(),
            upstream,
        });

        let (embedding, embedding_info) =
            EmbeddingInstance::load(&engine, &manifest.embedding, Arc::clone(&services))?;
        services
            .embedding
            .set(Arc::new(Mutex::new(embedding)))
            .map_err(|_| anyhow!("embedding component initialized more than once"))?;
        services
            .embedding_info
            .set(EmbeddingMetadata {
                id: embedding_info.id.clone(),
                version: embedding_info.version.clone(),
                model: embedding_info.model.clone(),
                revision: embedding_info.revision.clone(),
                dimensions: embedding_info.dimensions,
            })
            .map_err(|_| anyhow!("embedding metadata initialized more than once"))?;

        let mut descriptors = Vec::new();
        for named in &manifest.plugins {
            let (plugin, descriptor) =
                PluginInstance::load(&engine, &named.component, Arc::clone(&services))?;
            if descriptor.id != named.id {
                bail!(
                    "plugin manifest id `{}` does not match component descriptor `{}`",
                    named.id,
                    descriptor.id
                );
            }
            let mut plugins = services
                .plugins
                .write()
                .map_err(|_| anyhow!("plugin registry lock poisoned"))?;
            if plugins
                .insert(named.id.clone(), Arc::new(Mutex::new(plugin)))
                .is_some()
            {
                bail!("duplicate plugin `{}`", named.id);
            }
            descriptors.push(runtime_bindings::ai::vrules::types::PluginDescriptor {
                id: descriptor.id,
                version: descriptor.version,
                kind: match descriptor.kind {
                    plugin_bindings::ai::vrules::types::PluginKind::Admin => {
                        runtime_bindings::ai::vrules::types::PluginKind::Admin
                    }
                    plugin_bindings::ai::vrules::types::PluginKind::Rules => {
                        runtime_bindings::ai::vrules::types::PluginKind::Rules
                    }
                    plugin_bindings::ai::vrules::types::PluginKind::Storage => {
                        runtime_bindings::ai::vrules::types::PluginKind::Storage
                    }
                    plugin_bindings::ai::vrules::types::PluginKind::Provider => {
                        runtime_bindings::ai::vrules::types::PluginKind::Provider
                    }
                },
                operations: descriptor.operations,
            });
        }

        let runtime = RuntimeInstance::load(
            &engine,
            &manifest.runtime,
            Arc::clone(&services),
            descriptors,
            runtime_bindings::ai::vrules::types::EmbeddingInfo {
                id: embedding_info.id,
                version: embedding_info.version,
                model: embedding_info.model,
                revision: embedding_info.revision,
                dimensions: embedding_info.dimensions,
            },
        )?;
        Ok(Self {
            runtime: Mutex::new(runtime),
            services,
            admin_plugin: manifest.admin_plugin,
        })
    }

    pub fn mcp(&self, message: &str, identity: &Identity) -> Result<Option<String>> {
        self.runtime
            .lock()
            .map_err(|_| anyhow!("runtime component lock poisoned"))?
            .mcp(message, identity)
    }

    pub fn admin(&self, method: &str, params: &str) -> Result<String> {
        let payload = serde_json::json!({ "method": method, "params": params }).to_string();
        self.services
            .invoke(&self.admin_plugin, "rpc", &payload)
            .map_err(anyhow::Error::msg)
    }

    pub fn tick(&self) -> Result<()> {
        self.services
            .invoke(&self.admin_plugin, "tick", "{}")
            .map(|_| ())
            .map_err(anyhow::Error::msg)
    }

    /// Revision and output dimensions of the active embedding model.
    pub fn embedding_metadata(&self) -> Result<(String, u32)> {
        let info = self
            .services
            .embedding_info
            .get()
            .ok_or_else(|| anyhow!("embedding component is not loaded"))?;
        Ok((info.revision.clone(), info.dimensions))
    }

    /// Immutable lookup by content hash. Local cache only — the immutable GET
    /// route never cascades or computes.
    pub fn cache_lookup(
        &self,
        model: &str,
        canon: &str,
        text_hash: &str,
    ) -> Result<Option<(CacheKey, Vec<f32>, u32)>, CacheError> {
        let hash = cache_key::parse_text_hash(text_hash).ok_or_else(|| {
            CacheError::Invalid("hash must be exactly 32 lowercase hex characters".to_string())
        })?;
        let key = CacheKey::from_parts(model, canon, hash);
        let dims = self.services.dimensions();
        Ok(self
            .services
            .cache_get(&key, dims)
            .map(|(vector, generation)| (key, vector, generation)))
    }

    /// Compute-on-miss resolution under the active embedding model.
    pub fn resolve_embedding(
        &self,
        model: &str,
        canon: &str,
        text: &str,
    ) -> Result<(CacheKey, Vec<f32>, u32), ResolveError> {
        let (revision, _) = self
            .embedding_metadata()
            .map_err(|error| ResolveError::Failed(error.to_string()))?;
        if model != revision {
            return Err(ResolveError::ModelMismatch { active: revision });
        }
        let (vector, generation) = self
            .services
            .embed_cached(&revision, canon, text)
            .map_err(ResolveError::Failed)?;
        Ok((CacheKey::new(&revision, canon, text), vector, generation))
    }

    /// Write-up receiver: store a vector computed by a downstream node. Foreign
    /// model segments are accepted — content-addressed keys namespace them.
    pub fn cache_store(
        &self,
        model: &str,
        canon: &str,
        text_hash: &str,
        vector: Vec<f32>,
    ) -> Result<u32, CacheError> {
        let hash = cache_key::parse_text_hash(text_hash).ok_or_else(|| {
            CacheError::Invalid("hash must be exactly 32 lowercase hex characters".to_string())
        })?;
        if let Some(dims) = self.services.dimensions()
            && model == self.services.revision().as_deref().unwrap_or_default()
            && vector.len() != dims
        {
            return Err(CacheError::Invalid(format!(
                "vector has {} dimensions, the active model produces {dims}",
                vector.len()
            )));
        }
        let key = CacheKey::from_parts(model, canon, hash);
        let payload = serde_json::json!({ "key": key.to_hex(), "vector": vector }).to_string();
        let response = self.services.cache_invoke("put", &payload)?;
        Ok(generation_of(&response))
    }

    /// Bump the cache epoch: rule-driven mass invalidation, append-only.
    pub fn cache_expire(&self) -> Result<u32, CacheError> {
        let response = self.services.cache_invoke("expire", "{}")?;
        let value: serde_json::Value = serde_json::from_str(&response)
            .map_err(|error| CacheError::Failed(format!("decode expire result: {error}")))?;
        Ok(value["epoch"].as_u64().unwrap_or_default() as u32)
    }

    /// Raw cache statistics JSON from the cache component.
    pub fn cache_stats(&self) -> Result<String, CacheError> {
        self.services.cache_invoke("stats", "{}")
    }
}

fn generation_of(response: &str) -> u32 {
    serde_json::from_str::<serde_json::Value>(response)
        .ok()
        .and_then(|value| value["generation"].as_u64())
        .unwrap_or_default() as u32
}

impl RuntimeInstance {
    fn load(
        engine: &Engine,
        spec: &ComponentSpec,
        services: Arc<Services>,
        plugins: Vec<runtime_bindings::ai::vrules::types::PluginDescriptor>,
        embedding: runtime_bindings::ai::vrules::types::EmbeddingInfo,
    ) -> Result<Self> {
        let component = Component::from_file(engine, &spec.path).map_err(|error| {
            anyhow!("compile runtime component {}: {error}", spec.path.display())
        })?;
        let mut linker = Linker::new(engine);
        runtime_bindings::RuntimeComponent::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| {
            state
        })?;
        add_to_linker_sync(&mut linker)?;
        let state = host_state(spec, services)?;
        let mut store = Store::new(engine, state);
        let bindings =
            runtime_bindings::RuntimeComponent::instantiate(&mut store, &component, &linker)
                .map_err(|error| anyhow!("instantiate runtime component: {error}"))?;
        let config = serde_json::to_string(&spec.config)?;
        bindings
            .ai_vrules_runtime()
            .call_initialize(&mut store, &config, &plugins, &embedding)?
            .map_err(|message| anyhow!("initialize runtime component: {message}"))?;
        Ok(Self { store, bindings })
    }

    fn mcp(&mut self, message: &str, identity: &Identity) -> Result<Option<String>> {
        self.bindings
            .ai_vrules_runtime()
            .call_mcp(&mut self.store, message, identity)?
            .map_err(|message| anyhow!(message))
    }
}

impl PluginInstance {
    fn load(
        engine: &Engine,
        spec: &ComponentSpec,
        services: Arc<Services>,
    ) -> Result<(Self, plugin_bindings::ai::vrules::types::PluginDescriptor)> {
        let component = Component::from_file(engine, &spec.path).map_err(|error| {
            anyhow!("compile plugin component {}: {error}", spec.path.display())
        })?;
        let mut linker = Linker::new(engine);
        plugin_bindings::PluginComponent::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| {
            state
        })?;
        add_to_linker_sync(&mut linker)?;
        let state = host_state(spec, services)?;
        let mut store = Store::new(engine, state);
        let bindings =
            plugin_bindings::PluginComponent::instantiate(&mut store, &component, &linker)
                .map_err(|error| anyhow!("instantiate plugin component: {error}"))?;
        let config = serde_json::to_string(&spec.config)?;
        let descriptor = bindings
            .ai_vrules_plugin()
            .call_initialize(&mut store, &config)?
            .map_err(|message| anyhow!("initialize plugin component: {message}"))?;
        Ok((Self { store, bindings }, descriptor))
    }

    fn invoke(&mut self, operation: &str, payload: &str) -> Result<String, String> {
        self.bindings
            .ai_vrules_plugin()
            .call_invoke(&mut self.store, operation, payload)
            .map_err(|error| error.to_string())?
    }
}

impl EmbeddingInstance {
    fn load(
        engine: &Engine,
        spec: &ComponentSpec,
        services: Arc<Services>,
    ) -> Result<(Self, embedding_bindings::ai::vrules::types::EmbeddingInfo)> {
        let component = Component::from_file(engine, &spec.path).map_err(|error| {
            anyhow!(
                "compile embedding component {}: {error}",
                spec.path.display()
            )
        })?;
        let mut linker = Linker::new(engine);
        embedding_bindings::EmbeddingComponent::add_to_linker::<_, HasSelf<_>>(
            &mut linker,
            |state| state,
        )?;
        add_to_linker_sync(&mut linker)?;
        let state = host_state(spec, services)?;
        let mut store = Store::new(engine, state);
        let bindings =
            embedding_bindings::EmbeddingComponent::instantiate(&mut store, &component, &linker)
                .map_err(|error| anyhow!("instantiate embedding component: {error}"))?;
        let config = serde_json::to_string(&spec.config)?;
        let info = bindings
            .ai_vrules_embedding()
            .call_initialize(&mut store, &config)?
            .map_err(|message| anyhow!("initialize embedding component: {message}"))?;
        Ok((Self { store, bindings }, info))
    }

    fn embed(&mut self, text: &str) -> Result<Vec<f32>, String> {
        self.bindings
            .ai_vrules_embedding()
            .call_embed(&mut self.store, text)
            .map_err(|error| error.to_string())?
    }
}

impl Services {
    fn invoke(&self, plugin: &str, operation: &str, payload: &str) -> Result<String, String> {
        let instance = self
            .plugins
            .read()
            .map_err(|_| "plugin registry lock poisoned".to_string())?
            .get(plugin)
            .cloned()
            .ok_or_else(|| format!("plugin `{plugin}` is not loaded"))?;
        instance
            .lock()
            .map_err(|_| format!("plugin `{plugin}` lock poisoned"))?
            .invoke(operation, payload)
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        let revision = self
            .revision()
            .ok_or_else(|| "embedding component is not loaded".to_string())?;
        self.embed_cached(&revision, DEFAULT_CANON_NS, text)
            .map(|(vector, _)| vector)
    }

    /// The cache-through embed path shared by every embedding consumer.
    ///
    /// Lock order (each acquired and released in turn, never nested): the
    /// caller's component mutex is already held → cache plugin mutex →
    /// embedding mutex → cache plugin mutex again. The cache component makes
    /// no host calls, so no edge leads back into any of these.
    fn embed_cached(
        &self,
        model_version: &str,
        canon_ns: &str,
        text: &str,
    ) -> Result<(Vec<f32>, u32), String> {
        let key = CacheKey::new(model_version, canon_ns, text);
        let dims = self.dimensions();
        if let Some(hit) = self.cache_get(&key, dims) {
            return Ok(hit);
        }
        if let Some(vector) = self.upstream_fetch(model_version, canon_ns, &key, dims) {
            let generation = self.cache_put(&key, &vector).unwrap_or_default();
            return Ok((vector, generation));
        }
        let vector = self
            .embedding
            .get()
            .ok_or_else(|| "embedding component is not loaded".to_string())?
            .lock()
            .map_err(|_| "embedding component lock poisoned".to_string())?
            .embed(text)?;
        let generation = self.cache_put(&key, &vector).unwrap_or_default();
        self.upstream_store(model_version, canon_ns, &key, &vector);
        Ok((vector, generation))
    }

    fn revision(&self) -> Option<String> {
        self.embedding_info.get().map(|info| info.revision.clone())
    }

    fn dimensions(&self) -> Option<usize> {
        self.embedding_info
            .get()
            .map(|info| info.dimensions as usize)
    }

    /// Cache lookup. Any cache failure is logged and treated as a miss — the
    /// cache must never fail an embed.
    fn cache_get(&self, key: &CacheKey, dims: Option<usize>) -> Option<(Vec<f32>, u32)> {
        let plugin = self.cache_plugin.as_deref()?;
        let payload = serde_json::json!({ "key": key.to_hex() }).to_string();
        let response = match self.invoke(plugin, "get", &payload) {
            Ok(response) => response,
            Err(error) => {
                eprintln!("cache get failed (treated as a miss): {error}");
                return None;
            }
        };
        let value: serde_json::Value = serde_json::from_str(&response).ok()?;
        if value["found"].as_bool() != Some(true) {
            return None;
        }
        let vector = value["vector"]
            .as_array()?
            .iter()
            .map(|element| element.as_f64().map(|element| element as f32))
            .collect::<Option<Vec<_>>>()?;
        if dims.is_some_and(|dims| vector.len() != dims) {
            eprintln!("cache entry dimension mismatch (treated as a miss)");
            return None;
        }
        let generation = value["generation"].as_u64().unwrap_or_default() as u32;
        Some((vector, generation))
    }

    /// Best-effort cache write-back; a failure is logged, never propagated.
    fn cache_put(&self, key: &CacheKey, vector: &[f32]) -> Option<u32> {
        let plugin = self.cache_plugin.as_deref()?;
        let payload = serde_json::json!({ "key": key.to_hex(), "vector": vector }).to_string();
        match self.invoke(plugin, "put", &payload) {
            Ok(response) => Some(generation_of(&response)),
            Err(error) => {
                eprintln!("cache put failed: {error}");
                None
            }
        }
    }

    /// Strict cache operation for the REST transport's write/expire/stats
    /// routes, where "no cache plugin" must surface as an error.
    fn cache_invoke(&self, operation: &str, payload: &str) -> Result<String, CacheError> {
        let plugin = self
            .cache_plugin
            .as_deref()
            .ok_or(CacheError::Unavailable)?;
        self.invoke(plugin, operation, payload)
            .map_err(CacheError::Failed)
    }

    fn upstream_url(
        &self,
        upstream: &Upstream,
        model: &str,
        canon: &str,
        key: &CacheKey,
    ) -> String {
        format!(
            "http://{}/vrules-rest/v1/embeddings/{}/{}/{}",
            upstream.authority,
            cache_key::encode_path_segment(model),
            cache_key::encode_path_segment(canon),
            key.text_hash_hex()
        )
    }

    /// Pull-through from the parent tier; any failure falls back to local
    /// inference.
    fn upstream_fetch(
        &self,
        model: &str,
        canon: &str,
        key: &CacheKey,
        dims: Option<usize>,
    ) -> Option<Vec<f32>> {
        let upstream = self.upstream.as_ref()?;
        let url = self.upstream_url(upstream, model, canon, key);
        let response = match upstream.client.get(&url).send() {
            Ok(response) => response,
            Err(error) => {
                eprintln!("upstream cache tier fetch failed: {error}");
                return None;
            }
        };
        if !response.status().is_success() {
            return None;
        }
        let bytes = response.bytes().ok()?;
        let vector = cache_key::vector_from_le_bytes(&bytes)?;
        if dims.is_some_and(|dims| vector.len() != dims) {
            eprintln!("upstream cache tier returned a dimension mismatch (ignored)");
            return None;
        }
        Some(vector)
    }

    /// Best-effort write-up of a locally computed vector to the parent tier.
    fn upstream_store(&self, model: &str, canon: &str, key: &CacheKey, vector: &[f32]) {
        let Some(upstream) = self.upstream.as_ref() else {
            return;
        };
        let url = self.upstream_url(upstream, model, canon, key);
        let result = upstream
            .client
            .put(&url)
            .header("content-type", "application/octet-stream")
            .body(cache_key::vector_to_le_bytes(vector))
            .send();
        match result {
            Ok(response) if !response.status().is_success() => {
                eprintln!(
                    "upstream cache tier write-up rejected: HTTP {}",
                    response.status().as_u16()
                );
            }
            Ok(_) => {}
            Err(error) => eprintln!("upstream cache tier write-up failed: {error}"),
        }
    }

    fn http(
        &self,
        allowed_hosts: &HashSet<String>,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: &[u8],
    ) -> Result<HttpResponse, String> {
        let parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid URL: {e}"))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| "HTTP URL has no host".to_string())?;
        if !allowed_hosts.contains(host) {
            return Err(format!(
                "HTTP host `{host}` is not allowed for this component"
            ));
        }
        if parsed.scheme() != "https" && !is_metadata_host(host) {
            return Err("component HTTP requires HTTPS except for metadata endpoints".to_string());
        }
        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|e| format!("invalid HTTP method: {e}"))?;
        let mut request = self.http.request(method, parsed);
        for (name, value) in headers {
            request = request.header(name, value);
        }
        let response = request
            .body(body.to_vec())
            .send()
            .map_err(|e| format!("component HTTP request: {e}"))?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_string(), value.to_string()))
            })
            .collect();
        let body = response
            .bytes()
            .map_err(|e| format!("read component HTTP response: {e}"))?
            .to_vec();
        Ok((status, headers, body))
    }
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl runtime_bindings::ai::vrules::types::Host for HostState {}
impl plugin_bindings::ai::vrules::types::Host for HostState {}
impl embedding_bindings::ai::vrules::types::Host for HostState {}

macro_rules! impl_host {
    ($bindings:ident) => {
        impl $bindings::ai::vrules::host::Host for HostState {
            fn invoke(
                &mut self,
                plugin: String,
                operation: String,
                payload: String,
            ) -> std::result::Result<String, String> {
                self.services.invoke(&plugin, &operation, &payload)
            }

            fn embed(&mut self, text: String) -> std::result::Result<Vec<f32>, String> {
                self.services.embed(&text)
            }

            fn get_embedding_info(
                &mut self,
            ) -> std::result::Result<$bindings::ai::vrules::types::EmbeddingInfo, String> {
                self.services
                    .embedding_info
                    .get()
                    .cloned()
                    .ok_or_else(|| "embedding component is not loaded".to_string())
                    .map(|info| $bindings::ai::vrules::types::EmbeddingInfo {
                        id: info.id,
                        version: info.version,
                        model: info.model,
                        revision: info.revision,
                        dimensions: info.dimensions,
                    })
            }

            fn http(
                &mut self,
                request: $bindings::ai::vrules::types::HttpRequest,
            ) -> std::result::Result<$bindings::ai::vrules::types::HttpResponse, String> {
                let headers = request
                    .headers
                    .iter()
                    .map(|header| (header.name.clone(), header.value.clone()))
                    .collect::<Vec<_>>();
                self.services
                    .http(
                        &self.allowed_http_hosts,
                        &request.method,
                        &request.url,
                        &headers,
                        &request.body,
                    )
                    .map(
                        |(status, headers, body)| $bindings::ai::vrules::types::HttpResponse {
                            status,
                            headers: headers
                                .into_iter()
                                .map(|(name, value)| $bindings::ai::vrules::types::HttpHeader {
                                    name,
                                    value,
                                })
                                .collect(),
                            body,
                        },
                    )
            }

            fn log(&mut self, level: String, message: String) {
                eprintln!("component[{level}]: {message}");
            }
        }
    };
}

impl_host!(runtime_bindings);
impl_host!(plugin_bindings);
impl_host!(embedding_bindings);

fn host_state(spec: &ComponentSpec, services: Arc<Services>) -> Result<HostState> {
    let mut builder = WasiCtx::builder();
    match services.output {
        ComponentOutput::Stdio => {
            builder
                .stdout(wasmtime_wasi::cli::stderr())
                .inherit_stderr();
        }
        ComponentOutput::Daemon => {
            builder.inherit_stdout().inherit_stderr();
        }
    }
    for preopen in &spec.preopens {
        let dir_perms = if preopen.read_only {
            DirPerms::READ
        } else {
            DirPerms::all()
        };
        let file_perms = if preopen.read_only {
            FilePerms::READ
        } else {
            FilePerms::all()
        };
        builder
            .preopened_dir(&preopen.host, &preopen.guest, dir_perms, file_perms)
            .map_err(|error| {
                anyhow!(
                    "preopen {} as {}: {error}",
                    preopen.host.display(),
                    preopen.guest
                )
            })?;
    }
    Ok(HostState {
        wasi: builder.build(),
        table: ResourceTable::new(),
        services,
        allowed_http_hosts: spec.allowed_http_hosts.iter().cloned().collect(),
    })
}

fn is_metadata_host(host: &str) -> bool {
    matches!(host, "metadata.google.internal" | "169.254.169.254")
}
