//! Drawing.

use crate::app::{one_line, short_time, App, Modal, Tab};
use crate::editor::TextArea;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap};
use ratatui::Frame;
use std::collections::BTreeMap;

const ACCENT: Color = Color::Cyan;

fn selected() -> Style {
    Style::new()
        .bg(Color::Blue)
        .fg(Color::White)
        .add_modifier(Modifier::BOLD)
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let [top, body, foot] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    let mode = if app.live {
        Span::styled(" live ", Style::new().fg(Color::Green))
    } else {
        Span::styled(
            " read-only: OpenWhispr isn't running ",
            Style::new().fg(Color::Yellow),
        )
    };
    // Tabs and the app label share the first row; a block title there would
    // take a row of its own and push the tabs behind the border.
    let label = Line::from(vec![" OpenWhispr ".bold(), mode]).right_aligned();
    let [row, rule] = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(top);
    let [tabs_area, label_area] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(label.width() as u16)])
            .areas(row);
    // Drawn by hand so each tab's cells are known and clicks can switch to it.
    app.tab_areas.clear();
    let mut x = tabs_area.x;
    for (i, tab) in Tab::ALL.iter().enumerate() {
        let text = format!(" {} {} ", i + 1, tab.title());
        let width = Line::raw(text.as_str()).width() as u16;
        if x + width > tabs_area.right() {
            break;
        }
        let style = if *tab == app.tab {
            Style::new()
                .fg(ACCENT)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::new()
        };
        let rect = Rect::new(x, tabs_area.y, width, 1);
        frame.render_widget(Span::styled(text, style), rect);
        app.tab_areas.push((rect, *tab));
        x += width;
        if i + 1 < Tab::ALL.len() && x < tabs_area.right() {
            frame.render_widget(
                Span::styled("│", Style::new().dark_gray()),
                Rect::new(x, tabs_area.y, 1, 1),
            );
            x += 1;
        }
    }
    frame.render_widget(label, label_area);
    frame.render_widget(Block::new().borders(Borders::TOP), rule);

    app.rows_area = Rect::default();
    match app.tab {
        Tab::History => draw_history(frame, app, body),
        Tab::Insights => draw_insights(frame, app, body),
        Tab::Dictionary => draw_dictionary(frame, app, body),
        Tab::Snippets => draw_snippets(frame, app, body),
        Tab::Stats => draw_stats(frame, app, body),
    }
    draw_footer(frame, app, foot);
    draw_modal(frame, app);
}

/// The cells inside a bordered table that hold its rows.
fn rows_area(table: Rect, header: bool) -> Rect {
    let top = 1 + u16::from(header);
    Rect::new(
        table.x + 1,
        table.y + top,
        table.width.saturating_sub(2),
        table.height.saturating_sub(top + 1),
    )
}

