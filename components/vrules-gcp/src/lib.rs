#![deny(unsafe_code)]

use std::fs;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rsa::RsaPrivateKey;
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs1v15::SigningKey;
use rsa::pkcs8::DecodePrivateKey;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::Sha256;
use signature::{SignatureEncoding, Signer};

#[allow(unsafe_code)]
mod bindings {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "plugin-component",
    });
}

use bindings::ai::vrules::types::{
    HttpHeader, HttpRequest, HttpResponse, PluginDescriptor, PluginKind,
};
use bindings::exports::ai::vrules::plugin::Guest;

const CLOUD_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";

struct GcpComponent;

static STATE: OnceLock<Mutex<State>> = OnceLock::new();

struct State {
    config: Config,
    credential: Credential,
    token: Option<CachedToken>,
}

#[derive(Debug, Clone, Deserialize)]
struct Config {
    project: String,
    #[serde(default = "default_location")]
    location: String,
    #[serde(default = "default_standard_model")]
    model_standard: String,
    #[serde(default = "default_high_model")]
    model_high: String,
    #[serde(default)]
    credentials_file: Option<String>,
    #[serde(default)]
    access_token: Option<String>,
}

enum Credential {
    Static(String),
    AuthorizedUser {
        client_id: String,
        client_secret: String,
        refresh_token: String,
    },
    ServiceAccount {
        client_email: String,
        private_key: String,
        token_uri: String,
    },
    Metadata,
}

struct CachedToken {
    value: String,
    expires_at: u64,
}

#[derive(Debug, Deserialize)]
struct AuthorizedUserFile {
    client_id: String,
    client_secret: String,
    refresh_token: String,
}

#[derive(Debug, Deserialize)]
struct ServiceAccountFile {
    client_email: String,
    private_key: String,
    #[serde(default = "default_token_uri")]
    token_uri: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default = "default_expires_in")]
    expires_in: u64,
}

impl Guest for GcpComponent {
    fn initialize(config: String) -> Result<PluginDescriptor, String> {
        let config: Config =
            serde_json::from_str(&config).map_err(|e| format!("invalid GCP config: {e}"))?;
        if config.project.trim().is_empty() {
            return Err("GCP project must not be empty".to_string());
        }
        let credential = load_credential(&config)?;
        STATE
            .set(Mutex::new(State {
                config,
                credential,
                token: None,
            }))
            .map_err(|_| "GCP component is already initialized".to_string())?;
        Ok(PluginDescriptor {
            id: "ai.vrules.grounding".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            kind: PluginKind::Provider,
            operations: vec!["Ground".to_string(), "Summarize".to_string()],
        })
    }

    fn invoke(operation: String, payload: String) -> Result<String, String> {
        let args: Value =
            serde_json::from_str(&payload).map_err(|e| format!("invalid GCP request: {e}"))?;
        let effort = args.get("effort").and_then(Value::as_str).unwrap_or("low");
        let (prompt, grounded) = match operation.as_str() {
            "Ground" | "ground" | "web_ground" => {
                (required_string(&args, "query")?.to_string(), true)
            }
            "Summarize" | "summarize" => (
                format!(
                    "Summarize the following concisely:\n\n{}",
                    required_string(&args, "content")?
                ),
                false,
            ),
            other => return Err(format!("unsupported GCP operation `{other}`")),
        };
        let text = generate(&prompt, grounded, effort)?;
        Ok(json!({ "text": text }).to_string())
    }
}

