use std::collections::HashSet;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct ManifestPath(PathBuf);

impl ManifestPath {
    pub fn resolve(explicit: Option<PathBuf>) -> Result<Self, String> {
        if let Some(path) = explicit {
            return Ok(Self(path));
        }
        if let Some(path) = std::env::var_os("VRULES_COMPONENT_MANIFEST") {
            return Ok(Self(PathBuf::from(path)));
        }
        let executable = std::env::current_exe()
            .map_err(|e| format!("resolve current executable for component manifest: {e}"))?;
        let directory = executable
            .parent()
            .ok_or_else(|| "current executable has no parent directory".to_string())?;
        Ok(Self(directory.join("vrules-components.json")))
    }

    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ComponentManifest {
    pub runtime: ComponentSpec,
    pub embedding: ComponentSpec,
    pub admin_plugin: String,
    #[serde(default)]
    pub cache_plugin: Option<String>,
    #[serde(default)]
    pub plugins: Vec<NamedComponentSpec>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NamedComponentSpec {
    pub id: String,
    #[serde(flatten)]
    pub component: ComponentSpec,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ComponentSpec {
    pub path: PathBuf,
    #[serde(default = "empty_object")]
    pub config: Value,
    #[serde(default)]
    pub preopens: Vec<Preopen>,
    #[serde(default)]
    pub allowed_http_hosts: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Preopen {
    pub host: PathBuf,
    pub guest: String,
    #[serde(default)]
    pub read_only: bool,
}

impl ComponentManifest {
    pub fn load(path: &ManifestPath) -> Result<Self> {
        let text = std::fs::read_to_string(path.as_path()).with_context(|| {
            format!(
                "read component manifest {}; set VRULES_COMPONENT_MANIFEST or use --manifest",
                path.as_path().display()
            )
        })?;
        let mut manifest: Self = serde_json::from_str(&text)
            .with_context(|| format!("parse {}", path.as_path().display()))?;
        let base = path.as_path().parent().unwrap_or_else(|| Path::new("."));
        manifest.resolve_paths(base);
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn use_embedding_model(&mut self, path: &Path, name: Option<&str>) -> Result<()> {
        let model_path = path
            .canonicalize()
            .with_context(|| format!("resolve embedding model {}", path.display()))?;
        if !model_path.is_file() {
            bail!(
                "embedding model does not exist or is not a file: {}",
                model_path.display()
            );
        }
        let file_name = model_path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| {
                anyhow!(
                    "embedding model filename is not valid UTF-8: {}",
                    model_path.display()
                )
            })?;
        let model_name = match name {
            Some(value) if value.trim().is_empty() => {
                bail!("embedding model name must not be empty")
            }
            Some(value) => value.to_string(),
            None => model_path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or(file_name)
                .to_string(),
        };
        let model_dir = model_path
            .parent()
            .ok_or_else(|| anyhow!("embedding model has no parent directory"))?;
        let config = self
            .embedding
            .config
            .as_object_mut()
            .ok_or_else(|| anyhow!("embedding component config must be a JSON object"))?;
        config.insert(
            "model_path".to_string(),
            Value::String(format!("/models/{file_name}")),
        );
        config.insert("model".to_string(), Value::String(model_name));
        config.insert(
            "model_sha256".to_string(),
            Value::String(sha256_file(&model_path)?),
        );

        if let Some(preopen) = self
            .embedding
            .preopens
            .iter_mut()
            .find(|preopen| preopen.guest == "/models")
        {
            preopen.host = model_dir.to_path_buf();
            preopen.read_only = true;
        } else {
            self.embedding.preopens.push(Preopen {
                host: model_dir.to_path_buf(),
                guest: "/models".to_string(),
                read_only: true,
            });
        }
        Ok(())
    }

    fn resolve_paths(&mut self, base: &Path) {
        resolve_spec(base, &mut self.runtime);
        resolve_spec(base, &mut self.embedding);
        for plugin in &mut self.plugins {
            resolve_spec(base, &mut plugin.component);
        }
    }

    fn validate(&self) -> Result<()> {
        let mut ids = HashSet::new();
        for plugin in &self.plugins {
            if plugin.id.trim().is_empty() {
                bail!("plugin id must not be empty");
            }
            if !ids.insert(plugin.id.as_str()) {
                bail!("duplicate plugin id `{}`", plugin.id);
            }
        }
        if !ids.contains(self.admin_plugin.as_str()) {
            bail!(
                "admin_plugin `{}` does not name a configured plugin",
                self.admin_plugin
            );
        }
        if let Some(cache_plugin) = &self.cache_plugin {
            if !ids.contains(cache_plugin.as_str()) {
                bail!("cache_plugin `{cache_plugin}` does not name a configured plugin");
            }
            // The admin plugin calls `host.embed` while its own mutex is held;
            // routing the embed cache back through that same plugin would
            // relock a non-reentrant mutex and deadlock the shim.
            if cache_plugin == &self.admin_plugin {
                bail!("cache_plugin must not be the admin plugin");
            }
        }
        validate_spec("runtime", &self.runtime)?;
        validate_spec("embedding", &self.embedding)?;
        for plugin in &self.plugins {
            validate_spec(&plugin.id, &plugin.component)?;
        }
        Ok(())
    }
}

fn resolve_spec(base: &Path, spec: &mut ComponentSpec) {
    if spec.path.is_relative() {
        spec.path = base.join(&spec.path);
    }
    for preopen in &mut spec.preopens {
        if preopen.host.is_relative() {
            preopen.host = base.join(&preopen.host);
        }
    }
}

fn validate_spec(name: &str, spec: &ComponentSpec) -> Result<()> {
    if !spec.path.is_file() {
        bail!(
            "{name} component does not exist or is not a file: {}",
            spec.path.display()
        );
    }
    for preopen in &spec.preopens {
        if preopen.guest.trim().is_empty() {
            bail!("{name} has a preopen with an empty guest path");
        }
        if !preopen.host.is_dir() {
            bail!(
                "{name} preopen does not exist or is not a directory: {}",
                preopen.host.display()
            );
        }
    }
    Ok(())
}

fn empty_object() -> Value {
    Value::Object(Default::default())
}

fn sha256_file(path: &Path) -> Result<String> {
    let file =
        File::open(path).with_context(|| format!("open embedding model {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .with_context(|| format!("hash embedding model {}", path.display()))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedding_override_mounts_and_identifies_model() {
        let directory =
            std::env::temp_dir().join(format!("vrules-model-override-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let model = directory.join("custom.gguf");
        std::fs::write(&model, b"gguf").unwrap();
        let mut manifest = ComponentManifest {
            runtime: component("runtime.wasm"),
            embedding: component("embedding.wasm"),
            admin_plugin: "admin".into(),
            cache_plugin: None,
            plugins: Vec::new(),
        };

        manifest
            .use_embedding_model(&model, Some("Custom Model"))
            .unwrap();

        assert_eq!(manifest.embedding.config["model"], "Custom Model");
        assert_eq!(
            manifest.embedding.config["model_path"],
            "/models/custom.gguf"
        );
        assert_eq!(
            manifest.embedding.config["model_sha256"],
            "1cb1b7e0f8b96cee3445e317b8064d8805bf35c7dc7de82cddcb9f78d4c95e0e"
        );
        assert_eq!(manifest.embedding.preopens[0].host, directory);
        assert!(manifest.embedding.preopens[0].read_only);

        std::fs::remove_file(model).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    fn component(path: &str) -> ComponentSpec {
        ComponentSpec {
            path: PathBuf::from(path),
            config: Value::Object(Default::default()),
            preopens: Vec::new(),
            allowed_http_hosts: Vec::new(),
        }
    }
}
