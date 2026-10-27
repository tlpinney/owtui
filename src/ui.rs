//! Drawing.

use crate::app::{one_line, short_time, App, Modal, Tab};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Tabs, Wrap};
use ratatui::Frame;
use std::collections::BTreeMap;

const ACCENT: Color = Color::Cyan;

fn selected() -> Style {
    Style::new()
        .bg(Color::DarkGray)
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
    let tabs = Tabs::new(
        Tab::ALL
            .iter()
            .enumerate()
            .map(|(i, t)| format!("{} {}", i + 1, t.title())),
    )
    .select(app.tab.index())
    .highlight_style(Style::new().fg(ACCENT).add_modifier(Modifier::BOLD));
    // Tabs and the app label share the first row; a block title there would
    // take a row of its own and push the tabs behind the border.
    let label = Line::from(vec![" OpenWhispr ".bold(), mode]).right_aligned();
    let [row, rule] = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(top);
    let [tabs_area, label_area] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(label.width() as u16)])
            .areas(row);
    frame.render_widget(tabs, tabs_area);
    frame.render_widget(label, label_area);
    frame.render_widget(Block::new().borders(Borders::TOP), rule);

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
    let preview_lines = app
        .selected_item()
        .map(|t| preview_text(&t))
        .unwrap_or_default();

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
    .block(Block::bordered().title(format!(" {count} transcripts ")));
    frame.render_stateful_widget(table, list, &mut app.history);

    frame.render_widget(
        Paragraph::new(preview_lines)
            .wrap(Wrap { trim: false })
            .block(Block::bordered().title(" Preview ")),
        preview,
    );
}

fn preview_text(t: &crate::store::Transcription) -> Vec<Line<'static>> {
    let dim = Style::new().dark_gray();
    let mut meta = vec![short_time(t.when()), t.engine().to_string()];
    if let Some(ms) = t.audio_duration_ms {
        meta.push(format!("{:.1}s", ms as f64 / 1000.0));
    }
    let mut lines = vec![Line::styled(meta.join(" · "), dim), Line::raw("")];
    if let Some(fix) = &t.correction {
        lines.push(Line::styled(
            "Your correction",
            Style::new().fg(Color::Yellow).bold(),
        ));
        lines.extend(fix.lines().map(|l| Line::raw(l.to_string())));
        lines.push(Line::raw(""));
        lines.push(Line::styled("── app saved ──", dim));
        lines.extend(t.text.lines().map(|l| Line::styled(l.to_string(), dim)));
    } else {
        lines.extend(t.text.lines().map(|l| Line::raw(l.to_string())));
    }
    if t.cleaned_by_ai() {
        lines.push(Line::raw(""));
        lines.push(Line::styled("── heard (before cleanup) ──", dim));
        lines.extend(t.heard().lines().map(|l| Line::styled(l.to_string(), dim)));
    }
    lines
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
    .block(Block::bordered().title(format!(" {} mistakes ", ins.patterns.len())));
    frame.render_stateful_widget(table, list, &mut app.patterns);

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
        .block(Block::bordered().title(title));
    frame.render_stateful_widget(table, area, &mut app.dict);
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
    .block(Block::bordered().title(title));
    frame.render_stateful_widget(table, area, &mut app.snips);
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
        (Some(Modal::Correct { item, .. }), _) if item.correction.is_some() => &[
            ("^S", "save"),
            ("^D", "fix word → dictionary"),
            ("^P", "play"),
            ("^R", "revert to app's text"),
            ("Esc", "cancel"),
        ],
        (Some(Modal::Correct { .. }), _) => &[
            ("^S", "save"),
            ("^D", "fix word → dictionary"),
            ("^P", "play"),
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

fn draw_modal(frame: &mut Frame, app: &mut App) {
    let screen = frame.area();
    let Some(modal) = app.modal.as_mut() else {
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
            let [edit, heard] =
                Layout::vertical([Constraint::Percentage(60), Constraint::Percentage(40)])
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
            if let Some(q) = quick {
                let area = centered(screen, 64, 8);
                frame.render_widget(Clear, area);
                let outer = Block::bordered()
                    .border_style(Style::new().fg(Color::Yellow))
                    .title(" Fix word everywhere + add to dictionary ");
                let inner = outer.inner(area);
                frame.render_widget(outer, area);
                let [heard_area, fixed_area] =
                    Layout::vertical([Constraint::Length(3), Constraint::Length(3)]).areas(inner);
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
            }
        }
        Modal::Word { original, input } => {
            let area = centered(screen, 70, 3);
            frame.render_widget(Clear, area);
            let title = if original.is_some() {
                "Edit word"
            } else {
                "Add words, comma-separated"
            };
            input.render(frame, area, focus_block(title, true), true);
        }
        Modal::Snippet {
            original,
            trigger,
            text,
            on_text,
        } => {
            let area = centered(screen, screen.width * 8 / 10, screen.height * 6 / 10);
            frame.render_widget(Clear, area);
            let [top, bottom] =
                Layout::vertical([Constraint::Length(3), Constraint::Min(3)]).areas(area);
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
        }
        Modal::Confirm { question, .. } => {
            let area = centered(screen, 76, 7);
            frame.render_widget(Clear, area);
            frame.render_widget(
                Paragraph::new(question.clone())
                    .wrap(Wrap { trim: false })
                    .block(
                        Block::bordered()
                            .border_style(Style::new().fg(Color::Red))
                            .title(" Confirm · y / n "),
                    ),
                area,
            );
        }
    }
}
