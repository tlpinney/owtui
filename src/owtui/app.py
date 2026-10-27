"""owtui: browse and edit OpenWhisper history, dictionary, snippets and prompt."""

from __future__ import annotations

import argparse
import json
import sys
from datetime import datetime
from pathlib import Path
from typing import Any

from textual import on
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, Vertical, VerticalScroll
from textual.screen import ModalScreen
from textual.widgets import (
    Button, Checkbox, DataTable, Footer, Header, Input, Label, Static,
    TabbedContent, TabPane, TextArea,
)

from owtui.store import DB_FILENAME, SETTINGS_FILENAME, Entry, History, Settings, candidate_dirs, find_data_dir


def short_time(iso: str) -> str:
    try:
        return datetime.fromisoformat(iso).astimezone().strftime("%Y-%m-%d %H:%M")
    except ValueError:
        return iso[:16]


def one_line(text: str | None, width: int = 80) -> str:
    flat = " ".join((text or "").split())
    return flat if len(flat) <= width else flat[: width - 1] + "…"


# ------------------------------------------------------------------ modals


class EntryEditor(ModalScreen[dict | None]):
    BINDINGS = [Binding("ctrl+s", "save", "Save"), Binding("escape", "cancel", "Cancel")]

    def __init__(self, entry: Entry):
        super().__init__()
        self.entry = entry

    def compose(self) -> ComposeResult:
        with Vertical(classes="dialog wide"):
            yield Label(f"Edit entry · {short_time(self.entry.timestamp)} · {self.entry.model}")
            yield Input(self.entry.title or "", placeholder="Title (optional)", id="title")
            yield Label("Text (what OpenWhisper saved)")
            yield TextArea(self.entry.text, id="text", soft_wrap=True)
            if self.entry.raw_text:
                yield Label("Heard (raw engine output, read-only)")
                yield TextArea(self.entry.raw_text, id="raw", read_only=True, soft_wrap=True)
            with Horizontal(classes="buttons"):
                yield Button("Save  ^S", variant="primary", id="save")
                yield Button("Cancel", id="cancel")

    def action_save(self) -> None:
        self.dismiss({
            "title": self.query_one("#title", Input).value.strip(),
            "text": self.query_one("#text", TextArea).text,
        })

    def action_cancel(self) -> None:
        self.dismiss(None)

    @on(Button.Pressed, "#save")
    def _save(self) -> None:
        self.action_save()

    @on(Button.Pressed, "#cancel")
    def _cancel(self) -> None:
        self.dismiss(None)


class TermEditor(ModalScreen[dict | None]):
    BINDINGS = [Binding("ctrl+s", "save", "Save"), Binding("escape", "cancel", "Cancel")]

    def __init__(self, term: dict[str, Any] | None = None):
        super().__init__()
        self.term = term or {}

    def compose(self) -> ComposeResult:
        with Vertical(classes="dialog"):
            yield Label("Edit word" if self.term else "Add word")
            yield Input(self.term.get("term", ""), placeholder="Word or phrase, spelled your way", id="term")
            yield Input(
                ", ".join(self.term.get("heard") or []),
                placeholder="Sounds like (comma-separated, e.g. cube control)", id="heard",
                disabled=bool(self.term.get("learned")),
            )
            yield Checkbox("Starred (goes first where space is limited)", self.term.get("starred", False), id="starred")
            yield Checkbox("Protected (AI cleanup never changes it)", self.term.get("protected", False), id="protected")
            with Horizontal(classes="buttons"):
                yield Button("Save  ^S", variant="primary", id="save")
                yield Button("Cancel", id="cancel")

    def action_save(self) -> None:
        word = " ".join(self.query_one("#term", Input).value.split())
        if not word:
            self.notify("A word is required.", severity="error")
            return
        heard = [h.strip() for h in self.query_one("#heard", Input).value.split(",") if h.strip()]
        result = {
            "term": word[:64],
            "heard": [] if self.term.get("learned") else heard[:5],
            "starred": self.query_one("#starred", Checkbox).value,
            # Reviewed here, so it is no longer new.
            "new": False,
        }
        if self.query_one("#protected", Checkbox).value:
            result["protected"] = True
        elif "protected" in self.term:
            result["protected"] = False
        if "id" in self.term:
            result["id"] = self.term["id"]
        self.dismiss(result)

    def action_cancel(self) -> None:
        self.dismiss(None)

    @on(Button.Pressed, "#save")
    def _save(self) -> None:
        self.action_save()

    @on(Button.Pressed, "#cancel")
    def _cancel(self) -> None:
        self.dismiss(None)


