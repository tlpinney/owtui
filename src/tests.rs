//! Drives owtui against a fake OpenWhispr bridge; never touches real data.

use crate::app::{App, Modal, Tab};
use crate::editor::TextArea;
use crate::store::Store;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::{Arc, Mutex};

const TOKEN: &str = "t";

fn fake_bridge(dir: &Path) -> Arc<Mutex<Value>> {
    let state = Arc::new(Mutex::new(json!({
        "transcriptions": [
            {"id": 2, "timestamp": "2026-10-27 22:20:28.953Z", "text": "Run kube control get pods.",
             "raw_text": "run kube control get pods", "model": "parakeet", "audio_duration_ms": 3000,
             "status": "completed", "client_transcription_id": "c2"},
            {"id": 1, "timestamp": "2026-10-26 22:00:00.000Z", "text": "Hello there.",
             "raw_text": "Hello there.", "model": "parakeet", "status": "completed",
             "client_transcription_id": "c1"},
        ],
        "words": ["OpenWhispr"],
        "snippets": [{"trigger": "my link", "replacement": "https://example.com"}],
    })));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::fs::write(
        dir.join("bridge.json"),
        json!({"port": port, "token": TOKEN}).to_string(),
    )
    .unwrap();
    let shared = state.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            let mut parts = request.split_whitespace();
            let (method, path) = (
                parts.next().unwrap_or("").to_string(),
                parts.next().unwrap_or("").to_string(),
            );
            let (mut length, mut auth) = (0usize, String::new());
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                let header = header.trim_end();
                if header.is_empty() {
                    break;
                }
                if let Some((k, v)) = header.split_once(':') {
                    match k.to_ascii_lowercase().as_str() {
                        "content-length" => length = v.trim().parse().unwrap(),
                        "authorization" => auth = v.trim().to_string(),
                        _ => {}
                    }
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            assert_eq!(auth, format!("Bearer {TOKEN}"));

            let mut s = shared.lock().unwrap();
            let names = |v: &Value| -> Vec<String> {
                v.as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x.as_str().unwrap().to_lowercase())
                    .collect()
            };
            let data: Option<Value> = match (method.as_str(), path.as_str()) {
                ("GET", p) if p.starts_with("/v1/transcriptions/list") => {
                    Some(s["transcriptions"].clone())
                }
                ("GET", "/v1/dictionary/list") => Some(s["words"].clone()),
                ("GET", "/v1/snippets/list") => Some(s["snippets"].clone()),
                ("POST", "/v1/dictionary/update") => {
                    let remove = names(&body["remove"]);
                    let mut words: Vec<Value> = s["words"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|w| !remove.contains(&w.as_str().unwrap().to_lowercase()))
                        .cloned()
                        .collect();
                    words.extend(body["add"].as_array().unwrap().iter().cloned());
                    s["words"] = json!(words);
                    Some(json!({"words": words}))
                }
                ("POST", "/v1/snippets/update") => {
                    let mut gone = names(&body["remove"]);
                    gone.extend(
                        body["add"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|x| x["trigger"].as_str().unwrap().to_lowercase()),
                    );
                    let mut kept: Vec<Value> = s["snippets"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|x| !gone.contains(&x["trigger"].as_str().unwrap().to_lowercase()))
                        .cloned()
                        .collect();
                    kept.extend(body["add"].as_array().unwrap().iter().cloned());
                    s["snippets"] = json!(kept);
                    Some(json!({}))
                }
                ("DELETE", p) => {
                    let id: i64 = p.rsplit('/').next().unwrap().parse().unwrap();
                    let kept: Vec<Value> = s["transcriptions"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|t| t["id"] != id)
                        .cloned()
                        .collect();
                    s["transcriptions"] = json!(kept);
                    None
                }
                _ => Some(Value::Null),
            };
            drop(s);
            let (code, out) = match data {
                Some(d) => (200, json!({"data": d}).to_string()),
                None => (204, String::new()),
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {code} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}",
                out.len()
            );
        }
    });
    state
}

fn press(app: &mut App, code: KeyCode) {
    app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn ctrl(app: &mut App, c: char) {
    app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
}

fn type_text(app: &mut App, text: &str) {
    text.chars().for_each(|c| press(app, KeyCode::Char(c)));
}

#[test]
fn end_to_end_against_fake_bridge() {
    let dir = tempfile::tempdir().unwrap();
    let state = fake_bridge(dir.path());
    let store = Store::new(
        dir.path().join("bridge.json"),
        dir.path().join("none.db"),
        dir.path().join("c.json"),
    );
    let mut app = App::new(store, dir.path().to_path_buf());
    assert!(app.live);
    assert_eq!(app.visible().len(), 2);

    // Search, then correct the match.
    press(&mut app, KeyCode::Char('/'));
    type_text(&mut app, "kube");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.visible().len(), 1);
    press(&mut app, KeyCode::Char('e'));
    if let Some(Modal::Correct { editor, .. }) = app.modal.as_mut() {
        *editor = TextArea::new("Run kubectl get pods.", true);
    } else {
        panic!("correction editor didn't open");
    }
    ctrl(&mut app, 's');
    assert!(app.modal.is_none());
    assert_eq!(
        app.visible()[0].correction.as_deref(),
        Some("Run kubectl get pods.")
    );

    // Dictionary: add two, delete the selected first word.
    press(&mut app, KeyCode::Char('3'));
    assert_eq!(app.tab, Tab::Dictionary);
    press(&mut app, KeyCode::Char('a'));
    type_text(&mut app, "kubectl, Textual");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.words.as_ref().unwrap().len(), 3);
    press(&mut app, KeyCode::Char('d'));
    press(&mut app, KeyCode::Char('y'));

    // Snippets: rename a trigger.
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Char('e'));
    if let Some(Modal::Snippet { trigger, .. }) = app.modal.as_mut() {
        *trigger = TextArea::new("my site", false);
    }
    ctrl(&mut app, 's');

    // History: clear search, delete the older entry.
    press(&mut app, KeyCode::Char('1'));
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.visible().len(), 2);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Char('d'));
    press(&mut app, KeyCode::Char('y'));

    let s = state.lock().unwrap();
    assert_eq!(s["words"], json!(["kubectl", "Textual"]));
    assert_eq!(
        s["snippets"],
        json!([{"trigger": "my site", "replacement": "https://example.com"}])
    );
    assert_eq!(s["transcriptions"].as_array().unwrap().len(), 1);
    let fixes: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("c.json")).unwrap()).unwrap();
    assert_eq!(fixes["2"]["text"], "Run kubectl get pods.");
    assert_eq!(fixes["2"]["raw_text"], "run kube control get pods");
    assert_eq!(fixes["2"]["client_id"], "c2");
}