fn draw_history(frame: &mut Frame, app: &mut App, area: Rect) {
    let [search, main] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
    let query = app.search.text();
    if app.searching {
        let [label, field] =
            Layout::horizontal([Constraint::Length(9), Constraint::Min(0)]).areas(search);
        frame.render_widget(Span::styled(" Search: ", Style::new().fg(ACCENT)), label);
        app.search.render(frame, field, Block::new(), true);
    } else if query.is_empty() {
        frame.render_widget(
            Span::styled(" / to search", Style::new().dark_gray()),
            search,
        );
    } else {
        frame.render_widget(
            Line::from(vec![
                Span::styled(" Search: ", Style::new().fg(ACCENT)),
                Span::raw(query),
                Span::styled("   (Esc clears)", Style::new().dark_gray()),
            ]),
            search,
        );
    }

    let [list, preview] =
        Layout::horizontal([Constraint::Percentage(62), Constraint::Percentage(38)]).areas(main);
    let visible = app.visible();
    let count = visible.len();
    let rows: Vec<Row> = visible
        .iter()
        .map(|t| {
            let secs = t
                .audio_duration_ms
                .map(|ms| format!("{:.0}", ms as f64 / 1000.0));
            let mark = if t.correction.is_some() { "✎ " } else { "" };
            let playing = if app.player.playing == Some(t.id) {
                "▶ "
            } else {
                ""
            };
            Row::new(vec![
                Cell::from(short_time(t.when())),
                Cell::from(t.engine().to_string()),
                Cell::from(secs.unwrap_or_default()),
                Cell::from(Line::from(vec![
                    Span::styled(playing, Style::new().fg(Color::Green).bold()),
                    Span::styled(mark, Style::new().fg(Color::Yellow)),
                    Span::raw(one_line(t.final_text(), 200)),
                ])),
            ])
        })
        .collect();
    let item = app.selected_item();

    let table = Table::new(
        rows,
        [
            Constraint::Length(16),
            Constraint::Length(16),
            Constraint::Length(4),
            Constraint::Min(10),
        ],
    )
    .header(Row::new(["When", "Model", "Sec", "Text"]).style(Style::new().fg(ACCENT).bold()))
    .row_highlight_style(selected())
    .highlight_symbol("▶ ")
    .block(Block::bordered().title(format!(" {count} transcripts ")));
    frame.render_stateful_widget(table, list, &mut app.history);
    app.rows_area = rows_area(list, true);

    let outer = Block::bordered().title(" Preview · click a word to edit there ");
    let inner = outer.inner(preview);
    frame.render_widget(outer, preview);
    let Some(t) = item else {
        app.preview = None;
        return;
    };
    let (head, extras) = preview_text(&t);
    let extras_height = wrapped_height(&extras, inner.width).min(inner.height / 2);
    let [head_area, body, extras_area] = Layout::vertical([
        Constraint::Length(head.len() as u16),
        Constraint::Min(1),
        Constraint::Length(extras_height),
    ])
    .areas(inner);
    frame.render_widget(Paragraph::new(head), head_area);
    // Drawn with the editor's own wrapping, so a click maps to a text position.
    let key = (t.id, t.final_text().to_string());
    if app.preview.as_ref().map(|(k, _)| k) != Some(&key) {
        let mut text = TextArea::new(t.final_text(), true);
        text.set_cursor(0, 0);
        app.preview = Some((key, text));
    }
    if let Some((_, text)) = app.preview.as_mut() {
        text.render(frame, body, Block::new(), false);
    }
    frame.render_widget(
        Paragraph::new(extras).wrap(Wrap { trim: false }),
        extras_area,
    );
}

fn wrapped_height(lines: &[Line], width: u16) -> u16 {
    let width = usize::from(width.max(1));
    lines
        .iter()
        .map(|l| l.width().div_ceil(width).max(1) as u16)
        .sum()
}

/// The preview's header (above the clickable text) and extras (below it).
fn preview_text(t: &crate::store::Transcription) -> (Vec<Line<'static>>, Vec<Line<'static>>) {
    let dim = Style::new().dark_gray();
    let mut meta = vec![short_time(t.when()), t.engine().to_string()];
    if let Some(ms) = t.audio_duration_ms {
        meta.push(format!("{:.1}s", ms as f64 / 1000.0));
    }
    let mut head = vec![Line::styled(meta.join(" · "), dim), Line::raw("")];
    let mut extras = Vec::new();
    if t.correction.is_some() {
        head.push(Line::styled(
            "Your correction",
            Style::new().fg(Color::Yellow).bold(),
        ));
        extras.push(Line::styled("── app saved ──", dim));
        extras.extend(t.text.lines().map(|l| Line::styled(l.to_string(), dim)));
    }
    if t.cleaned_by_ai() {
        extras.push(Line::styled("── heard (before cleanup) ──", dim));
        extras.extend(t.heard().lines().map(|l| Line::styled(l.to_string(), dim)));
    }
    (head, extras)
}

