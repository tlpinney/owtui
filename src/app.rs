//! App state and key handling, kept free of terminal I/O so tests can drive it.

use crate::editor::TextArea;
use crate::insights::{self, Insights};
use crate::store::{Snippet, Store, Transcription};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::widgets::TableState;
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    History,
    Insights,
    Dictionary,
    Snippets,
    Stats,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Tab::History,
        Tab::Insights,
        Tab::Dictionary,
        Tab::Snippets,
        Tab::Stats,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Tab::History => "History",
            Tab::Insights => "Insights",
            Tab::Dictionary => "Dictionary",
            Tab::Snippets => "Snippets",
            Tab::Stats => "Stats",
        }
    }

    pub fn index(self) -> usize {
        Tab::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }
}

pub enum Modal {
    Correct {
        item: Transcription,
        editor: TextArea,
        quick: Option<Box<QuickFix>>,
    },
    Word {
        original: Option<String>,
        input: TextArea,
    },
    Snippet {
        original: Option<String>,
        trigger: TextArea,
        text: TextArea,
        on_text: bool,
    },
    Confirm {
        question: String,
        action: Pending,
    },
}

/// Fix a misheard phrase everywhere in the transcript and add the right word to the dictionary.
pub struct QuickFix {
    pub heard: TextArea,
    pub fixed: TextArea,
    pub on_fixed: bool,
}

pub enum Pending {
    Transcription(Box<Transcription>),
    Word(String),
    Snippet(String),
}

pub struct Status {
    pub text: String,
    pub error: bool,
    at: Instant,
}

impl Status {
    pub fn fresh(&self) -> bool {
        self.at.elapsed() < Duration::from_secs(6)
    }
}

pub struct App {
    pub store: Store,
    pub export_dir: PathBuf,
    pub tab: Tab,
    pub items: Vec<Transcription>,
    pub live: bool,
    pub search: TextArea,
    pub searching: bool,
    pub history: TableState,
    pub words: Result<Vec<String>, String>,
    pub dict: TableState,
    pub snippets: Result<Vec<Snippet>, String>,
    pub snips: TableState,
    pub insights: Insights,
    pub patterns: TableState,
    pub modal: Option<Modal>,
    pub status: Option<Status>,
    pub quit: bool,
}

fn clamp(state: &mut TableState, len: usize) {
    if len == 0 {
        state.select(None);
    } else {
        state.select(Some(state.selected().unwrap_or(0).min(len - 1)));
    }
}

impl App {
    pub fn new(store: Store, export_dir: PathBuf) -> Self {
        let mut app = Self {
            store,
            export_dir,
            tab: Tab::History,
            items: Vec::new(),
            live: false,
            search: TextArea::new("", false),
            searching: false,
            history: TableState::default(),
            words: Ok(Vec::new()),
            dict: TableState::default(),
            snippets: Ok(Vec::new()),
            snips: TableState::default(),
            insights: Insights::default(),
            patterns: TableState::default(),
            modal: None,
            status: None,
            quit: false,
        };
        app.reload();
        app
    }

    pub fn notify(&mut self, text: impl Into<String>) {
        self.status = Some(Status {
            text: text.into(),
            error: false,
            at: Instant::now(),
        });
    }

    pub fn fail(&mut self, text: impl Into<String>) {
        self.status = Some(Status {
            text: text.into(),
            error: true,
            at: Instant::now(),
        });
    }

    // ---- loading

    pub fn reload(&mut self) {
        self.load_history();
        self.load_dictionary();
        self.load_snippets();
    }

    fn load_history(&mut self) {
        match self.store.transcriptions() {
            Ok((items, live)) => {
                self.items = items;
                self.live = live;
            }
            Err(e) => self.fail(format!("Can't read history: {e}")),
        }
        self.clamp_history();
        self.refresh_insights();
    }

