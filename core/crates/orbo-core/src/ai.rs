//! Optional local AI through an Ollama server. Configured process-wide by the app; the engine in
//! `engines/ai.rs` does the work. Nothing here talks to the network except `Ollama`, and only when a job runs.

use std::sync::RwLock;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const DEFAULT_URL: &str = "http://localhost:11434";
pub const UNREACHABLE: &str = "Ollama ist nicht erreichbar. Läuft ollama serve?";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AiConfig {
    pub url: String,
    pub model: Option<String>,
    /// Enabled actions: rename, alt_text, summary, translate.
    pub actions: Vec<String>,
    /// Allow a server on another machine. File contents then leave this device.
    #[serde(default)]
    pub allow_remote: bool,
}

static CONFIG: RwLock<Option<AiConfig>> = RwLock::new(None);

/// `None` switches the AI features off.
pub fn configure(cfg: Option<AiConfig>) {
    *CONFIG.write().unwrap() = cfg;
}

pub fn config() -> Option<AiConfig> {
    CONFIG.read().unwrap().clone()
}

/// Cheap check used by the catalog on every drag: configured and the action switched on. No network.
pub fn action_enabled(action: Option<&str>) -> bool {
    let Some(action) = action else { return false };
    CONFIG.read().unwrap().as_ref().map_or(false, |c| c.actions.iter().any(|a| a == action))
}

/// Host part of a URL without scheme, credentials, port and IPv6 brackets.
pub fn url_host(url: &str) -> String {
    let rest = url.trim().split_once("://").map_or(url.trim(), |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit('@').next().unwrap_or("");
    let host = if let Some(inner) = authority.strip_prefix('[') {
        inner.split(']').next().unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    };
    host.to_lowercase()
}

pub fn is_local_url(url: &str) -> bool {
    let h = url_host(url);
    h == "localhost" || h == "::1" || h.starts_with("127.") && h.split('.').count() == 4 && h.split('.').all(|p| p.parse::<u8>().is_ok())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// This device.
    Local,
    /// Home or company network, VPN (NetBird, Tailscale), link-local or a bare host name.
    Private,
    /// Anything that may be on the internet.
    Public,
}

/// Classifies the server address without DNS: only literal private ranges and LAN-style names count as private.
pub fn server_scope(url: &str) -> Scope {
    if is_local_url(url) {
        return Scope::Local;
    }
    let h = url_host(url);
    if let Ok(ip) = h.parse::<std::net::Ipv4Addr>() {
        let [a, b, ..] = ip.octets();
        let private = a == 10 || (a == 172 && (16..=31).contains(&b)) || (a == 192 && b == 168) || (a == 169 && b == 254) || (a == 100 && (64..=127).contains(&b));
        return if private { Scope::Private } else { Scope::Public };
    }
    if let Ok(ip) = h.parse::<std::net::Ipv6Addr>() {
        let first = ip.segments()[0];
        let private = (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80;
        return if private { Scope::Private } else { Scope::Public };
    }
    let lan_name = !h.is_empty() && (!h.contains('.') || [".local", ".lan", ".home", ".home.arpa", ".internal", ".fritz.box", ".netbird.cloud", ".ts.net"].iter().any(|s| h.ends_with(s)));
    if lan_name { Scope::Private } else { Scope::Public }
}

/// Local and own-network servers are fine; a server that may be on the internet needs explicit consent.
pub fn check_allowed(cfg: &AiConfig) -> Result<()> {
    if cfg.allow_remote || server_scope(&cfg.url) != Scope::Public {
        return Ok(());
    }
    bail!("Der KI-Server {} liegt nicht in deinem Netz. Dateiinhalte würden über das Internet gehen. Erlaube das nur bewusst in den Einstellungen unter KI.", url_host(&cfg.url))
}

/// One question to a model. Mockable so the plumbing runs without Ollama.
pub trait Llm: Send + Sync {
    fn generate(&self, prompt: &str, images_b64: &[String]) -> Result<String>;
}

pub struct Ollama {
    base: String,
    pub model: String,
}

fn agent(secs: u64) -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(3)))
        .timeout_global(Some(Duration::from_secs(secs)))
        .build()
        .into()
}

fn transport_error(e: ureq::Error) -> anyhow::Error {
    match e {
        ureq::Error::Timeout(ureq::Timeout::Resolve | ureq::Timeout::Connect) => anyhow!(UNREACHABLE),
        ureq::Error::Timeout(_) => anyhow!("Das Modell antwortet nicht rechtzeitig. Ein kleineres Modell oder eine kürzere Datei hilft."),
        ureq::Error::BadUri(_) | ureq::Error::Protocol(_) => anyhow!("Die KI-Adresse ist ungültig. Erwartet wird zum Beispiel http://localhost:11434"),
        _ => anyhow!(UNREACHABLE),
    }
}

fn api_error(status: u16, body: &Value, model: &str) -> anyhow::Error {
    let msg = body["error"].as_str().unwrap_or("").to_string();
    if status == 404 || msg.contains("not found") {
        return anyhow!("Das Modell „{model}“ ist nicht installiert. Mit „ollama pull {model}“ laden oder ein anderes Modell wählen.");
    }
    anyhow!("Ollama meldet einen Fehler ({status}): {msg}")
}

impl Ollama {
    fn call(&self, path: &str, body: &Value, secs: u64) -> Result<(u16, Value)> {
        let mut resp = agent(secs).post(&format!("{}{path}", self.base)).send_json(body).map_err(transport_error)?;
        let status = resp.status().as_u16();
        let json = resp.body_mut().read_json::<Value>().unwrap_or(Value::Null);
        Ok((status, json))
    }