fn draw_insights(frame: &mut Frame, app: &mut App, area: Rect) {
    let [summary, list, example] = Layout::vertical([
        Constraint::Length(5),
        Constraint::Min(5),
        Constraint::Length(6),
    ])
    .areas(area);
    let ins = &app.insights;
    let (verdict, fine_tune) = ins.verdict();
    let dim = Style::new().dark_gray();
    let summary_lines = vec![
        Line::from(vec![
            Span::styled("  Changed transcripts ", dim),
            Span::raw(ins.compared.to_string()),
            Span::styled("   corrected by you ", dim),
            Span::raw(ins.corrected_by_you.to_string()),
            Span::styled("   word errors ", dim),
            Span::raw(ins.word_errors.to_string()),
        ]),
        Line::from(vec![
            Span::styled("  Repeated mistakes cover ", dim),
            Span::raw(format!("{:.0}%", ins.coverage() * 100.0)),
            Span::styled(" of word errors", dim),
        ]),
        Line::styled(
            format!("  {verdict}"),
            Style::new()
                .fg(if fine_tune {
                    Color::Yellow
                } else {
                    Color::Green
                })
                .bold(),
        ),
    ];
    frame.render_widget(
        Paragraph::new(summary_lines).block(Block::bordered().title(" Insights ")),
        summary,
    );

    let rows: Vec<Row> = ins
        .patterns
        .iter()
        .map(|p| {
            let dict = if p.in_dictionary {
                Span::styled("✓ in dictionary", Style::new().fg(Color::Green))
            } else {
                Span::styled("a to add", dim)
            };
            Row::new(vec![
                Cell::from(p.count.to_string()),
                Cell::from(if p.by_you > 0 { "you" } else { "AI" }),
                Cell::from(Span::styled(p.heard.clone(), Style::new().fg(Color::Red))),
                Cell::from(Span::styled(
                    p.fixed.clone(),
                    Style::new().fg(Color::Green).bold(),
                )),
                Cell::from(dict),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [
            Constraint::Length(5),
            Constraint::Length(4),
            Constraint::Percentage(35),
            Constraint::Percentage(30),
            Constraint::Min(16),
        ],
    )
    .header(
        Row::new(["Seen", "By", "Heard", "Should be", "Dictionary"])
            .style(Style::new().fg(ACCENT).bold()),
    )
    .row_highlight_style(selected())
    .highlight_symbol("▶ ")
    .block(Block::bordered().title(format!(" {} mistakes ", ins.patterns.len())));
    frame.render_stateful_widget(table, list, &mut app.patterns);
    app.rows_area = rows_area(list, true);

    let pattern = app
        .patterns
        .selected()
        .and_then(|i| app.insights.patterns.get(i));
    let lines = pattern
        .and_then(|p| app.items.iter().find(|t| t.id == p.example))
        .map(|t| {
            vec![
                Line::from(vec![
                    Span::styled("heard  ", dim),
                    Span::raw(one_line(t.heard(), 300)),
                ]),
                Line::from(vec![
                    Span::styled("final  ", dim),
                    Span::raw(one_line(t.final_text(), 300)),
                ]),
            ]
        })
        .unwrap_or_default();
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(Block::bordered().title(" Newest example · Enter opens it ")),
        example,
    );
}

fn error_box(frame: &mut Frame, area: Rect, title: &str, message: &str) {
    frame.render_widget(
        Paragraph::new(message.to_string())
            .style(Style::new().fg(Color::Yellow))
            .block(Block::bordered().title(format!(" {title} "))),
        area,
    );
}

fn draw_dictionary(frame: &mut Frame, app: &mut App, area: Rect) {
    let words = match &app.words {
        Ok(words) => words,
        Err(e) => return error_box(frame, area, "Dictionary", e),
    };
    let title = format!(" {} words · a add (comma-separated) ", words.len());
    let rows: Vec<Row> = words.iter().map(|w| Row::new([w.clone()])).collect();
    let table = Table::new(rows, [Constraint::Min(10)])
        .row_highlight_style(selected())
        .highlight_symbol("▶ ")
        .block(Block::bordered().title(title));
    frame.render_stateful_widget(table, area, &mut app.dict);
    app.rows_area = rows_area(area, false);
}

fn draw_snippets(frame: &mut Frame, app: &mut App, area: Rect) {
    let snippets = match &app.snippets {
        Ok(s) => s,
        Err(e) => return error_box(frame, area, "Snippets", e),
    };
    let title = format!(" {} snippets ", snippets.len());
    let rows: Vec<Row> = snippets
        .iter()
        .map(|s| Row::new([s.trigger.clone(), one_line(&s.replacement, 200)]))
        .collect();
    let table = Table::new(
        rows,
        [Constraint::Percentage(30), Constraint::Percentage(70)],
    )
    .header(Row::new(["Trigger", "Replacement"]).style(Style::new().fg(ACCENT).bold()))
    .row_highlight_style(selected())
    .highlight_symbol("▶ ")
    .block(Block::bordered().title(title));
    frame.render_stateful_widget(table, area, &mut app.snips);
    app.rows_area = rows_area(area, true);
}

fn draw_stats(frame: &mut Frame, app: &App, area: Rect) {
    let items = &app.items;
    let audio_min = items
        .iter()
        .filter_map(|t| t.audio_duration_ms)
        .sum::<i64>() as f64
        / 60_000.0;
    let words: usize = items
        .iter()
        .map(|t| t.final_text().split_whitespace().count())
        .sum();
    let mut models: BTreeMap<&str, usize> = BTreeMap::new();
    for t in items {
        *models.entry(t.engine()).or_default() += 1;
    }
    let mut models: Vec<_> = models.into_iter().collect();
    models.sort_by_key(|m| std::cmp::Reverse(m.1));

    let row = |label: &str, value: String| {
        Line::from(vec![
            Span::styled(format!("  {label:<20}"), Style::new().dark_gray()),
            Span::raw(value),
        ])
    };
    let mut lines = vec![
        Line::raw(""),
        row("Transcripts", items.len().to_string()),
        row("Words", words.to_string()),
        row("Audio", format!("{audio_min:.1} min")),
        row(
            "First",
            items
                .last()
                .map(|t| short_time(t.when()))
                .unwrap_or("-".into()),
        ),
        row(
            "Latest",
            items
                .first()
                .map(|t| short_time(t.when()))
                .unwrap_or("-".into()),
        ),
        row(
            "Cleaned by AI",
            items
                .iter()
                .filter(|t| t.cleaned_by_ai())
                .count()
                .to_string(),
        ),
        row(
            "Your corrections",
            items
                .iter()
                .filter(|t| t.correction.is_some())
                .count()
                .to_string(),
        ),
        row(
            "Training pairs",
            items
                .iter()
                .filter(|t| t.changed() && t.status == "completed")
                .count()
                .to_string(),
        ),
        Line::raw(""),
        Line::styled("  Models", Style::new().fg(ACCENT).bold()),
    ];
    lines.extend(
        models
            .into_iter()
            .map(|(m, n)| row(if m.is_empty() { "?" } else { m }, n.to_string())),
    );
    lines.push(Line::raw(""));
    lines.push(row(
        "Corrections file",
        app.store.corrections_path.display().to_string(),
    ));
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" Stats ")),
        area,
    );
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    if let Some(status) = app.status.as_ref().filter(|s| s.fresh()) {
        let color = if status.error {
            Color::Red
        } else {
            Color::Green
        };
        frame.render_widget(
            Span::styled(format!(" {}", status.text), Style::new().fg(color)),
            area,
        );
        return;
    }
    let hints: &[(&str, &str)] = match (&app.modal, app.tab) {
        (Some(Modal::Correct { quick: Some(_), .. }), _) => &[
            ("Tab", "switch field"),
            ("Enter", "replace + add to dictionary"),
            ("Esc", "back"),
        ],
        (Some(Modal::Correct { editor, .. }), _) if editor.has_selection() => &[
            ("type", "replace word"),
            ("^D", "fix everywhere → dictionary"),
            ("^S", "save"),
            ("Esc", "cancel"),
        ],
        (Some(Modal::Correct { item, .. }), _) if item.correction.is_some() => &[
            ("^S", "save"),
            ("^D", "fix word → dictionary"),
            ("^P", "play"),
            ("⌥←→", "word"),
            ("^R", "revert to app's text"),
            ("Esc", "cancel"),
        ],
        (Some(Modal::Correct { .. }), _) => &[
            ("^S", "save"),
            ("^D", "fix word → dictionary"),
            ("^P", "play"),
            ("⌥←→", "word"),
            ("Esc", "cancel"),
        ],
        (Some(Modal::Word { .. }), _) => &[("Enter/^S", "save"), ("Esc", "cancel")],
        (Some(Modal::Snippet { .. }), _) => {
            &[("Tab", "switch field"), ("^S", "save"), ("Esc", "cancel")]
        }
        (Some(Modal::Confirm { .. }), _) => &[("y", "yes"), ("n/Esc", "no")],
        (None, _) if app.searching => &[("type", "filter"), ("Enter/Esc", "back to list")],
        (None, Tab::History) => &[
            ("↑↓", "move"),
            ("e", "correct"),
            ("p", "play"),
            ("d", "delete"),
            ("/", "search"),
            ("x", "export"),
            ("Tab", "next tab"),
            ("r", "reload"),
            ("q", "quit"),
        ],
        (None, Tab::Insights) => &[
            ("↑↓", "move"),
            ("a", "add fix to dictionary"),
            ("p", "play example"),
            ("Enter", "open example"),
            ("Tab", "next tab"),
            ("q", "quit"),
        ],
        (None, Tab::Dictionary) | (None, Tab::Snippets) => &[
            ("↑↓", "move"),
            ("a", "add"),
            ("e", "edit"),
            ("d", "delete"),
            ("Tab", "next tab"),
            ("r", "reload"),
            ("q", "quit"),
        ],
        (None, Tab::Stats) => &[("x", "export pairs"), ("Tab", "next tab"), ("q", "quit")],
    };
    let mut spans = vec![Span::raw(" ")];
    for (k, label) in hints {
        spans.push(Span::styled(*k, Style::new().fg(ACCENT).bold()));
        spans.push(Span::styled(
            format!(" {label}  "),
            Style::new().dark_gray(),
        ));
    }
    frame.render_widget(Line::from(spans), area);
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let (w, h) = (width.min(area.width), height.min(area.height));
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

