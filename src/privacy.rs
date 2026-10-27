//! Where OpenWhispr's data goes: account and sync settings, which engines
//! run locally, and what has already left the machine.
//!
//! Everything is read from OpenWhispr's own files, read-only:
//! - `transcriptions.db`: per-row `cloud_id` (set once uploaded by sync),
//!   `sync_status`, and each transcript's speech `provider`.
//! - `Cookies`: an account session leaves cookies for openwhispr.com.
//! - `Local Storage/leveldb`: the app's settings (signed in, cloud backup,
//!   engines). Chromium keeps recent writes in uncompressed `.log` files and
//!   compacts older ones into `.ldb` tables; only the logs are read, so a
//!   setting that only lives in a table reads as unknown.
//!
//! Sync rule (OpenWhispr src/services/syncPassPolicy.ts): uploads need signed
//! in AND cloud backup on AND a subscription AND policy; usage stats need
//! signed in AND insights sync on.

use rusqlite::{Connection, OpenFlags};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Counts {
    pub transcripts: usize,
    pub words: usize,
    pub snippets: usize,
    pub notes: usize,
    pub usage_events: usize,
}

impl Counts {
    pub fn total(&self) -> usize {
        self.transcripts + self.words + self.snippets + self.notes + self.usage_events
    }
}

#[derive(Debug, Default)]
pub struct Privacy {
    pub signed_in: Option<bool>,
    pub session_cookies: usize,
    pub cloud_backup: Option<bool>,
    pub usage_sync: Option<bool>,
    /// "local" or a cloud engine name; None when unknown.
    pub speech: Option<String>,
    pub cleanup: Option<String>,
    /// Transcripts recognised by a cloud engine: count and first time.
    pub cloud_transcribed: usize,
    pub first_cloud: Option<String>,
    /// Rows sync has uploaded (they carry a cloud id).
    pub uploaded: Counts,
    /// Rows marked pending: they'd upload if syncing were turned on.
    pub pending: Counts,
    /// Some settings live only in compacted files owtui can't read.
    pub settings_partial: bool,
}

pub enum Verdict {
    Private(String),
    Warning(String),
    Alert(String),
}

impl Privacy {
    pub fn verdict(&self) -> Verdict {
        let cloud_engine =
            |e: &Option<String>| e.as_deref().is_some_and(|e| e != "local" && e != "off");
        if self.uploaded.total() > 0 {
            return Verdict::Alert(format!(
                "{} items have been uploaded to OpenWhispr's cloud.",
                self.uploaded.total()
            ));
        }
        if self.signed_in == Some(true) && self.cloud_backup == Some(true) {
            return Verdict::Alert(
                "Cloud backup is on and you're signed in: pending items will upload.".into(),
            );
        }
        if cloud_engine(&self.speech) || cloud_engine(&self.cleanup) {
            return Verdict::Warning(
                "A cloud engine is selected: new dictations leave this Mac to be processed.".into(),
            );
        }
        if self.signed_in == Some(true) || self.session_cookies > 0 {
            return Verdict::Warning(
                "Signed in. Nothing uploads unless cloud backup is turned on.".into(),
            );
        }
        if self.signed_in.is_none() || self.cloud_backup.is_none() && self.settings_partial {
            return Verdict::Warning(
                "Couldn't read every setting; check them in OpenWhispr.".into(),
            );
        }
        Verdict::Private(
            "Private: nothing uploaded, signed out, and both engines run on this Mac.".into(),
        )
    }
}

fn count(conn: &Connection, sql: &str) -> usize {
    conn.query_row(sql, [], |r| r.get::<_, i64>(0))
        .map_or(0, |n| n as usize)
}

fn open_ro(path: &Path) -> Option<Connection> {
    path.is_file().then_some(())?;
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()
}