    /// Checks the safety rule, reaches the server and picks a model (the configured one, else the first suitable).
    pub fn connect(cfg: &AiConfig, need_vision: bool) -> Result<Ollama> {
        check_allowed(cfg)?;
        let url = if cfg.url.trim().is_empty() { DEFAULT_URL } else { cfg.url.trim() };
        let mut o = Ollama { base: url.trim_end_matches('/').to_string(), model: String::new() };
        let model = match cfg.model.as_deref().filter(|m| !m.is_empty()) {
            Some(m) => m.to_string(),
            None => o.first_model(need_vision)?,
        };
        o.model = model;
        let caps = o.capabilities()?;
        if need_vision && caps.as_ref().map_or(false, |c| !c.iter().any(|x| x == "vision")) {
            bail!("Das gewählte Modell kann keine Bilder lesen ({}). Wähle in den Einstellungen unter KI ein Modell mit Bildverständnis, zum Beispiel ein Vision-Modell.", o.model);
        }
        Ok(o)
    }

    fn first_model(&self, need_vision: bool) -> Result<String> {
        let mut resp = agent(5).get(&format!("{}/api/tags", self.base)).call().map_err(transport_error)?;
        let tags = resp.body_mut().read_json::<Value>().unwrap_or(Value::Null);
        let names: Vec<String> = tags["models"].as_array().map(|a| a.iter().filter_map(|m| m["name"].as_str().map(String::from)).collect()).unwrap_or_default();
        if names.is_empty() {
            bail!("Auf dem Ollama-Server ist noch kein Modell installiert. Mit „ollama pull“ eines laden.");
        }
        if need_vision {
            for n in &names {
                let probe = Ollama { base: self.base.clone(), model: n.clone() };
                if probe.capabilities()?.map_or(false, |c| c.iter().any(|x| x == "vision")) {
                    return Ok(n.clone());
                }
            }
        }
        Ok(names[0].clone())
    }

    /// `None` when the server does not report capabilities (older versions).
    fn capabilities(&self) -> Result<Option<Vec<String>>> {
        let (status, json) = self.call("/api/show", &json!({ "model": self.model }), 10)?;
        if status != 200 {
            return Err(api_error(status, &json, &self.model));
        }
        Ok(json["capabilities"].as_array().map(|a| a.iter().filter_map(|c| c.as_str().map(String::from)).collect()))
    }
}

impl Llm for Ollama {
    fn generate(&self, prompt: &str, images_b64: &[String]) -> Result<String> {
        let mut msg = json!({ "role": "user", "content": prompt });
        if !images_b64.is_empty() {
            msg["images"] = json!(images_b64);
        }
        let mut body = json!({
            "model": self.model, "messages": [msg], "stream": false, "think": false,
            "options": { "temperature": 0.2, "num_ctx": 8192 },
        });
        let (mut status, mut json) = self.call("/api/chat", &body, 600)?;
        // Models without thinking support may reject the flag.
        if status != 200 && json["error"].as_str().map_or(false, |e| e.contains("think")) {
            body.as_object_mut().unwrap().remove("think");
            (status, json) = self.call("/api/chat", &body, 600)?;
        }
        if status != 200 {
            return Err(api_error(status, &json, &self.model));
        }
        let text = json["message"]["content"].as_str().unwrap_or("");
        Ok(text.to_string())
    }
}

/// Removes thinking blocks and a wrapping code fence; models often add them despite the prompt.
pub fn clean_answer(raw: &str) -> String {
    let mut s = raw.to_string();
    while let Some(a) = s.find("<think>") {
        match s[a..].find("</think>") {
            Some(b) => s.replace_range(a..a + b + "</think>".len(), ""),
            None => {
                s.truncate(a);
                break;
            }
        }
    }
    if let Some(b) = s.find("</think>") {
        s = s[b + "</think>".len()..].to_string();
    }
    let t = s.trim();
    if t.len() >= 6 && t.starts_with("```") && t.ends_with("```") {
        let inner = &t[3..t.len() - 3];
        let inner = inner.split_once('\n').map_or(inner, |(_, rest)| rest);
        return inner.trim().to_string();
    }
    t.to_string()
}

pub fn strip_quotes(s: &str) -> &str {
    s.trim().trim_matches(|c| matches!(c, '"' | '\'' | '„' | '“' | '”' | '‚' | '‘' | '’' | '«' | '»' | '`')).trim()
}

/// Splits at paragraph, line or sentence borders into pieces of at most `max` characters.
pub fn chunk_text(text: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let push_piece = |piece: &str, sep: &str, cur: &mut String, out: &mut Vec<String>| {
        if !cur.is_empty() && cur.chars().count() + sep.len() + piece.chars().count() > max {
            out.push(std::mem::take(cur));
        }
        if !cur.is_empty() {
            cur.push_str(sep);
        }
        cur.push_str(piece);
    };
    for para in text.split("\n\n") {
        if para.chars().count() <= max {
            push_piece(para, "\n\n", &mut cur, &mut out);
            continue;
        }
        for line in para.split_inclusive(['\n', '.', '!', '?']) {
            let mut rest = line;
            while rest.chars().count() > max {
                let cut = rest.char_indices().nth(max).map_or(rest.len(), |(i, _)| i);
                push_piece(&rest[..cut], "", &mut cur, &mut out);
                rest = &rest[cut..];
            }
            push_piece(rest, "", &mut cur, &mut out);
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out.into_iter().filter(|c| !c.trim().is_empty()).collect()
}