fn focus_block(title: &str, focused: bool) -> Block<'static> {
    let style = if focused {
        Style::new().fg(ACCENT)
    } else {
        Style::new().dark_gray()
    };
    Block::bordered()
        .border_style(style)
        .title(format!(" {title} "))
}

#[derive(Clone, Copy)]
enum Look {
    Primary,
    Plain,
    Danger,
}

/// One clickable button: its label, the key it stands for, and its look.
type ButtonSpec<'a> = (&'a str, KeyEvent, Look);

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

/// Draws a row of buttons and records where each one is, so a click can
/// replay its key.
fn draw_buttons(
    frame: &mut Frame,
    area: Rect,
    buttons: &[ButtonSpec],
    hits: &mut Vec<(Rect, KeyEvent)>,
) {
    let mut x = area.x;
    for (label, key, look) in buttons {
        let text = format!(" {label} ");
        let width = Line::raw(text.as_str()).width() as u16;
        if x + width > area.right() {
            break;
        }
        let style = match look {
            Look::Primary => Style::new().bg(Color::Blue).fg(Color::White).bold(),
            Look::Danger => Style::new().bg(Color::Red).fg(Color::White).bold(),
            Look::Plain => Style::new().bg(Color::DarkGray).fg(Color::White),
        };
        let rect = Rect::new(x, area.y, width, 1);
        frame.render_widget(Span::styled(text, style), rect);
        hits.push((rect, *key));
        x += width + 2;
    }
}