    fn load_dictionary(&mut self) {
        self.words = self.store.dictionary().map_err(|e| e.to_string());
        clamp(&mut self.dict, self.words.as_ref().map_or(0, Vec::len));
        self.refresh_insights();
    }

    fn refresh_insights(&mut self) {
        let words = self.words.as_deref().unwrap_or(&[]);
        self.insights = insights::analyze(&self.items, words);
        clamp(&mut self.patterns, self.insights.patterns.len());
    }

    /// Adds `word` unless the dictionary already has it; true when added.
    fn add_word(&mut self, word: &str) -> anyhow::Result<bool> {
        let known = self.words.as_deref().unwrap_or(&[]);
        if known
            .iter()
            .any(|w| w.to_lowercase() == word.to_lowercase())
        {
            return Ok(false);
        }
        self.store.update_dictionary(&[word.to_string()], &[])?;
        self.load_dictionary();
        Ok(true)
    }

    fn load_snippets(&mut self) {
        self.snippets = self.store.snippets().map_err(|e| e.to_string());
        clamp(&mut self.snips, self.snippets.as_ref().map_or(0, Vec::len));
    }

    fn clamp_history(&mut self) {
        let len = self.visible().len();
        clamp(&mut self.history, len);
    }

    /// History entries matching the search, newest first.
    pub fn visible(&self) -> Vec<&Transcription> {
        let needle = self.search.text().to_lowercase();
        self.items
            .iter()
            .filter(|t| {
                needle.is_empty()
                    || t.final_text().to_lowercase().contains(&needle)
                    || t.heard().to_lowercase().contains(&needle)
            })
            .collect()
    }

    pub fn selected_item(&self) -> Option<Transcription> {
        self.history
            .selected()
            .and_then(|i| self.visible().get(i).map(|t| (*t).clone()))
    }

    fn selected_word(&self) -> Option<String> {
        let words = self.words.as_ref().ok()?;
        self.dict.selected().and_then(|i| words.get(i).cloned())
    }

    fn selected_snippet(&self) -> Option<Snippet> {
        let snippets = self.snippets.as_ref().ok()?;
        self.snips.selected().and_then(|i| snippets.get(i).cloned())
    }