#[test]
fn offline_reads_db_and_exports() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("transcriptions.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE transcriptions (id INTEGER PRIMARY KEY, text TEXT, raw_text TEXT, timestamp TEXT,
           model TEXT, status TEXT, deleted_at TEXT, client_transcription_id TEXT, audio_duration_ms INTEGER);
         INSERT INTO transcriptions VALUES (1,'Hi Sam.','hi sam','2026-10-27 00:00:00','p','completed',NULL,'c1',1500);
         INSERT INTO transcriptions VALUES (2,'gone','gone','2026-10-27 00:00:00','p','completed','x','c2',NULL);",
    )
    .unwrap();
    drop(conn);
    let store = Store::new(dir.path().join("nope.json"), db, dir.path().join("c.json"));
    let (items, live) = store.transcriptions().unwrap();
    assert!(!live);
    assert_eq!(items.iter().map(|t| t.id).collect::<Vec<_>>(), [1]);
    assert_eq!(items[0].audio_duration_ms, Some(1500));

    let out = dir.path().join("pairs.jsonl");
    assert_eq!(store.export_pairs(&out).unwrap(), 1);
    let pair: Value = serde_json::from_str(std::fs::read_to_string(&out).unwrap().trim()).unwrap();
    assert_eq!(pair["input"], "hi sam");
    assert_eq!(pair["output"], "Hi Sam.");
    assert_eq!(pair["corrected"], false);
}

#[test]
fn draws_every_tab_and_modal() {
    use ratatui::{backend::TestBackend, Terminal};
    let dir = tempfile::tempdir().unwrap();
    let _state = fake_bridge(dir.path());
    let store = Store::new(
        dir.path().join("bridge.json"),
        dir.path().join("none.db"),
        dir.path().join("c.json"),
    );
    let mut app = App::new(store, dir.path().to_path_buf());
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    let mut screen = |app: &mut App| {
        terminal.draw(|f| crate::ui::draw(f, app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        buf.content().iter().map(|c| c.symbol()).collect::<String>()
    };
    let history = screen(&mut app);
    assert!(history.contains("2 transcripts") && history.contains("Run kube control"));
    for code in [KeyCode::Tab, KeyCode::Tab, KeyCode::Tab, KeyCode::Tab] {
        press(&mut app, code);
        screen(&mut app);
    }
    assert!(screen(&mut app).contains("Training pairs"));
    press(&mut app, KeyCode::Char('2'));
    assert!(screen(&mut app).contains("Repeated mistakes cover"));
    press(&mut app, KeyCode::Char('1'));
    press(&mut app, KeyCode::Char('e'));
    assert!(screen(&mut app).contains("What you meant"));
}

#[test]
fn quick_fix_and_insights_feed_the_dictionary() {
    let dir = tempfile::tempdir().unwrap();
    let state = fake_bridge(dir.path());
    let store = Store::new(
        dir.path().join("bridge.json"),
        dir.path().join("none.db"),
        dir.path().join("c.json"),
    );
    let mut app = App::new(store, dir.path().to_path_buf());

    // Correct the newest entry with the quick-fix: "kube control" -> "kubectl".
    press(&mut app, KeyCode::Char('e'));
    ctrl(&mut app, 'd');
    match app.modal.as_mut() {
        Some(Modal::Correct { quick: Some(q), .. }) => {
            assert_eq!(q.heard.text(), "", "cursor sits after the full stop");
            q.heard = TextArea::new("kube control", false);
        }
        _ => panic!("quick-fix box didn't open"),
    }
    type_text(&mut app, "kubectl");
    press(&mut app, KeyCode::Enter);
    match app.modal.as_ref() {
        Some(Modal::Correct {
            quick: None,
            editor,
            ..
        }) => {
            assert_eq!(editor.text(), "Run kubectl get pods.")
        }
        _ => panic!("quick-fix should close back to the editor"),
    }
    assert_eq!(
        state.lock().unwrap()["words"],
        json!(["OpenWhispr", "kubectl"])
    );
    ctrl(&mut app, 's');
    assert!(app.modal.is_none());

    // Insights picks the correction up, already in the dictionary.
    press(&mut app, KeyCode::Char('2'));
    let top = &app.insights.patterns[0];
    assert_eq!(
        (top.heard.as_str(), top.fixed.as_str(), top.by_you),
        ("kube control", "kubectl", 1)
    );
    assert!(top.in_dictionary);
    press(&mut app, KeyCode::Char('a'));
    assert_eq!(
        state.lock().unwrap()["words"].as_array().unwrap().len(),
        2,
        "no duplicate add"
    );

    // Enter jumps to the example on History.
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.tab, Tab::History);
    assert_eq!(app.selected_item().unwrap().id, 2);
}