pub fn read(app_dir: &Path) -> Privacy {
    let mut p = Privacy::default();

    if let Some(db) = open_ro(&app_dir.join("transcriptions.db")) {
        let tables = [
            ("transcriptions", 0),
            ("custom_dictionary", 1),
            ("snippets", 2),
            ("notes", 3),
            ("analytics_events", 4),
        ];
        for (table, slot) in tables {
            let has_cloud_id = count(
                &db,
                &format!(
                    "SELECT count(*) FROM pragma_table_info('{table}') WHERE name = 'cloud_id'"
                ),
            ) > 0;
            let uploaded = if has_cloud_id {
                count(
                    &db,
                    &format!("SELECT count(*) FROM {table} WHERE cloud_id IS NOT NULL"),
                )
            } else {
                count(
                    &db,
                    &format!("SELECT count(*) FROM {table} WHERE sync_status = 'synced'"),
                )
            };
            let pending = count(
                &db,
                &format!("SELECT count(*) FROM {table} WHERE sync_status = 'pending' AND deleted_at IS NULL"),
            );
            for (counts, n) in [(&mut p.uploaded, uploaded), (&mut p.pending, pending)] {
                match slot {
                    0 => counts.transcripts = n,
                    1 => counts.words = n,
                    2 => counts.snippets = n,
                    3 => counts.notes = n,
                    _ => counts.usage_events = n,
                }
            }
        }
        // Local engines record provider "local-…"; anything else ran in a cloud.
        let cloud = "provider IS NOT NULL AND provider NOT LIKE 'local%'";
        p.cloud_transcribed = count(
            &db,
            &format!("SELECT count(*) FROM transcriptions WHERE {cloud}"),
        );
        p.first_cloud = db
            .query_row(
                &format!("SELECT min(timestamp) FROM transcriptions WHERE {cloud}"),
                [],
                |r| r.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten();
    }

    if let Some(cookies) = open_ro(&app_dir.join("Cookies")) {
        p.session_cookies = count(
            &cookies,
            "SELECT count(*) FROM cookies WHERE host_key LIKE '%openwhispr%'",
        );
    }

    let (settings, partial) = local_storage(&app_dir.join("Local Storage/leveldb"));
    p.settings_partial = partial;
    let flag = |key: &str| settings.get(key).map(|v| v == "true");
    p.signed_in = flag("isSignedIn");
    p.usage_sync = flag("insightsSyncEnabled");
    // Never set means never turned on, unless older settings are unreadable.
    p.cloud_backup = flag("cloudBackupEnabled").or((!partial).then_some(false));
    p.speech = match flag("useLocalWhisper") {
        Some(true) => Some("local".into()),
        Some(false) => Some("OpenWhispr Cloud or an API".into()),
        None => None,
    };
    p.cleanup = match (
        flag("useCleanupModel"),
        settings.get("cleanupMode").map(String::as_str),
    ) {
        (Some(false), _) => Some("off".into()),
        (_, Some("local")) => Some("local".into()),
        (_, Some(mode)) => Some(mode.to_string()),
        _ => None,
    };
    p
}

/// The settings in Chromium's localStorage logs, latest value per key, and
/// whether compacted tables (which may hold older settings) exist.
pub fn local_storage(dir: &Path) -> (HashMap<String, String>, bool) {
    let mut settings = HashMap::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return (settings, false);
    };
    let mut logs = Vec::new();
    let mut partial = false;
    for path in entries.flatten().map(|e| e.path()) {
        match path.extension().and_then(|e| e.to_str()) {
            Some("log") => logs.push(path),
            Some("ldb") => partial = true,
            _ => {}
        }
    }
    // Log files are numbered; older first, so later writes win.
    logs.sort();
    for log in logs {
        if let Ok(data) = fs::read(&log) {
            for batch in log_records(&data) {
                apply_batch(&batch, &mut settings);
            }
        }
    }
    (settings, partial)
}

/// Reassembles LevelDB log records: 32 KiB blocks of fragments, each with a
/// 7-byte header (checksum, length, type: 1 full, 2 first, 3 middle, 4 last).
fn log_records(data: &[u8]) -> Vec<Vec<u8>> {
    const BLOCK: usize = 32 * 1024;
    let mut records = Vec::new();
    let mut pending: Vec<u8> = Vec::new();
    let mut pos = 0;
    while pos + 7 <= data.len() {
        let left_in_block = BLOCK - pos % BLOCK;
        if left_in_block < 7 {
            pos += left_in_block;
            continue;
        }
        let len = u16::from_le_bytes([data[pos + 4], data[pos + 5]]) as usize;
        let kind = data[pos + 6];
        let start = pos + 7;
        let Some(fragment) = data.get(start..start + len) else {
            break;
        };
        match kind {
            1 => records.push(fragment.to_vec()),
            2 => pending = fragment.to_vec(),
            3 => pending.extend_from_slice(fragment),
            4 => {
                pending.extend_from_slice(fragment);
                records.push(std::mem::take(&mut pending));
            }
            _ => {}
        }
        pos = start + len;
    }
    records
}