fn draw_modal(frame: &mut Frame, app: &mut App) {
    let screen = frame.area();
    let mut hits = Vec::new();
    let Some(modal) = app.modal.as_mut() else {
        app.buttons = hits;
        return;
    };
    match modal {
        Modal::Correct {
            item,
            editor,
            quick,
        } => {
            let area = centered(screen, screen.width * 9 / 10, screen.height * 8 / 10);
            frame.render_widget(Clear, area);
            let outer = Block::bordered()
                .border_style(Style::new().fg(ACCENT))
                .title(format!(
                    " Correct · {} · {} ",
                    short_time(item.when()),
                    item.engine()
                ));
            let inner = outer.inner(area);
            frame.render_widget(outer, area);
            let [edit, heard, _, buttons] = Layout::vertical([
                Constraint::Percentage(60),
                Constraint::Min(3),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .areas(inner);
            editor.render(
                frame,
                edit,
                focus_block("What you meant (saved in owtui)", true),
                true,
            );
            frame.render_widget(
                Paragraph::new(item.heard().to_string())
                    .style(Style::new().dark_gray())
                    .wrap(Wrap { trim: false })
                    .block(focus_block("Heard (speech engine, before cleanup)", false)),
                heard,
            );
            let mut specs: Vec<ButtonSpec> = vec![
                ("Save  ^S", ctrl('s'), Look::Primary),
                ("Fix word → dictionary  ^D", ctrl('d'), Look::Plain),
                ("Play  ^P", ctrl('p'), Look::Plain),
            ];
            if item.correction.is_some() {
                specs.push(("Revert to app's text  ^R", ctrl('r'), Look::Plain));
            }
            specs.push(("Cancel  Esc", key(KeyCode::Esc), Look::Plain));
            if let Some(q) = quick {
                let area = centered(screen, 64, 10);
                frame.render_widget(Clear, area);
                let outer = Block::bordered()
                    .border_style(Style::new().fg(Color::Yellow))
                    .title(" Fix word everywhere + add to dictionary ");
                let inner = outer.inner(area);
                frame.render_widget(outer, area);
                let [heard_area, fixed_area, _, quick_buttons] = Layout::vertical([
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Length(1),
                    Constraint::Length(1),
                ])
                .areas(inner);
                q.heard.render(
                    frame,
                    heard_area,
                    focus_block("Heard as", !q.on_fixed),
                    !q.on_fixed,
                );
                q.fixed.render(
                    frame,
                    fixed_area,
                    focus_block("Should be", q.on_fixed),
                    q.on_fixed,
                );
                // Only the box on top takes clicks.
                draw_buttons(frame, buttons, &specs, &mut Vec::new());
                draw_buttons(
                    frame,
                    quick_buttons,
                    &[
                        ("Replace + add  Enter", key(KeyCode::Enter), Look::Primary),
                        ("Back  Esc", key(KeyCode::Esc), Look::Plain),
                    ],
                    &mut hits,
                );
            } else {
                draw_buttons(frame, buttons, &specs, &mut hits);
            }
        }
        Modal::Word { original, input } => {
            let area = centered(screen, 70, 5);
            frame.render_widget(Clear, area);
            let [field, _, buttons] = Layout::vertical([
                Constraint::Length(3),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .areas(area);
            let title = if original.is_some() {
                "Edit word"
            } else {
                "Add words, comma-separated"
            };
            input.render(frame, field, focus_block(title, true), true);
            draw_buttons(
                frame,
                buttons,
                &[
                    ("Save  Enter", key(KeyCode::Enter), Look::Primary),
                    ("Cancel  Esc", key(KeyCode::Esc), Look::Plain),
                ],
                &mut hits,
            );
        }
        Modal::Snippet {
            original,
            trigger,
            text,
            on_text,
        } => {
            let area = centered(screen, screen.width * 8 / 10, screen.height * 6 / 10);
            frame.render_widget(Clear, area);
            let [top, bottom, _, buttons] = Layout::vertical([
                Constraint::Length(3),
                Constraint::Min(3),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .areas(area);
            let title = if original.is_some() {
                "Trigger"
            } else {
                "New snippet · trigger"
            };
            trigger.render(frame, top, focus_block(title, !*on_text), !*on_text);
            text.render(
                frame,
                bottom,
                focus_block("Replacement text", *on_text),
                *on_text,
            );
            draw_buttons(
                frame,
                buttons,
                &[
                    ("Save  ^S", ctrl('s'), Look::Primary),
                    ("Cancel  Esc", key(KeyCode::Esc), Look::Plain),
                ],
                &mut hits,
            );
        }
        Modal::Confirm { question, .. } => {
            let area = centered(screen, 76, 8);
            frame.render_widget(Clear, area);
            let outer = Block::bordered()
                .border_style(Style::new().fg(Color::Red))
                .title(" Confirm ");
            let inner = outer.inner(area);
            frame.render_widget(outer, area);
            let [text, buttons] =
                Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
            frame.render_widget(
                Paragraph::new(question.clone()).wrap(Wrap { trim: false }),
                text,
            );
            draw_buttons(
                frame,
                buttons,
                &[
                    ("Yes  y", key(KeyCode::Char('y')), Look::Danger),
                    ("No  n", key(KeyCode::Char('n')), Look::Plain),
                ],
                &mut hits,
            );
        }
    }
    app.buttons = hits;
}