class SnippetEditor(ModalScreen[dict | None]):
    BINDINGS = [Binding("ctrl+s", "save", "Save"), Binding("escape", "cancel", "Cancel")]

    def __init__(self, snippet: dict[str, Any] | None = None):
        super().__init__()
        self.snippet = snippet or {}

    def compose(self) -> ComposeResult:
        with Vertical(classes="dialog wide"):
            yield Label("Edit snippet" if self.snippet else "New snippet")
            yield Input(self.snippet.get("trigger", ""), placeholder="Trigger phrase, e.g. my calendar link", id="trigger")
            yield TextArea(self.snippet.get("text", ""), id="text", soft_wrap=True)
            yield Checkbox("Keep formatting (light Markdown)", self.snippet.get("formatted", False), id="formatted")
            with Horizontal(classes="buttons"):
                yield Button("Save  ^S", variant="primary", id="save")
                yield Button("Cancel", id="cancel")

    def action_save(self) -> None:
        trigger = " ".join(self.query_one("#trigger", Input).value.split())
        text = self.query_one("#text", TextArea).text.strip()
        if len(trigger) < 2 or not text:
            self.notify("A trigger (2+ characters) and text are required.", severity="error")
            return
        result = {"trigger": trigger[:60], "text": text[:10_000],
                  "formatted": self.query_one("#formatted", Checkbox).value}
        if "id" in self.snippet:
            result["id"] = self.snippet["id"]
        self.dismiss(result)

    def action_cancel(self) -> None:
        self.dismiss(None)

    @on(Button.Pressed, "#save")
    def _save(self) -> None:
        self.action_save()

    @on(Button.Pressed, "#cancel")
    def _cancel(self) -> None:
        self.dismiss(None)


class Confirm(ModalScreen[bool]):
    BINDINGS = [Binding("y", "yes", "Yes"), Binding("n,escape", "no", "No")]

    def __init__(self, question: str):
        super().__init__()
        self.question = question

    def compose(self) -> ComposeResult:
        with Vertical(classes="dialog"):
            yield Label(self.question)
            with Horizontal(classes="buttons"):
                yield Button("Yes (y)", variant="error", id="yes")
                yield Button("No (n)", id="no")

    def action_yes(self) -> None:
        self.dismiss(True)

    def action_no(self) -> None:
        self.dismiss(False)

    @on(Button.Pressed)
    def _pressed(self, event: Button.Pressed) -> None:
        self.dismiss(event.button.id == "yes")


# --------------------------------------------------------------------- app


