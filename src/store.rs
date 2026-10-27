//! OpenWhispr data access.
//!
//! Live data goes through OpenWhispr's CLI bridge, a local HTTP API the app
//! starts on 127.0.0.1 and describes in `~/.openwhispr/cli-bridge.json`. Going
//! through it keeps the running app's windows and cloud sync up to date; the
//! app's own code warns that writing its SQLite tables by hand loses rows on
//! the next launch. When the app isn't running, history is read from
//! `transcriptions.db` read-only and changes are refused.
//!
//! OpenWhispr has no way to change a transcript's text from outside, so
//! corrections made here are kept in owtui's own file. They overlay the app's
//! text in the UI and are the "output" side of exported training pairs.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub fn app_data_dir() -> PathBuf {
    // Electron's userData: ~/Library/Application Support, ~/.config or %APPDATA%.
    dirs::config_dir().unwrap_or_default().join("open-whispr")
}

pub fn owtui_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".local/share"))
        .join("owtui")
}

pub enum Method {
    Get,
    Post(Value),
    Delete,
}

pub struct Bridge {
    pub file: PathBuf,
    agent: ureq::Agent,
}

#[derive(Deserialize)]
struct BridgeInfo {
    port: u16,
    token: String,
}

impl Bridge {
    pub fn new(file: PathBuf) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(5)))
            .build()
            .into();
        Self { file, agent }
    }

    fn endpoint(&self) -> Result<(String, String)> {
        // Read every call: the app writes a new port and token on each launch.
        let raw = fs::read_to_string(&self.file)
            .map_err(|_| anyhow!("OpenWhispr isn't running (no CLI bridge file)."))?;
        let info: BridgeInfo =
            serde_json::from_str(&raw).map_err(|_| anyhow!("Unreadable CLI bridge file."))?;
        Ok((
            format!("http://127.0.0.1:{}", info.port),
            format!("Bearer {}", info.token),
        ))
    }

    pub fn request(&self, method: Method, path: &str) -> Result<Value> {
        let (base, auth) = self.endpoint()?;
        let url = format!("{base}{path}");
        let sent = match method {
            Method::Get => self.agent.get(&url).header("Authorization", &auth).call(),
            Method::Delete => self
                .agent
                .delete(&url)
                .header("Authorization", &auth)
                .call(),
            Method::Post(body) => self
                .agent
                .post(&url)
                .header("Authorization", &auth)
                .header("Content-Type", "application/json")
                .send(body.to_string()),
        };
        let mut resp = sent.map_err(|_| anyhow!("Can't reach OpenWhispr. Is it running?"))?;
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        if status >= 400 {
            let message = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(String::from))
                .unwrap_or_else(|| format!("HTTP {status}"));
            return Err(anyhow!(message));
        }
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        let parsed: Value = serde_json::from_str(&text).context("Bad response from OpenWhispr")?;
        Ok(parsed["data"].clone())
    }
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Transcription {
    pub id: i64,
    pub timestamp: Option<String>,
    pub created_at: Option<String>,
    pub text: String,
    pub raw_text: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub audio_duration_ms: Option<i64>,
    pub status: String,
    pub client_transcription_id: Option<String>,
    #[serde(skip)]
    pub correction: Option<String>,
}

impl Transcription {
    pub fn when(&self) -> &str {
        self.timestamp
            .as_deref()
            .or(self.created_at.as_deref())
            .unwrap_or("")
    }

    /// What you meant: your correction, else the app's text.
    pub fn final_text(&self) -> &str {
        self.correction.as_deref().unwrap_or(&self.text)
    }

    /// What the speech engine wrote before AI cleanup.
    pub fn heard(&self) -> &str {
        self.raw_text
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.text)
    }

    pub fn cleaned_by_ai(&self) -> bool {
        self.raw_text
            .as_deref()
            .is_some_and(|r| r.trim() != self.text.trim())
    }

    pub fn changed(&self) -> bool {
        self.heard().trim() != self.final_text().trim()
    }

    pub fn engine(&self) -> &str {
        self.model
            .as_deref()
            .or(self.provider.as_deref())
            .unwrap_or("")
    }
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Snippet {
    pub trigger: String,
    pub replacement: String,
}

pub struct Store {
    pub bridge: Bridge,
    pub db_path: PathBuf,
    pub corrections_path: PathBuf,
}

impl Store {
    pub fn new(bridge_file: PathBuf, db_path: PathBuf, corrections_path: PathBuf) -> Self {
        Self {
            bridge: Bridge::new(bridge_file),
            db_path,
            corrections_path,
        }
    }

    pub fn default_paths() -> Self {
        let home = dirs::home_dir().unwrap_or_default();
        Self::new(
            home.join(".openwhispr/cli-bridge.json"),
            app_data_dir().join("transcriptions.db"),
            owtui_dir().join("corrections.json"),
        )
    }

    // ---- history

    /// Transcripts newest first, with corrections applied, and whether they came live.
    pub fn transcriptions(&self) -> Result<(Vec<Transcription>, bool)> {
        let (rows, live) = match self
            .bridge
            .request(Method::Get, "/v1/transcriptions/list?limit=5000")
        {
            Ok(rows) => (rows, true),
            Err(_) => (self.rows_from_db()?, false),
        };
        let mut items: Vec<Transcription> = serde_json::from_value(rows).unwrap_or_default();
        let fixes = self.corrections();
        for t in &mut items {
            let Some(fix) = fixes.get(&t.id.to_string()) else {
                continue;
            };
            // The client id guards against a recreated database reusing ids.
            let same = match fix.get("client_id") {
                None | Some(Value::Null) => true,
                Some(id) => id.as_str() == t.client_transcription_id.as_deref(),
            };
            if same {
                t.correction = fix.get("text").and_then(Value::as_str).map(String::from);
            }
        }
        Ok((items, live))
    }

