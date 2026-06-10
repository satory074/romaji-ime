//! Cloud-AI conversion: the headline feature.
//!
//! The user types *loose* romaji and an LLM turns it into natural Japanese,
//! tolerating typos and using the surrounding document text as context (the
//! Sumibi approach). This module defines the provider-agnostic [`Converter`]
//! trait plus an [`HttpConverter`] that talks to an OpenAI-compatible or
//! Anthropic chat API (behind the `cloud-http` feature).
//!
//! The call is **blocking**; the session runs it on a background thread and the
//! frontend polls, so the input thread is never blocked (see
//! [`crate::Session::begin_ai_convert`]).

use std::path::Path;
use std::sync::Arc;

/// What to convert: the raw romaji the user typed, a best-effort local kana
/// rendering, and the surrounding document text for disambiguation.
#[derive(Debug, Clone, Default)]
pub struct ConvertRequest {
    pub romaji: String,
    pub kana: String,
    pub context_before: String,
    pub context_after: String,
}

#[derive(Debug, Clone)]
pub enum AiError {
    Config(String),
    Network(String),
    Parse(String),
}

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AiError::Config(m) => write!(f, "config error: {m}"),
            AiError::Network(m) => write!(f, "network error: {m}"),
            AiError::Parse(m) => write!(f, "parse error: {m}"),
        }
    }
}

/// A pluggable conversion backend. `Send + Sync` so the session can hand an
/// `Arc<dyn Converter>` to a background thread.
pub trait Converter: Send + Sync {
    /// Blocking: return ranked Japanese candidates (best first).
    fn convert(&self, req: &ConvertRequest) -> Result<Vec<String>, AiError>;
}

/// Result of polling an in-flight conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiPoll {
    Pending,
    Ready,
    Error(String),
}

// ---------------------------------------------------------------------------
// Prompt + response parsing (no network; unit-tested directly)
// ---------------------------------------------------------------------------

/// The system instruction shared by all providers.
pub fn system_prompt() -> &'static str {
    "You are a Japanese romaji input method (IME) with no mode switching: the \
     user types everything as loose Latin text — romaji for Japanese, and plain \
     Latin for English words, names, code identifiers, and URLs mixed in. \
     Convert it into natural Japanese, but KEEP intended English/Latin words, \
     identifiers, and URLs in the Latin alphabet with correct casing \
     (e.g. 'github'->'GitHub', 'ok'->'OK', 'api'->'API'). Tolerate typos, \
     missing vowels, and abbreviations. Use the surrounding context if given. \
     Respond with ONLY a JSON array of up to 5 candidate strings, best first, \
     and nothing else. Each candidate is the conversion of the marked input \
     only (do not include the surrounding context)."
}

/// Build the user message. The text to convert is wrapped in 《》 between the
/// surrounding context so the model knows exactly what to replace.
pub fn user_prompt(req: &ConvertRequest) -> String {
    format!(
        "context-before: {}\ncontext-after: {}\nromaji: {}\nkana: {}\nConvert 《{}》.",
        req.context_before, req.context_after, req.romaji, req.kana, req.romaji
    )
}

/// Parse the model's reply into candidates. Prefers a JSON array; falls back to
/// extracting a bracketed array, then to non-empty lines.
pub fn parse_candidates(reply: &str) -> Vec<String> {
    let trimmed = reply
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    if let Ok(v) = serde_json::from_str::<Vec<String>>(trimmed) {
        return clean(v);
    }
    // Find the first [...] block and try again.
    if let (Some(start), Some(end)) = (trimmed.find('['), trimmed.rfind(']')) {
        if end > start {
            if let Ok(v) = serde_json::from_str::<Vec<String>>(&trimmed[start..=end]) {
                return clean(v);
            }
        }
    }
    // Last resort: each non-empty line is a candidate.
    clean(
        trimmed
            .lines()
            .map(|l| {
                l.trim()
                    .trim_start_matches(['-', '*', '・', ' '])
                    .to_string()
            })
            .collect(),
    )
}