fn varint(data: &[u8], pos: &mut usize) -> Option<usize> {
    let (mut value, mut shift) = (0usize, 0);
    loop {
        let byte = *data.get(*pos)?;
        *pos += 1;
        value |= ((byte & 0x7f) as usize) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
        shift += 7;
        if shift > 35 {
            return None;
        }
    }
}

/// A write batch: sequence (8 bytes), count (4), then entries of
/// tag (1 put, 0 delete), key, and for puts a value.
fn apply_batch(batch: &[u8], settings: &mut HashMap<String, String>) {
    let mut pos = 12;
    while pos < batch.len() {
        let tag = batch[pos];
        pos += 1;
        let Some(key) = varint(batch, &mut pos).and_then(|n| batch.get(pos..pos + n)) else {
            return;
        };
        pos += key.len();
        let name = setting_name(key);
        match tag {
            1 => {
                let Some(value) = varint(batch, &mut pos).and_then(|n| batch.get(pos..pos + n))
                else {
                    return;
                };
                pos += value.len();
                if let Some(name) = name {
                    settings.insert(name, decode(value));
                }
            }
            0 => {
                if let Some(name) = name {
                    settings.remove(&name);
                }
            }
            _ => return,
        }
    }
}

/// Chromium keys are `_<origin>\0<encoding><name>`; only the name matters.
fn setting_name(key: &[u8]) -> Option<String> {
    let rest = key.strip_prefix(b"_")?;
    let at = rest.iter().position(|&b| b == 0)?;
    let name = rest.get(at + 1..)?;
    Some(decode(name))
}

/// Values start with an encoding byte: 1 Latin-1, 0 UTF-16LE.
fn decode(value: &[u8]) -> String {
    match value.split_first() {
        Some((1, rest)) => rest.iter().map(|&b| b as char).collect(),
        Some((0, rest)) => String::from_utf16_lossy(
            &rest
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect::<Vec<_>>(),
        ),
        _ => String::from_utf8_lossy(value).into_owned(),
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Writes a Chromium-style localStorage log with the given puts (None deletes).
    pub fn write_log(path: &Path, entries: &[(&str, Option<&str>)]) {
        let mut batch = vec![0u8; 12];
        for (name, value) in entries {
            let mut key = b"_file://\x00\x01".to_vec();
            key.extend_from_slice(name.as_bytes());
            batch.push(u8::from(value.is_some()));
            batch.push(key.len() as u8);
            batch.extend_from_slice(&key);
            if let Some(value) = value {
                batch.push(value.len() as u8 + 1);
                batch.push(1);
                batch.extend_from_slice(value.as_bytes());
            }
        }
        let mut data = vec![0, 0, 0, 0];
        data.extend_from_slice(&(batch.len() as u16).to_le_bytes());
        data.push(1);
        data.extend_from_slice(&batch);
        fs::write(path, data).unwrap();
    }

    #[test]
    fn reads_latest_settings_from_the_log() {
        let dir = tempfile::tempdir().unwrap();
        write_log(
            &dir.path().join("000003.log"),
            &[
                ("isSignedIn", Some("true")),
                ("cleanupMode", Some("openwhispr")),
                ("isSignedIn", Some("false")),
                ("cleanupMode", Some("local")),
                ("gone", Some("x")),
                ("gone", None),
            ],
        );
        let (settings, partial) = local_storage(dir.path());
        assert!(!partial);
        assert_eq!(
            settings.get("isSignedIn").map(String::as_str),
            Some("false")
        );
        assert_eq!(
            settings.get("cleanupMode").map(String::as_str),
            Some("local")
        );
        assert!(!settings.contains_key("gone"));
    }

    #[test]
    fn reads_records_split_across_fragments() {
        let mut data = Vec::new();
        let batch = {
            let mut b = vec![0u8; 12];
            let key = b"_file://\x00\x01isSignedIn";
            b.push(1);
            b.push(key.len() as u8);
            b.extend_from_slice(key);
            b.extend_from_slice(&[5, 1]);
            b.extend_from_slice(b"true");
            b
        };
        let (a, b) = batch.split_at(10);
        for (kind, part) in [(2u8, a), (4u8, b)] {
            data.extend_from_slice(&[0, 0, 0, 0]);
            data.extend_from_slice(&(part.len() as u16).to_le_bytes());
            data.push(kind);
            data.extend_from_slice(part);
        }
        let mut settings = HashMap::new();
        for record in log_records(&data) {
            apply_batch(&record, &mut settings);
        }
        assert_eq!(settings.get("isSignedIn").map(String::as_str), Some("true"));
    }
}