    fn rows_from_db(&self) -> Result<Value> {
        if !self.db_path.is_file() {
            return Ok(Value::Array(vec![]));
        }
        let conn = rusqlite::Connection::open_with_flags(
            &self.db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(Duration::from_secs(5))?;
        let mut stmt = conn.prepare(
            "SELECT * FROM transcriptions WHERE deleted_at IS NULL AND status != 'discarded' \
             ORDER BY timestamp DESC LIMIT 5000",
        )?;
        let names: Vec<String> = stmt.column_names().into_iter().map(String::from).collect();
        let rows = stmt.query_map([], |row| {
            let mut obj = Map::new();
            for (i, name) in names.iter().enumerate() {
                use rusqlite::types::ValueRef;
                let value = match row.get_ref(i)? {
                    ValueRef::Null | ValueRef::Blob(_) => Value::Null,
                    ValueRef::Integer(n) => json!(n),
                    ValueRef::Real(f) => json!(f),
                    ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
                };
                obj.insert(name.clone(), value);
            }
            Ok(Value::Object(obj))
        })?;
        Ok(Value::Array(rows.collect::<rusqlite::Result<_>>()?))
    }

    /// The recording OpenWhispr kept for `t`: `audio/OpenWhispr-<time>-<id>.<ext>`.
    pub fn audio_path(&self, t: &Transcription) -> Option<PathBuf> {
        let dir = self.db_path.parent()?.join("audio");
        let suffix = format!("-{}", t.id);
        fs::read_dir(dir)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .find(|p| {
                p.file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|stem| stem.ends_with(&suffix))
            })
    }

    /// Size and modified time of the database and its write-ahead log.
    /// OpenWhispr writes every change there, so a different value means
    /// something changed; checking it costs a stat, not a request.
    pub fn data_signature(&self) -> Vec<Option<(u64, SystemTime)>> {
        let wal = self.db_path.with_file_name(format!(
            "{}-wal",
            self.db_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
        ));
        [&self.db_path, &wal]
            .iter()
            .map(|p| {
                fs::metadata(p)
                    .ok()
                    .and_then(|m| Some((m.len(), m.modified().ok()?)))
            })
            .collect()
    }

    pub fn delete_transcription(&self, t: &Transcription) -> Result<()> {
        self.bridge
            .request(Method::Delete, &format!("/v1/transcriptions/{}", t.id))?;
        self.correct(t, None)
    }

    // ---- corrections

    pub fn corrections(&self) -> Map<String, Value> {
        fs::read_to_string(&self.corrections_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default()
    }

    /// Saves `text` as the correction, or drops it for None or the app's own text.
    pub fn correct(&self, t: &Transcription, text: Option<&str>) -> Result<()> {
        let mut data = self.corrections();
        let key = t.id.to_string();
        match text {
            Some(text) if text.trim() != t.text.trim() => {
                data.insert(
                    key,
                    json!({
                        "text": text,
                        "app_text": t.text,
                        "raw_text": t.raw_text,
                        "client_id": t.client_transcription_id,
                        "timestamp": t.when(),
                        "edited_at": chrono::Utc::now().to_rfc3339(),
                    }),
                );
            }
            _ => {
                data.remove(&key);
            }
        }
        let dir = self
            .corrections_path
            .parent()
            .context("bad corrections path")?;
        fs::create_dir_all(dir)?;
        let tmp = dir.join(".corrections.json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(&Value::Object(data))?)?;
        fs::rename(&tmp, &self.corrections_path)?;
        Ok(())
    }

    // ---- training export

    pub fn export_pairs(&self, out: &Path) -> Result<usize> {
        let (items, _) = self.transcriptions()?;
        let mut file =
            fs::File::create(out).with_context(|| format!("can't write {}", out.display()))?;
        let mut count = 0;
        for t in items
            .iter()
            .filter(|t| t.changed() && t.status == "completed")
        {
            let pair = json!({
                "id": t.id,
                "timestamp": t.when(),
                "model": t.model,
                "input": t.heard(),
                "output": t.final_text(),
                "corrected": t.correction.is_some(),
            });
            writeln!(file, "{pair}")?;
            count += 1;
        }
        Ok(count)
    }

    // ---- dictionary: a list of words

    pub fn dictionary(&self) -> Result<Vec<String>> {
        Ok(serde_json::from_value(
            self.bridge.request(Method::Get, "/v1/dictionary/list")?,
        )?)
    }

    pub fn update_dictionary(&self, add: &[String], remove: &[String]) -> Result<()> {
        self.bridge.request(
            Method::Post(json!({ "add": add, "remove": remove })),
            "/v1/dictionary/update",
        )?;
        Ok(())
    }

    // ---- snippets: {trigger, replacement}

    pub fn snippets(&self) -> Result<Vec<Snippet>> {
        Ok(serde_json::from_value(
            self.bridge.request(Method::Get, "/v1/snippets/list")?,
        )?)
    }

    pub fn update_snippets(&self, add: &[Snippet], remove: &[String]) -> Result<()> {
        let add: Vec<Value> = add
            .iter()
            .map(|s| json!({ "trigger": s.trigger, "replacement": s.replacement }))
            .collect();
        self.bridge.request(
            Method::Post(json!({ "add": add, "remove": remove })),
            "/v1/snippets/update",
        )?;
        Ok(())
    }
}