fn clean(v: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for s in v {
        let s = s.trim().to_string();
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
        if out.len() >= 8 {
            break;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Configuration + construction from config.json / environment
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Deserialize, Default)]
pub struct AiConfig {
    pub provider: Option<String>, // "openai" (default) | "anthropic"
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub endpoint: Option<String>,
    pub timeout_ms: Option<u64>,
}

impl AiConfig {
    fn from_env() -> Self {
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        AiConfig {
            provider: env("ROMAJI_IME_PROVIDER"),
            api_key: env("ROMAJI_IME_API_KEY"),
            model: env("ROMAJI_IME_MODEL"),
            endpoint: env("ROMAJI_IME_ENDPOINT"),
            timeout_ms: env("ROMAJI_IME_TIMEOUT_MS").and_then(|s| s.parse().ok()),
        }
    }

    fn is_usable(&self) -> bool {
        // Cloud APIs need a key; local servers (LM Studio/Ollama) just need an endpoint.
        self.api_key.is_some() || self.endpoint.is_some()
    }
}

/// Build a converter from `{data_dir}/config.json`, falling back to environment
/// variables. Returns `None` if AI is not configured or the `cloud-http`
/// feature is off.
pub fn converter_from_config(data_dir: Option<&Path>) -> Option<Arc<dyn Converter>> {
    let mut cfg = data_dir
        .map(|d| d.join("config.json"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<AiConfig>(&s).ok())
        .unwrap_or_default();

    // Environment fills any gaps (and works in `cargo run` / terminal contexts).
    let env = AiConfig::from_env();
    cfg.provider = cfg.provider.or(env.provider);
    cfg.api_key = cfg.api_key.or(env.api_key);
    cfg.model = cfg.model.or(env.model);
    cfg.endpoint = cfg.endpoint.or(env.endpoint);
    cfg.timeout_ms = cfg.timeout_ms.or(env.timeout_ms);

    if !cfg.is_usable() {
        return None;
    }
    build_http_converter(cfg)
}

#[cfg(feature = "cloud-http")]
fn build_http_converter(cfg: AiConfig) -> Option<Arc<dyn Converter>> {
    Some(Arc::new(HttpConverter::new(cfg)))
}

#[cfg(not(feature = "cloud-http"))]
fn build_http_converter(_cfg: AiConfig) -> Option<Arc<dyn Converter>> {
    None
}

// ---------------------------------------------------------------------------
// HTTP converter (OpenAI-compatible + Anthropic)
// ---------------------------------------------------------------------------

#[cfg(feature = "cloud-http")]
mod http {
    use super::*;

    #[derive(Clone, Copy, PartialEq)]
    enum Provider {
        OpenAi,
        Anthropic,
    }

    pub struct HttpConverter {
        provider: Provider,
        endpoint: String,
        api_key: String,
        model: String,
        timeout_ms: u64,
    }

    impl HttpConverter {
        pub fn new(cfg: AiConfig) -> Self {
            // Gemini speaks an OpenAI-compatible endpoint, so it reuses the OpenAi
            // wire shape with Google's base URL and a default Flash model.
            let (provider, default_endpoint, default_model) = match cfg.provider.as_deref() {
                Some("anthropic") => (
                    Provider::Anthropic,
                    "https://api.anthropic.com",
                    "claude-3-5-haiku-latest",
                ),
                Some("gemini") => (
                    Provider::OpenAi,
                    "https://generativelanguage.googleapis.com/v1beta/openai",
                    "gemini-2.0-flash",
                ),
                _ => (Provider::OpenAi, "https://api.openai.com/v1", "gpt-4o-mini"),
            };
            HttpConverter {
                provider,
                endpoint: cfg.endpoint.unwrap_or_else(|| default_endpoint.to_string()),
                api_key: cfg.api_key.unwrap_or_default(),
                model: cfg.model.unwrap_or_else(|| default_model.to_string()),
                timeout_ms: cfg.timeout_ms.unwrap_or(5000),
            }
        }

        fn url(&self) -> String {
            match self.provider {
                Provider::OpenAi => {
                    format!("{}/chat/completions", self.endpoint.trim_end_matches('/'))
                }
                Provider::Anthropic => {
                    format!("{}/v1/messages", self.endpoint.trim_end_matches('/'))
                }
            }
        }

        fn body(&self, req: &ConvertRequest) -> serde_json::Value {
            let sys = system_prompt();
            let user = user_prompt(req);
            match self.provider {
                Provider::OpenAi => serde_json::json!({
                    "model": self.model,
                    "temperature": 0.3,
                    "messages": [
                        {"role": "system", "content": sys},
                        {"role": "user", "content": user},
                    ],
                }),
                Provider::Anthropic => serde_json::json!({
                    "model": self.model,
                    "max_tokens": 256,
                    "system": sys,
                    "messages": [{"role": "user", "content": user}],
                }),
            }
        }

        fn extract_text(&self, resp: &serde_json::Value) -> Option<String> {
            match self.provider {
                Provider::OpenAi => resp
                    .get("choices")?
                    .get(0)?
                    .get("message")?
                    .get("content")?
                    .as_str()
                    .map(str::to_string),
                Provider::Anthropic => resp
                    .get("content")?
                    .get(0)?
                    .get("text")?
                    .as_str()
                    .map(str::to_string),
            }
        }
    }

    impl Converter for HttpConverter {
        fn convert(&self, req: &ConvertRequest) -> Result<Vec<String>, AiError> {
            if self.api_key.is_empty() && self.endpoint.starts_with("https://api.") {
                return Err(AiError::Config("missing API key".into()));
            }
            // ureq with only the native-tls feature needs an explicit connector;
            // without it HTTPS fails immediately ("no TLS").
            let connector = native_tls::TlsConnector::new()
                .map_err(|e| AiError::Network(format!("TLS init: {e}")))?;
            let agent = ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_millis(self.timeout_ms))
                .tls_connector(std::sync::Arc::new(connector))
                .build();

            let mut request = agent
                .post(&self.url())
                .set("content-type", "application/json");
            request = match self.provider {
                Provider::OpenAi => {
                    request.set("authorization", &format!("Bearer {}", self.api_key))
                }
                Provider::Anthropic => request
                    .set("x-api-key", &self.api_key)
                    .set("anthropic-version", "2023-06-01"),
            };

            let body = self.body(req).to_string();
            let resp = request
                .send_string(&body)
                .map_err(|e| AiError::Network(e.to_string()))?;
            let text = resp
                .into_string()
                .map_err(|e| AiError::Network(e.to_string()))?;
            let json: serde_json::Value =
                serde_json::from_str(&text).map_err(|e| AiError::Parse(e.to_string()))?;
            let content = self
                .extract_text(&json)
                .ok_or_else(|| AiError::Parse("unexpected response shape".into()))?;
            Ok(parse_candidates(&content))
        }
    }
}

#[cfg(feature = "cloud-http")]
pub use http::HttpConverter;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_array() {
        assert_eq!(
            parse_candidates(r#"["日本語","にほんご"]"#),
            vec!["日本語", "にほんご"]
        );
    }

    #[test]
    fn parses_fenced_json() {
        let reply = "```json\n[\"今日は\", \"こんにちは\"]\n```";
        assert_eq!(parse_candidates(reply), vec!["今日は", "こんにちは"]);
    }

    #[test]
    fn parses_array_embedded_in_prose() {
        let reply = "Here you go: [\"水\", \"見ず\"] hope that helps";
        assert_eq!(parse_candidates(reply), vec!["水", "見ず"]);
    }

    #[test]
    fn falls_back_to_lines() {
        let reply = "日本語\n- にほんご\n* 二本後";
        assert_eq!(
            parse_candidates(reply),
            vec!["日本語", "にほんご", "二本後"]
        );
    }

    #[test]
    fn dedups_and_drops_empty() {
        assert_eq!(parse_candidates(r#"["あ","","あ","い"]"#), vec!["あ", "い"]);
    }

    #[test]
    fn user_prompt_marks_the_input() {
        let req = ConvertRequest {
            romaji: "nihongo".into(),
            kana: "にほんご".into(),
            ..Default::default()
        };
        let p = user_prompt(&req);
        assert!(p.contains("《nihongo》"));
        assert!(p.contains("にほんご"));
    }
}