class OWTui(App):
    TITLE = "OpenWhisper"
    CSS = """
    .dialog { width: 70; height: auto; max-height: 90%; padding: 1 2;
              border: thick $accent; background: $surface; }
    .dialog.wide { width: 110; }
    .dialog TextArea { height: 10; }
    .dialog #raw { height: 6; }
    ModalScreen { align: center middle; }
    .buttons { height: auto; margin-top: 1; }
    .buttons Button { margin-right: 2; }
    #history-search { margin: 0 0 1 0; }
    #history-body { height: 1fr; }
    #history-table { width: 3fr; }
    #history-detail { width: 2fr; border-left: solid $panel; padding: 0 1; }
    #prompt-text { height: 2fr; }
    #rules-text { height: 1fr; }
    .hint { color: $text-muted; margin: 1 0 0 0; }
    """
    BINDINGS = [
        Binding("q", "quit", "Quit"),
        Binding("r", "reload", "Reload"),
        Binding("x", "export", "Export training pairs"),
        Binding("e", "edit", "Edit"),
        Binding("a", "add", "Add"),
        Binding("s", "star", "Star"),
        Binding("delete,d", "delete", "Delete"),
        Binding("slash", "search", "Search", show=False),
    ]

    def __init__(self, data_dir: Path):
        super().__init__()
        self.data_dir = data_dir
        db = data_dir / DB_FILENAME
        self.history = History(db) if db.is_file() else None
        self.settings = Settings(data_dir / SETTINGS_FILENAME)
        self._entries: dict[str, Entry] = {}
        self._terms: dict[str, dict] = {}
        self._snippets: dict[str, dict] = {}
        self.sub_title = str(data_dir)

    def compose(self) -> ComposeResult:
        yield Header()
        with TabbedContent(id="tabs"):
            with TabPane("History", id="tab-history"):
                yield Input(placeholder="Search history ( / )", id="history-search")
                with Horizontal(id="history-body"):
                    yield DataTable(id="history-table", cursor_type="row", zebra_stripes=True)
                    with VerticalScroll(id="history-detail"):
                        yield Static(id="history-preview")
            with TabPane("Dictionary", id="tab-dictionary"):
                yield DataTable(id="dict-table", cursor_type="row", zebra_stripes=True)
            with TabPane("Snippets", id="tab-snippets"):
                yield DataTable(id="snip-table", cursor_type="row", zebra_stripes=True)
            with TabPane("Prompt", id="tab-prompt"):
                yield Label("Custom cleanup prompt (empty = the built-in preset for your cleanup level)")
                yield TextArea(id="prompt-text", soft_wrap=True)
                yield Label("Learned cleanup rules, one per line (max 50)", classes="hint")
                yield TextArea(id="rules-text", soft_wrap=True)
                with Horizontal(classes="buttons"):
                    yield Button("Save prompt & rules", variant="primary", id="save-prompt")
                    yield Button("Revert", id="revert-prompt")
            with TabPane("Stats", id="tab-stats"):
                yield Static(id="stats")
        yield Footer()

    def on_mount(self) -> None:
        self.query_one("#history-table", DataTable).add_columns("When", "App", "Kind", "Text")
        self.query_one("#dict-table", DataTable).add_columns("★", "Word", "Sounds like", "Flags")
        self.query_one("#snip-table", DataTable).add_columns("Trigger", "Text", "Fmt")
        self.action_reload()

    # ---- loading

    def action_reload(self) -> None:
        self.load_history()
        self.load_dictionary()
        self.load_snippets()
        self.load_prompt()
        self.load_stats()

    def load_history(self) -> None:
        table = self.query_one("#history-table", DataTable)
        table.clear()
        self._entries.clear()
        if not self.history:
            self.query_one("#history-preview", Static).update(f"No {DB_FILENAME} in {self.data_dir}")
            return
        search = self.query_one("#history-search", Input).value.strip()
        for entry in self.history.entries(search):
            self._entries[entry.id] = entry
            mark = "✎ " if entry.edited else ""
            table.add_row(
                short_time(entry.timestamp), entry.app_name or "", entry.entry_kind or "",
                mark + one_line(entry.title or entry.text), key=entry.id,
            )
        self.show_preview()

    def load_dictionary(self) -> None:
        table = self.query_one("#dict-table", DataTable)
        table.clear()
        self._terms.clear()
        for term in self.settings.dictionary():
            tid = term.get("id") or term["term"]
            self._terms[tid] = term
            flags = [f for f in ("learned", "new", "protected") if term.get(f)]
            table.add_row(
                "★" if term.get("starred") else "", term["term"],
                ", ".join(term.get("heard") or []), " ".join(flags), key=tid,
            )

    def load_snippets(self) -> None:
        table = self.query_one("#snip-table", DataTable)
        table.clear()
        self._snippets.clear()
        for snip in self.settings.snippets():
            sid = snip.get("id") or snip.get("trigger", "")
            self._snippets[sid] = snip
            table.add_row(snip.get("trigger", ""), one_line(snip.get("text"), 70),
                          "✓" if snip.get("formatted") else "", key=sid)

    def load_prompt(self) -> None:
        self.query_one("#prompt-text", TextArea).text = self.settings.prompt()
        self.query_one("#rules-text", TextArea).text = "\n".join(self.settings.rules())

    def load_stats(self) -> None:
        lines = [f"Data folder: {self.data_dir}", ""]
        if self.history:
            s = self.history.stats()
            lines += [
                f"Transcripts:     {s['count']}",
                f"First:           {short_time(s['first']) if s['first'] else '-'}",
                f"Latest:          {short_time(s['last']) if s['last'] else '-'}",
                f"Audio:           {s['audio_seconds'] / 60:.1f} min",
                f"Edited (pairs):  {sum(1 for _ in self.history.training_pairs())}",
                "", "Top apps:",
            ] + [f"  {name:<24} {n}" for name, n in s["by_app"]]
        lines += ["", f"Dictionary words: {len(self.settings.dictionary())}",
                  f"Snippets:         {len(self.settings.snippets())}",
                  f"Cleanup rules:    {len(self.settings.rules())}"]
        self.query_one("#stats", Static).update("\n".join(lines))

    # ---- history

    @on(Input.Changed, "#history-search")
    def _search_changed(self) -> None:
        self.load_history()

    @on(DataTable.RowHighlighted, "#history-table")
    def _row_changed(self) -> None:
        self.show_preview()

    def _current_key(self, table_id: str) -> str | None:
        table = self.query_one(table_id, DataTable)
        if table.row_count == 0:
            return None
        return table.coordinate_to_cell_key(table.cursor_coordinate).row_key.value

    def show_preview(self) -> None:
        preview = self.query_one("#history-preview", Static)
        key = self._current_key("#history-table")
        entry = self._entries.get(key) if key else None
        if not entry:
            preview.update("")
            return
        meta = [short_time(entry.timestamp), entry.model]
        if entry.cleanup_level:
            meta.append(f"cleanup: {entry.cleanup_level}")
        if entry.audio_duration:
            meta.append(f"{entry.audio_duration:.1f}s")
        body = [f"[b]{entry.title or 'Untitled'}[/b]", "[dim]" + " · ".join(meta) + "[/dim]", "", entry.text]
        if entry.edited:
            body += ["", "[dim]── heard ──[/dim]", f"[dim]{entry.raw_text}[/dim]"]
        preview.update("\n".join(body))

    def action_search(self) -> None:
        self.query_one("#tabs", TabbedContent).active = "tab-history"
        self.query_one("#history-search", Input).focus()

    # ---- context-sensitive actions

    @property
    def tab(self) -> str:
        return self.query_one("#tabs", TabbedContent).active

    def check_action(self, action: str, parameters: tuple) -> bool | None:
        # Single-letter keys type into inputs and text areas, not trigger actions.
        if action in {"edit", "add", "star", "delete", "reload", "export", "quit", "search"}:
            if isinstance(self.focused, (Input, TextArea)):
                return False
        allowed = {
            "edit": {"tab-history", "tab-dictionary", "tab-snippets"},
            "add": {"tab-dictionary", "tab-snippets"},
            "star": {"tab-dictionary"},
            "delete": {"tab-history", "tab-dictionary", "tab-snippets"},
        }
        if action in allowed:
            return self.tab in allowed[action]
        return True

    @on(TabbedContent.TabActivated)
    def _tab_changed(self) -> None:
        self.refresh_bindings()

    def on_descendant_focus(self) -> None:
        self.refresh_bindings()

    def action_edit(self) -> None:
        if self.tab == "tab-history":
            key = self._current_key("#history-table")
            if key and (entry := self._entries.get(key)):
                def done(result: dict | None) -> None:
                    if result and self.history:
                        self.history.update(entry.id, text=result["text"], title=result["title"])
                        self.notify("Entry saved.")
                        self.load_history()
                self.push_screen(EntryEditor(entry), done)
        elif self.tab == "tab-dictionary":
            key = self._current_key("#dict-table")
            if key:
                self._edit_term(self._terms[key])
        elif self.tab == "tab-snippets":
            key = self._current_key("#snip-table")
            if key:
                self._edit_snippet(self._snippets[key])

    @on(DataTable.RowSelected)
    def _row_selected(self) -> None:
        self.action_edit()

    def action_add(self) -> None:
        if self.tab == "tab-dictionary":
            self._edit_term(None)
        elif self.tab == "tab-snippets":
            self._edit_snippet(None)

    def _edit_term(self, term: dict | None) -> None:
        def done(result: dict | None) -> None:
            if result:
                lowered = result["term"].casefold()
                if any(t["term"].casefold() == lowered and t.get("id") != result.get("id")
                       for t in self._terms.values()):
                    self.notify(f"“{result['term']}” is already in the dictionary.", severity="warning")
                    return
                self.settings.save_term(result)
                self.notify("Dictionary saved.")
                self.load_dictionary()
        self.push_screen(TermEditor(term), done)

    def _edit_snippet(self, snippet: dict | None) -> None:
        def done(result: dict | None) -> None:
            if result:
                self.settings.save_snippet(result)
                self.notify("Snippet saved.")
                self.load_snippets()
        self.push_screen(SnippetEditor(snippet), done)

    def action_star(self) -> None:
        key = self._current_key("#dict-table")
        if key and (term := self._terms.get(key)) and term.get("id"):
            self.settings.save_term({"id": term["id"], "starred": not term.get("starred")})
            self.load_dictionary()

    def action_delete(self) -> None:
        if self.tab == "tab-history":
            key = self._current_key("#history-table")
            entry = self._entries.get(key) if key else None
            if not entry or not self.history:
                return
            question = f"Delete this transcript?\n\n{one_line(entry.text, 60)}"
            remove = lambda: self.history.delete(entry.id)
            reload = self.load_history
        elif self.tab == "tab-dictionary":
            key = self._current_key("#dict-table")
            term = self._terms.get(key) if key else None
            if not term or not term.get("id"):
                return
            question = f"Remove “{term['term']}” from the dictionary?"
            remove = lambda: self.settings.delete_term(term["id"])
            reload = self.load_dictionary
        elif self.tab == "tab-snippets":
            key = self._current_key("#snip-table")
            snip = self._snippets.get(key) if key else None
            if not snip or not snip.get("id"):
                return
            question = f"Delete snippet “{snip.get('trigger')}”?"
            remove = lambda: self.settings.delete_snippet(snip["id"])
            reload = self.load_snippets
        else:
            return

        def done(yes: bool) -> None:
            if yes:
                remove()
                reload()
                self.notify("Deleted.")
        self.push_screen(Confirm(question), done)

    # ---- prompt

    @on(Button.Pressed, "#save-prompt")
    def _save_prompt(self) -> None:
        rules = self.query_one("#rules-text", TextArea).text.splitlines()
        if len([r for r in rules if r.strip()]) > 50:
            self.notify("OpenWhisper keeps at most 50 rules; extras would be ignored.", severity="warning")
        self.settings.save_prompt_and_rules(self.query_one("#prompt-text", TextArea).text, rules)
        self.notify("Prompt and rules saved.")
        self.load_prompt()

    @on(Button.Pressed, "#revert-prompt")
    def _revert_prompt(self) -> None:
        self.load_prompt()

    # ---- export

    def action_export(self) -> None:
        if not self.history:
            self.notify("No history database.", severity="error")
            return
        out = Path.cwd() / f"openwhisper-pairs-{datetime.now():%Y%m%d-%H%M%S}.jsonl"
        count = export_pairs(self.history, out)
        self.notify(f"Wrote {count} raw→final pairs to {out}", timeout=8)


def export_pairs(history: History, out: Path) -> int:
    count = 0
    with open(out, "w", encoding="utf-8") as fh:
        for pair in history.training_pairs():
            fh.write(json.dumps(pair, ensure_ascii=False) + "\n")
            count += 1
    return count


def main() -> None:
    parser = argparse.ArgumentParser(prog="owtui", description=__doc__)
    parser.add_argument("--data-dir", help="OpenWhisper data folder (default: auto-detect)")
    parser.add_argument("--export", metavar="FILE", help="write raw→final training pairs as JSONL and exit")
    args = parser.parse_args()

    data_dir = find_data_dir(args.data_dir)
    if not data_dir:
        tried = "\n  ".join(str(p) for p in candidate_dirs(args.data_dir))
        sys.exit(f"No OpenWhisper data found. Looked in:\n  {tried}\n"
                 "Pass --data-dir or set OPENWHISPER_DATA_DIR.")
    if args.export:
        if not (data_dir / DB_FILENAME).is_file():
            sys.exit(f"No {DB_FILENAME} in {data_dir}")
        print(f"{export_pairs(History(data_dir / DB_FILENAME), Path(args.export))} pairs written")
        return
    OWTui(data_dir).run()


if __name__ == "__main__":
    main()