fn generate(prompt: &str, grounded: bool, effort: &str) -> Result<String, String> {
    let mut state = state()?.lock().map_err(|_| "GCP lock poisoned")?;
    let model = if effort.eq_ignore_ascii_case("high") {
        state.config.model_high.clone()
    } else {
        state.config.model_standard.clone()
    };
    let token = access_token(&mut state)?;
    let host = if state.config.location == "global" {
        "aiplatform.googleapis.com".to_string()
    } else {
        format!("{}-aiplatform.googleapis.com", state.config.location)
    };
    let url = format!(
        "https://{host}/v1/projects/{}/locations/{}/publishers/google/models/{model}:generateContent",
        state.config.project, state.config.location
    );
    let mut body = json!({
        "contents": [{ "role": "user", "parts": [{ "text": prompt }] }],
    });
    if grounded {
        body["tools"] = json!([{ "googleSearch": {} }]);
    }
    let body = serde_json::to_vec(&body).map_err(|e| format!("encode Vertex request: {e}"))?;
    let response = request_with_retry(HttpRequest {
        method: "POST".to_string(),
        url,
        headers: vec![
            HttpHeader {
                name: "authorization".to_string(),
                value: format!("Bearer {token}"),
            },
            HttpHeader {
                name: "content-type".to_string(),
                value: "application/json".to_string(),
            },
        ],
        body,
    })?;
    if !(200..300).contains(&response.status) {
        return Err(format!(
            "Vertex returned {}: {}",
            response.status,
            String::from_utf8_lossy(&response.body)
        ));
    }
    let payload: Value = serde_json::from_slice(&response.body)
        .map_err(|e| format!("decode Vertex response: {e}"))?;
    extract_text(&payload).ok_or_else(|| "Vertex response contains no candidate text".to_string())
}

fn access_token(state: &mut State) -> Result<String, String> {
    let now = now_secs();
    if let Some(token) = &state.token
        && token.expires_at.saturating_sub(60) > now
    {
        return Ok(token.value.clone());
    }
    let response = match &state.credential {
        Credential::Static(token) => {
            return Ok(token.clone());
        }
        Credential::AuthorizedUser {
            client_id,
            client_secret,
            refresh_token,
        } => oauth_token(
            TOKEN_ENDPOINT,
            &[
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("refresh_token", refresh_token),
                ("grant_type", "refresh_token"),
            ],
        )?,
        Credential::ServiceAccount {
            client_email,
            private_key,
            token_uri,
        } => {
            let assertion = service_account_assertion(client_email, private_key, token_uri)?;
            oauth_token(
                token_uri,
                &[
                    ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                    ("assertion", &assertion),
                ],
            )?
        }
        Credential::Metadata => metadata_token()?,
    };
    state.token = Some(CachedToken {
        value: response.access_token.clone(),
        expires_at: now.saturating_add(response.expires_in),
    });
    Ok(response.access_token)
}

fn load_credential(config: &Config) -> Result<Credential, String> {
    if let Some(token) = config
        .access_token
        .as_deref()
        .filter(|token| !token.trim().is_empty())
    {
        return Ok(Credential::Static(token.to_string()));
    }
    let Some(path) = config.credentials_file.as_deref() else {
        return Ok(Credential::Metadata);
    };
    let text = fs::read_to_string(path).map_err(|e| format!("read credentials {path}: {e}"))?;
    let value: Value =
        serde_json::from_str(&text).map_err(|e| format!("parse credentials {path}: {e}"))?;
    match value.get("type").and_then(Value::as_str) {
        Some("authorized_user") => {
            let file: AuthorizedUserFile = serde_json::from_value(value)
                .map_err(|e| format!("parse authorized_user credentials: {e}"))?;
            Ok(Credential::AuthorizedUser {
                client_id: file.client_id,
                client_secret: file.client_secret,
                refresh_token: file.refresh_token,
            })
        }
        Some("service_account") => {
            let file: ServiceAccountFile = serde_json::from_value(value)
                .map_err(|e| format!("parse service_account credentials: {e}"))?;
            Ok(Credential::ServiceAccount {
                client_email: file.client_email,
                private_key: file.private_key,
                token_uri: file.token_uri,
            })
        }
        Some(kind) => Err(format!("unsupported ADC credential type `{kind}`")),
        None => Err("ADC credentials have no `type`".to_string()),
    }
}

