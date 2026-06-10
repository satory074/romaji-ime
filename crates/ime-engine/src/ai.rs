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

    /// Stream candidates as they are generated, calling `sink` with the growing
    /// list each time more become available (so the frontend can show the first
    /// candidate at time-to-first-token rather than waiting for completion).
    /// Default: non-streaming — deliver everything in one shot.
    fn convert_streaming(
        &self,
        req: &ConvertRequest,
        sink: &mut dyn FnMut(Vec<String>),
    ) -> Result<(), AiError> {
        let all = self.convert(req)?;
        sink(all);
        Ok(())
    }
}

/// Result of polling an in-flight conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiPoll {
    Pending,
    /// Partial candidates are available and more may still arrive (streaming).
    Streaming,
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
     Respond with up to 4 candidates, ONE PER LINE, best first — no JSON, no \
     numbering, no quotes, no extra text. Each line is the conversion of the \
     marked input only (do not include the surrounding context)."
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
    // Last resort (and the normal path now that we ask for one-per-line output):
    // each non-empty line is a candidate.
    clean(trimmed.lines().map(clean_line).collect())
}

/// Normalize one candidate line: strip bullets, surrounding quotes/backticks.
pub fn clean_line(line: &str) -> String {
    line.trim()
        .trim_start_matches(['-', '*', '・', ' '])
        .trim()
        .trim_matches('"')
        .trim_matches('`')
        .trim()
        .to_string()
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
    use std::collections::{HashMap, VecDeque};
    use std::io::BufRead;
    use std::sync::Mutex;
    use std::time::Duration;

    #[derive(Clone, Copy, PartialEq)]
    enum Provider {
        OpenAi,
        Anthropic,
    }

    /// Small thread-safe FIFO cache so repeated input (re-typing, corrections)
    /// returns instantly without a network round trip.
    struct FifoCache {
        map: HashMap<String, Vec<String>>,
        order: VecDeque<String>,
        cap: usize,
    }

    impl FifoCache {
        fn new(cap: usize) -> Self {
            FifoCache {
                map: HashMap::new(),
                order: VecDeque::new(),
                cap,
            }
        }
        fn get(&self, key: &str) -> Option<Vec<String>> {
            self.map.get(key).cloned()
        }
        fn put(&mut self, key: String, value: Vec<String>) {
            if self.map.contains_key(&key) {
                return;
            }
            if self.order.len() >= self.cap {
                if let Some(old) = self.order.pop_front() {
                    self.map.remove(&old);
                }
            }
            self.order.push_back(key.clone());
            self.map.insert(key, value);
        }
    }

    /// Build the ureq agent ONCE and reuse it so the TLS connection is kept alive
    /// across conversions (a fresh agent per call meant a TLS handshake every
    /// time). ureq with only the native-tls feature needs an explicit connector.
    fn build_agent(timeout_ms: u64) -> ureq::Agent {
        let builder = ureq::AgentBuilder::new().timeout(Duration::from_millis(timeout_ms));
        match native_tls::TlsConnector::new() {
            Ok(connector) => builder
                .tls_connector(std::sync::Arc::new(connector))
                .build(),
            Err(_) => builder.build(),
        }
    }

    pub struct HttpConverter {
        provider: Provider,
        url: String,
        api_key: String,
        model: String,
        agent: ureq::Agent,
        cache: Mutex<FifoCache>,
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
            let endpoint = cfg.endpoint.unwrap_or_else(|| default_endpoint.to_string());
            let base = endpoint.trim_end_matches('/');
            let url = match provider {
                Provider::OpenAi => format!("{base}/chat/completions"),
                Provider::Anthropic => format!("{base}/v1/messages"),
            };
            HttpConverter {
                provider,
                url,
                api_key: cfg.api_key.unwrap_or_default(),
                model: cfg.model.unwrap_or_else(|| default_model.to_string()),
                agent: build_agent(cfg.timeout_ms.unwrap_or(5000)),
                cache: Mutex::new(FifoCache::new(256)),
            }
        }

        fn cache_key(req: &ConvertRequest) -> String {
            format!(
                "{}\u{1}{}\u{1}{}",
                req.romaji, req.context_before, req.context_after
            )
        }

        fn body(&self, req: &ConvertRequest) -> serde_json::Value {
            let sys = system_prompt();
            let user = user_prompt(req);
            match self.provider {
                Provider::OpenAi => serde_json::json!({
                    "model": self.model,
                    "temperature": 0.3,
                    "max_tokens": 128,   // small output -> faster (a few short candidates)
                    "messages": [
                        {"role": "system", "content": sys},
                        {"role": "user", "content": user},
                    ],
                }),
                Provider::Anthropic => serde_json::json!({
                    "model": self.model,
                    "max_tokens": 128,
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

        fn check_key(&self) -> Result<(), AiError> {
            if self.api_key.is_empty() && self.url.starts_with("https://") {
                Err(AiError::Config("missing API key".into()))
            } else {
                Ok(())
            }
        }

        fn cached(&self, key: &str) -> Option<Vec<String>> {
            self.cache.lock().ok().and_then(|c| c.get(key))
        }

        fn store(&self, key: String, value: Vec<String>) {
            if let Ok(mut c) = self.cache.lock() {
                c.put(key, value);
            }
        }

        /// A POST to the API with content-type + provider auth headers set,
        /// using the persistent keep-alive agent.
        fn auth_request(&self) -> ureq::Request {
            let r = self
                .agent
                .post(&self.url)
                .set("content-type", "application/json");
            match self.provider {
                Provider::OpenAi => r.set("authorization", &format!("Bearer {}", self.api_key)),
                Provider::Anthropic => r
                    .set("x-api-key", &self.api_key)
                    .set("anthropic-version", "2023-06-01"),
            }
        }

        /// Complete candidate lines from accumulated streamed text. The trailing
        /// line is dropped unless the text ends with a newline (it may be partial),
        /// and JSON-looking lines are ignored (final parse recovers those).
        fn complete_lines(content: &str) -> Vec<String> {
            let mut lines: Vec<&str> = content.split('\n').collect();
            if !content.ends_with('\n') {
                lines.pop();
            }
            let mut out: Vec<String> = Vec::new();
            for l in lines {
                let c = clean_line(l);
                if !c.is_empty() && !c.starts_with('[') && !c.starts_with('{') && !out.contains(&c)
                {
                    out.push(c);
                }
            }
            out
        }
    }

    impl Converter for HttpConverter {
        fn convert(&self, req: &ConvertRequest) -> Result<Vec<String>, AiError> {
            self.check_key()?;
            let key = Self::cache_key(req);
            if let Some(hit) = self.cached(&key) {
                return Ok(hit); // instant, no network
            }
            let body = self.body(req).to_string();
            let resp = self
                .auth_request()
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
            let candidates = parse_candidates(&content);
            if !candidates.is_empty() {
                self.store(key, candidates.clone());
            }
            Ok(candidates)
        }

        fn convert_streaming(
            &self,
            req: &ConvertRequest,
            sink: &mut dyn FnMut(Vec<String>),
        ) -> Result<(), AiError> {
            self.check_key()?;
            let key = Self::cache_key(req);
            if let Some(hit) = self.cached(&key) {
                sink(hit); // instant
                return Ok(());
            }
            // SSE streaming is implemented for the OpenAI-compatible shape (incl.
            // Gemini). Anthropic falls back to one non-streamed delivery.
            if self.provider != Provider::OpenAi {
                let all = self.convert(req)?;
                sink(all);
                return Ok(());
            }

            let mut body = self.body(req);
            body["stream"] = serde_json::json!(true);
            let resp = self
                .auth_request()
                .send_string(&body.to_string())
                .map_err(|e| AiError::Network(e.to_string()))?;

            let reader = std::io::BufReader::new(resp.into_reader());
            let mut content = String::new();
            let mut emitted = 0usize;
            for line in reader.lines() {
                let line = line.map_err(|e| AiError::Network(e.to_string()))?;
                let data = match line.strip_prefix("data:") {
                    Some(d) => d.trim(),
                    None => continue, // blank / comment / non-data SSE line
                };
                if data == "[DONE]" {
                    break;
                }
                if let Ok(chunk) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(delta) = chunk["choices"][0]["delta"]["content"].as_str() {
                        content.push_str(delta);
                        let cands = Self::complete_lines(&content);
                        if cands.len() > emitted {
                            emitted = cands.len();
                            sink(cands);
                        }
                    }
                }
            }
            // Authoritative final list (parse_candidates also recovers JSON if the
            // model ignored the one-per-line instruction).
            let final_cands = parse_candidates(&content);
            if !final_cands.is_empty() {
                self.store(key, final_cands.clone());
                sink(final_cands);
            }
            Ok(())
        }
    }

    #[cfg(test)]
    mod cache_tests {
        use super::*;

        #[test]
        fn fifo_cache_evicts_oldest() {
            let mut c = FifoCache::new(2);
            c.put("a".into(), vec!["1".into()]);
            c.put("b".into(), vec!["2".into()]);
            c.put("c".into(), vec!["3".into()]); // evicts "a"
            assert!(c.get("a").is_none());
            assert_eq!(c.get("b"), Some(vec!["2".to_string()]));
            assert_eq!(c.get("c"), Some(vec!["3".to_string()]));
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