    // ---- keys

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if self.modal.is_some() {
            self.handle_modal(key);
            return;
        }
        if self.searching {
            match key.code {
                KeyCode::Enter | KeyCode::Esc => self.searching = false,
                _ => {
                    self.search.input(key);
                    self.history.select(Some(0));
                    self.clamp_history();
                }
            }
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('r') => {
                self.reload();
                self.notify("Reloaded.");
            }
            KeyCode::Char('x') => self.export(),
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => self.switch(1),
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                self.switch(Tab::ALL.len() - 1)
            }
            KeyCode::Char(c @ '1'..='5') => self.tab = Tab::ALL[c as usize - '1' as usize],
            KeyCode::Char('/') => {
                self.tab = Tab::History;
                self.searching = true;
            }
            KeyCode::Esc if self.tab == Tab::History && !self.search.text().is_empty() => {
                self.search = TextArea::new("", false);
                self.clamp_history();
            }
            KeyCode::Down | KeyCode::Char('j') => self.step(1),
            KeyCode::Up | KeyCode::Char('k') => self.step(-1),
            KeyCode::PageDown => self.step(10),
            KeyCode::PageUp => self.step(-10),
            KeyCode::Home | KeyCode::Char('g') => self.step(isize::MIN),
            KeyCode::End | KeyCode::Char('G') => self.step(isize::MAX),
            KeyCode::Char('e') | KeyCode::Enter => self.edit(),
            KeyCode::Char('a') => self.add(),
            KeyCode::Char('d') | KeyCode::Delete => self.delete(),
            _ => {}
        }
    }

    fn switch(&mut self, by: usize) {
        self.tab = Tab::ALL[(self.tab.index() + by) % Tab::ALL.len()];
    }

    fn step(&mut self, delta: isize) {
        let len = match self.tab {
            Tab::History => self.visible().len(),
            Tab::Insights => self.insights.patterns.len(),
            Tab::Dictionary => self.words.as_ref().map_or(0, Vec::len),
            Tab::Snippets => self.snippets.as_ref().map_or(0, Vec::len),
            Tab::Stats => return,
        };
        let state = match self.tab {
            Tab::History => &mut self.history,
            Tab::Insights => &mut self.patterns,
            Tab::Dictionary => &mut self.dict,
            _ => &mut self.snips,
        };
        if len == 0 {
            return;
        }
        let current = state.selected().unwrap_or(0) as isize;
        let next = current.saturating_add(delta).clamp(0, len as isize - 1);
        state.select(Some(next as usize));
    }

    fn selected_pattern(&self) -> Option<insights::Pattern> {
        self.patterns
            .selected()
            .and_then(|i| self.insights.patterns.get(i).cloned())
    }

    /// Shows a pattern's newest example on History, ready to correct.
    fn open_example(&mut self) {
        let Some(pattern) = self.selected_pattern() else {
            return;
        };
        self.search = TextArea::new("", false);
        let row = self.visible().iter().position(|t| t.id == pattern.example);
        self.history.select(row);
        self.tab = Tab::History;
    }

    fn edit(&mut self) {
        if self.tab == Tab::Insights {
            return self.open_example();
        }
        self.modal = match self.tab {
            Tab::History => self.selected_item().map(|item| Modal::Correct {
                editor: TextArea::new(item.final_text(), true),
                item,
                quick: None,
            }),
            Tab::Dictionary => self.selected_word().map(|w| Modal::Word {
                input: TextArea::new(&w, false),
                original: Some(w),
            }),
            Tab::Snippets => self.selected_snippet().map(|s| Modal::Snippet {
                trigger: TextArea::new(&s.trigger, false),
                text: TextArea::new(&s.replacement, true),
                original: Some(s.trigger),
                on_text: false,
            }),
            Tab::Insights | Tab::Stats => None,
        };
    }

    fn add(&mut self) {
        if self.tab == Tab::Insights {
            if let Some(pattern) = self.selected_pattern() {
                match self.add_word(&pattern.fixed) {
                    Ok(true) => {
                        self.notify(format!("Added “{}” to the dictionary.", pattern.fixed))
                    }
                    Ok(false) => {
                        self.notify(format!("“{}” is already in the dictionary.", pattern.fixed))
                    }
                    Err(e) => self.fail(e.to_string()),
                }
            }
            return;
        }
        self.modal = match self.tab {
            Tab::Dictionary => Some(Modal::Word {
                original: None,
                input: TextArea::new("", false),
            }),
            Tab::Snippets => Some(Modal::Snippet {
                original: None,
                trigger: TextArea::new("", false),
                text: TextArea::new("", true),
                on_text: false,
            }),
            _ => None,
        };
    }

    fn delete(&mut self) {
        let (question, action) = match self.tab {
            Tab::History => match self.selected_item() {
                Some(t) => (
                    format!(
                        "Delete this transcript from OpenWhispr?\n\n{}",
                        one_line(t.final_text(), 70)
                    ),
                    Pending::Transcription(Box::new(t)),
                ),
                None => return,
            },
            Tab::Dictionary => match self.selected_word() {
                Some(w) => (
                    format!("Remove “{w}” from the dictionary?"),
                    Pending::Word(w),
                ),
                None => return,
            },
            Tab::Snippets => match self.selected_snippet() {
                Some(s) => (
                    format!("Delete snippet “{}”?", s.trigger),
                    Pending::Snippet(s.trigger),
                ),
                None => return,
            },
            Tab::Insights | Tab::Stats => return,
        };
        self.modal = Some(Modal::Confirm { question, action });
    }

    fn handle_modal(&mut self, key: KeyEvent) {
        let Some(modal) = self.modal.take() else {
            return;
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let save = ctrl && key.code == KeyCode::Char('s');
        // Esc in the quick-fix box only closes the box.
        let quick_open = matches!(modal, Modal::Correct { quick: Some(_), .. });
        if key.code == KeyCode::Esc && !quick_open {
            return;
        }
        match modal {
            Modal::Confirm { question, action } => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => self.run(action),
                KeyCode::Char('n') | KeyCode::Char('N') => {}
                _ => self.modal = Some(Modal::Confirm { question, action }),
            },
            Modal::Correct {
                item,
                mut editor,
                quick: Some(mut q),
            } => {
                let mut quick = None;
                match key.code {
                    KeyCode::Esc => {}
                    KeyCode::Tab | KeyCode::BackTab => {
                        q.on_fixed = !q.on_fixed;
                        quick = Some(q);
                    }
                    KeyCode::Enter => quick = self.apply_quick_fix(&mut editor, q),
                    _ if save => quick = self.apply_quick_fix(&mut editor, q),
                    _ => {
                        if q.on_fixed {
                            q.fixed.input(key);
                        } else {
                            q.heard.input(key);
                        }
                        quick = Some(q);
                    }
                }
                self.modal = Some(Modal::Correct {
                    item,
                    editor,
                    quick,
                });
            }
            Modal::Correct {
                item, mut editor, ..
            } => {
                if ctrl && key.code == KeyCode::Char('d') {
                    let quick = Some(Box::new(QuickFix {
                        heard: TextArea::new(&editor.word_at_cursor(), false),
                        fixed: TextArea::new("", false),
                        on_fixed: true,
                    }));
                    self.modal = Some(Modal::Correct {
                        item,
                        editor,
                        quick,
                    });
                } else if save {
                    let text = editor.text();
                    let result = self.store.correct(&item, Some(&text));
                    self.finish(result, "Correction saved.", Tab::History);
                } else if ctrl && key.code == KeyCode::Char('r') && item.correction.is_some() {
                    let result = self.store.correct(&item, None);
                    self.finish(result, "Correction removed.", Tab::History);
                } else {
                    editor.input(key);
                    self.modal = Some(Modal::Correct {
                        item,
                        editor,
                        quick: None,
                    });
                }
            }
            Modal::Word {
                original,
                mut input,
            } => {
                if save || key.code == KeyCode::Enter {
                    let value = input.text();
                    let result = match &original {
                        Some(old) => {
                            let word = value.split_whitespace().collect::<Vec<_>>().join(" ");
                            if word.is_empty() || &word == old {
                                return;
                            }
                            self.store
                                .update_dictionary(&[word], std::slice::from_ref(old))
                        }
                        None => {
                            let words: Vec<String> = value
                                .split(',')
                                .map(|w| w.split_whitespace().collect::<Vec<_>>().join(" "))
                                .filter(|w| !w.is_empty())
                                .collect();
                            if words.is_empty() {
                                return;
                            }
                            self.store.update_dictionary(&words, &[])
                        }
                    };
                    self.finish(result, "Dictionary saved.", Tab::Dictionary);
                } else {
                    input.input(key);
                    self.modal = Some(Modal::Word { original, input });
                }
            }
            Modal::Snippet {
                original,
                mut trigger,
                mut text,
                mut on_text,
            } => {
                if save {
                    let snippet = Snippet {
                        trigger: trigger
                            .text()
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" "),
                        replacement: text.text().trim().to_string(),
                    };
                    if snippet.trigger.is_empty() || snippet.replacement.is_empty() {
                        self.fail("A trigger and text are required.");
                        self.modal = Some(Modal::Snippet {
                            original,
                            trigger,
                            text,
                            on_text,
                        });
                        return;
                    }
                    // A renamed trigger replaces the old snippet.
                    let remove: Vec<String> = original
                        .filter(|old| old.to_lowercase() != snippet.trigger.to_lowercase())
                        .into_iter()
                        .collect();
                    let result = self.store.update_snippets(&[snippet], &remove);
                    self.finish(result, "Snippet saved.", Tab::Snippets);
                    return;
                }
                match key.code {
                    KeyCode::Tab | KeyCode::BackTab => on_text = !on_text,
                    _ if on_text => {
                        text.input(key);
                    }
                    _ => {
                        trigger.input(key);
                    }
                }
                self.modal = Some(Modal::Snippet {
                    original,
                    trigger,
                    text,
                    on_text,
                });
            }
        }
    }

    /// Replaces the phrase in the editor and adds the fix to the dictionary.
    /// Returns the box when it should stay open.
    fn apply_quick_fix(
        &mut self,
        editor: &mut TextArea,
        q: Box<QuickFix>,
    ) -> Option<Box<QuickFix>> {
        let heard = q.heard.text().trim().to_string();
        let fixed = q
            .fixed
            .text()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if heard.is_empty() || fixed.is_empty() {
            self.fail("Fill in what was heard and what it should be.");
            return Some(q);
        }
        let (text, count) = insights::replace_phrase(&editor.text(), &heard, &fixed);
        editor.set_text(&text);
        let replaced = match count {
            0 => format!("“{heard}” not found"),
            1 => "Replaced 1".to_string(),
            n => format!("Replaced {n}"),
        };
        match self.add_word(&fixed) {
            Ok(true) => self.notify(format!(
                "{replaced} · added “{fixed}” to the dictionary · ^S saves"
            )),
            Ok(false) => self.notify(format!(
                "{replaced} · “{fixed}” already in the dictionary · ^S saves"
            )),
            Err(e) => self.fail(format!("{replaced}, but the dictionary: {e}")),
        }
        None
    }

    fn run(&mut self, action: Pending) {
        let (result, tab) = match &action {
            Pending::Transcription(t) => (self.store.delete_transcription(t), Tab::History),
            Pending::Word(w) => (
                self.store.update_dictionary(&[], std::slice::from_ref(w)),
                Tab::Dictionary,
            ),
            Pending::Snippet(s) => (
                self.store.update_snippets(&[], std::slice::from_ref(s)),
                Tab::Snippets,
            ),
        };
        self.finish(result, "Deleted.", tab);
    }

    fn finish(&mut self, result: anyhow::Result<()>, done: &str, tab: Tab) {
        match result {
            Ok(()) => {
                match tab {
                    Tab::Dictionary => self.load_dictionary(),
                    Tab::Snippets => self.load_snippets(),
                    _ => self.load_history(),
                }
                self.notify(done);
            }
            Err(e) => self.fail(e.to_string()),
        }
    }

    fn export(&mut self) {
        let name = format!(
            "openwhispr-pairs-{}.jsonl",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        );
        let out = self.export_dir.join(name);
        match self.store.export_pairs(&out) {
            Ok(n) => self.notify(format!("Wrote {n} pairs to {}", out.display())),
            Err(e) => self.fail(e.to_string()),
        }
    }
}

pub fn one_line(text: &str, width: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= width {
        flat
    } else {
        flat.chars()
            .take(width.saturating_sub(1))
            .collect::<String>()
            + "…"
    }
}

pub fn short_time(ts: &str) -> String {
    use chrono::{DateTime, Local, NaiveDateTime};
    let iso = ts.replacen(' ', "T", 1);
    if let Ok(dt) = DateTime::parse_from_rfc3339(&iso) {
        return dt
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M")
            .to_string();
    }
    // SQLite's CURRENT_TIMESTAMP: UTC without a zone.
    if let Ok(naive) = NaiveDateTime::parse_from_str(ts, "%Y-%m-%d %H:%M:%S") {
        return naive
            .and_utc()
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M")
            .to_string();
    }
    ts.chars().take(16).collect()
}