fn oauth_token(url: &str, fields: &[(&str, &str)]) -> Result<TokenResponse, String> {
    let body = fields
        .iter()
        .map(|(name, value)| format!("{}={}", form_encode(name), form_encode(value)))
        .collect::<Vec<_>>()
        .join("&")
        .into_bytes();
    let response = request_with_retry(HttpRequest {
        method: "POST".to_string(),
        url: url.to_string(),
        headers: vec![HttpHeader {
            name: "content-type".to_string(),
            value: "application/x-www-form-urlencoded".to_string(),
        }],
        body,
    })?;
    parse_token_response(response, "OAuth token")
}

fn metadata_token() -> Result<TokenResponse, String> {
    let response = request_with_retry(HttpRequest {
        method: "GET".to_string(),
        url: format!(
            "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token?scopes={}",
            form_encode(CLOUD_SCOPE)
        ),
        headers: vec![HttpHeader {
            name: "metadata-flavor".to_string(),
            value: "Google".to_string(),
        }],
        body: Vec::new(),
    })?;
    parse_token_response(response, "metadata token")
}

fn parse_token_response(response: HttpResponse, context: &str) -> Result<TokenResponse, String> {
    if !(200..300).contains(&response.status) {
        return Err(format!(
            "{context} returned {}: {}",
            response.status,
            String::from_utf8_lossy(&response.body)
        ));
    }
    serde_json::from_slice(&response.body).map_err(|e| format!("decode {context}: {e}"))
}

fn service_account_assertion(
    client_email: &str,
    private_key: &str,
    token_uri: &str,
) -> Result<String, String> {
    let now = now_secs();
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
    let claims = serde_json::to_vec(&json!({
        "iss": client_email,
        "scope": CLOUD_SCOPE,
        "aud": token_uri,
        "iat": now,
        "exp": now.saturating_add(3600),
    }))
    .map_err(|e| format!("encode service-account claims: {e}"))?;
    let claims = URL_SAFE_NO_PAD.encode(claims);
    let signing_input = format!("{header}.{claims}");
    let key = RsaPrivateKey::from_pkcs8_pem(private_key)
        .or_else(|_| RsaPrivateKey::from_pkcs1_pem(private_key))
        .map_err(|e| format!("parse service-account private key: {e}"))?;
    let signing_key = SigningKey::<Sha256>::new(key);
    let signature = signing_key.sign(signing_input.as_bytes());
    Ok(format!(
        "{signing_input}.{}",
        URL_SAFE_NO_PAD.encode(signature.to_bytes())
    ))
}

fn request_with_retry(request: HttpRequest) -> Result<HttpResponse, String> {
    let mut last_error = String::new();
    for attempt in 0..=3 {
        match bindings::ai::vrules::host::http(&request) {
            Ok(response) if !matches!(response.status, 408 | 429 | 500 | 502 | 503 | 504) => {
                return Ok(response);
            }
            Ok(response) if attempt == 3 => return Ok(response),
            Ok(response) => {
                last_error = format!("HTTP {}", response.status);
            }
            Err(error) if attempt == 3 => return Err(error),
            Err(error) => last_error = error,
        }
        std::thread::sleep(Duration::from_millis(200u64 << attempt));
    }
    Err(format!("request retries exhausted: {last_error}"))
}

fn extract_text(value: &Value) -> Option<String> {
    let parts = value
        .get("candidates")?
        .get(0)?
        .get("content")?
        .get("parts")?
        .as_array()?;
    let text = parts
        .iter()
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<String>();
    (!text.is_empty()).then_some(text)
}

fn form_encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn required_string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("missing `{key}`"))
}

fn state() -> Result<&'static Mutex<State>, String> {
    STATE
        .get()
        .ok_or_else(|| "GCP component is not initialized".to_string())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn default_location() -> String {
    "global".to_string()
}

fn default_standard_model() -> String {
    "gemini-3.5-flash".to_string()
}

fn default_high_model() -> String {
    "gemini-3.1-pro-preview".to_string()
}

fn default_token_uri() -> String {
    TOKEN_ENDPOINT.to_string()
}

fn default_expires_in() -> u64 {
    3_600
}

#[allow(unsafe_code)]
mod component_export {
    use super::GcpComponent;
    use crate::bindings;

    crate::bindings::export!(GcpComponent with_types_in bindings);
}
