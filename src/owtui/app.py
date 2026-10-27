"""owtui: browse and edit OpenWhispr history, dictionary and snippets."""

from __future__ import annotations

import argparse
import json
import sys
from datetime import datetime
from pathlib import Path

from textual import on
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, Vertical, VerticalScroll
from textual.markup import escape
from textual.screen import ModalScreen
from textual.widgets import (
    Button, DataTable, Footer, Header, Input, Label, Static, TabbedContent, TabPane, TextArea,
)

from owtui.store import BridgeError, Store, Transcription


def short_time(iso: str) -> str:
    try:
        return datetime.fromisoformat(iso.replace("Z", "+00:00")).astimezone().strftime("%Y-%m-%d %H:%M")
    except ValueError:
        return iso[:16]


def one_line(text: str | None, width: int = 80) -> str:
    flat = " ".join((text or "").split())
    return flat if len(flat) <= width else flat[: width - 1] + "…"


# ------------------------------------------------------------------ modals


class Dialog(ModalScreen):
    BINDINGS = [Binding("ctrl+s", "save", "Save"), Binding("escape", "cancel", "Cancel")]

    def buttons(self) -> ComposeResult:
        with Horizontal(classes="buttons"):
            yield Button("Save  ^S", variant="primary", id="save")
            yield Button("Cancel", id="cancel")

    def action_cancel(self) -> None:
        self.dismiss(None)

    @on(Button.Pressed, "#save")
    def _save(self) -> None:
        self.action_save()

    @on(Button.Pressed, "#cancel")
    def _cancel(self) -> None:
        self.dismiss(None)


class CorrectionEditor(Dialog):
    """Returns the corrected text, "" to drop the correction, or None."""

    def __init__(self, t: Transcription):
        super().__init__()
        self.t = t

    def compose(self) -> ComposeResult:
        with Vertical(classes="dialog wide"):
            yield Label(f"Correct transcript · {short_time(self.t.timestamp)} · {self.t.model or ''}")
            yield Label("What you meant (saved in owtui, used for training)", classes="hint")
            yield TextArea(self.t.final, id="text", soft_wrap=True)
            yield Label("Heard (speech engine, before cleanup)", classes="hint")
            yield TextArea(self.t.heard, id="raw", read_only=True, soft_wrap=True)
            with Horizontal(classes="buttons"):
                yield Button("Save  ^S", variant="primary", id="save")
                if self.t.correction is not None:
                    yield Button("Revert to app's text", id="revert")
                yield Button("Cancel", id="cancel")

    def action_save(self) -> None:
        self.dismiss(self.query_one("#text", TextArea).text)

    @on(Button.Pressed, "#revert")
    def _revert(self) -> None:
        self.dismiss("")


class WordEditor(Dialog):
    def __init__(self, word: str = ""):
        super().__init__()
        self.word = word

    def compose(self) -> ComposeResult:
        with Vertical(classes="dialog"):
            yield Label("Edit word" if self.word else "Add words")
            yield Input(self.word, id="word",
                        placeholder="Word or phrase" if self.word else "Words, comma-separated")
            yield from self.buttons()

    def action_save(self) -> None:
        value = self.query_one("#word", Input).value
        words = [" ".join(value.split())] if self.word else [w.strip() for w in value.split(",")]
        self.dismiss([w for w in words if w] or None)


class SnippetEditor(Dialog):
    def __init__(self, snippet: dict | None = None):
        super().__init__()
        self.snippet = snippet or {}

    def compose(self) -> ComposeResult:
        with Vertical(classes="dialog wide"):
            yield Label("Edit snippet" if self.snippet else "New snippet")
            yield Input(self.snippet.get("trigger", ""), id="trigger",
                        placeholder="Trigger phrase, e.g. my calendar link")
            yield TextArea(self.snippet.get("replacement", ""), id="text", soft_wrap=True)
            yield from self.buttons()

    def action_save(self) -> None:
        trigger = " ".join(self.query_one("#trigger", Input).value.split())
        text = self.query_one("#text", TextArea).text.strip()
        if not trigger or not text:
            self.notify("A trigger and text are required.", severity="error")
            return
        self.dismiss({"trigger": trigger, "replacement": text})


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
    TITLE = "OpenWhispr"
    CSS = """
    ModalScreen { align: center middle; }
    .dialog { width: 70; height: auto; max-height: 90%; padding: 1 2;
              border: thick $accent; background: $surface; }
    .dialog.wide { width: 110; }
    .dialog TextArea { height: 10; }
    .dialog #raw { height: 6; }
    .buttons { height: auto; margin-top: 1; }
    .buttons Button { margin-right: 2; }
    .hint { color: $text-muted; margin-top: 1; }
    #history-search { margin-bottom: 1; }
    #history-body { height: 1fr; }
    #history-table { width: 3fr; }
    #history-detail { width: 2fr; border-left: solid $panel; padding: 0 1; }
    """
    BINDINGS = [
        Binding("q", "quit", "Quit"),
        Binding("r", "reload", "Reload"),
        Binding("x", "export", "Export pairs"),
        Binding("e", "edit", "Edit"),
        Binding("a", "add", "Add"),
        Binding("d,delete", "delete", "Delete"),
        Binding("slash", "search", "Search", show=False),
    ]

    def __init__(self, store: Store):
        super().__init__()
        self.store = store
        self._items: dict[str, Transcription] = {}
        self._snippets: dict[str, dict] = {}

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
            with TabPane("Stats", id="tab-stats"):
                yield Static(id="stats")
        yield Footer()

    def on_mount(self) -> None:
        self.query_one("#history-table", DataTable).add_columns("When", "Model", "Sec", "Text")
        self.query_one("#dict-table", DataTable).add_columns("Word")
        self.query_one("#snip-table", DataTable).add_columns("Trigger", "Replacement")
        self.action_reload()

    # ---- loading

    def action_reload(self) -> None:
        self.load_history()
        self.load_list("#dict-table")
        self.load_list("#snip-table")
        self.load_stats()

    def load_history(self) -> None:
        table = self.query_one("#history-table", DataTable)
        table.clear()
        self._items.clear()
        search = self.query_one("#history-search", Input).value.strip()
        for t in self.store.transcriptions(search):
            self._items[str(t.id)] = t
            mark = "✎ " if t.correction is not None else ""
            secs = f"{t.audio_duration_ms / 1000:.0f}" if t.audio_duration_ms else ""
            table.add_row(short_time(t.timestamp), t.model or t.provider or "", secs,
                          mark + one_line(t.final), key=str(t.id))
        self.sub_title = "live" if self.store.live else "read-only (OpenWhispr not running)"
        self.show_preview()

    def load_list(self, table_id: str) -> None:
        table = self.query_one(table_id, DataTable)
        table.clear()
        try:
            if table_id == "#dict-table":
                for word in self.store.dictionary():
                    table.add_row(word, key=word)
            else:
                self._snippets.clear()
                for s in self.store.snippets():
                    self._snippets[s["trigger"]] = s
                    table.add_row(s["trigger"], one_line(s.get("replacement"), 90), key=s["trigger"])
        except BridgeError as exc:
            table.add_row(f"({exc})", *([""] * (len(table.columns) - 1)))

    def load_stats(self) -> None:
        items = self.store.transcriptions()
        audio = sum(t.audio_duration_ms or 0 for t in items) / 60000
        words = sum(len(t.final.split()) for t in items)
        models: dict[str, int] = {}
        for t in items:
            models[t.model or t.provider or "?"] = models.get(t.model or t.provider or "?", 0) + 1
        lines = [
            f"Transcripts:        {len(items)}",
            f"Words:              {words}",
            f"Audio:              {audio:.1f} min",
            f"First:              {short_time(items[-1].timestamp) if items else '-'}",
            f"Latest:             {short_time(items[0].timestamp) if items else '-'}",
            f"Cleaned by AI:      {sum(1 for t in items if t.raw_text and t.raw_text.strip() != t.text.strip())}",
            f"Your corrections:   {sum(1 for t in items if t.correction is not None)}",
            f"Training pairs:     {sum(1 for t in items if t.changed and t.status == 'completed')}",
            "", "Models:",
        ] + [f"  {m:<28} {n}" for m, n in sorted(models.items(), key=lambda kv: -kv[1])]
        lines += ["", f"Corrections file: {self.store.corrections.path}"]
        self.query_one("#stats", Static).update(escape("\n".join(lines)))

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

    def _current_item(self) -> Transcription | None:
        key = self._current_key("#history-table")
        return self._items.get(key) if key else None

    def show_preview(self) -> None:
        preview = self.query_one("#history-preview", Static)
        t = self._current_item()
        if not t:
            preview.update("")
            return
        meta = [short_time(t.timestamp), t.model or t.provider or ""]
        if t.audio_duration_ms:
            meta.append(f"{t.audio_duration_ms / 1000:.1f}s")
        body = ["[dim]" + escape(" · ".join(meta)) + "[/dim]", ""]
        if t.correction is not None:
            body += ["[b]Your correction[/b]", escape(t.correction), "",
                     "[dim]── app saved ──[/dim]", f"[dim]{escape(t.text)}[/dim]"]
        else:
            body.append(escape(t.text))
        if t.raw_text and t.raw_text.strip() != t.text.strip():
            body += ["", "[dim]── heard (before cleanup) ──[/dim]", f"[dim]{escape(t.raw_text)}[/dim]"]
        preview.update("\n".join(body))

    def action_search(self) -> None:
        self.query_one("#tabs", TabbedContent).active = "tab-history"
        self.query_one("#history-search", Input).focus()

    # ---- context-sensitive actions

    @property
    def tab(self) -> str:
        return self.query_one("#tabs", TabbedContent).active

    def check_action(self, action: str, parameters: tuple) -> bool | None:
        # Letter keys belong to the focused text field.
        if isinstance(self.focused, (Input, TextArea)) and action in {
            "edit", "add", "delete", "reload", "export", "quit", "search",
        }:
            return False
        allowed = {
            "edit": {"tab-history", "tab-dictionary", "tab-snippets"},
            "add": {"tab-dictionary", "tab-snippets"},
            "delete": {"tab-history", "tab-dictionary", "tab-snippets"},
        }
        return self.tab in allowed[action] if action in allowed else True

    @on(TabbedContent.TabActivated)
    def _tab_changed(self) -> None:
        self.refresh_bindings()

    def on_descendant_focus(self) -> None:
        self.refresh_bindings()

    def _bridge(self, change, reload) -> None:
        """Run a change that needs the running app, then refresh."""
        try:
            change()
        except BridgeError as exc:
            self.notify(str(exc), severity="error")
            return
        reload()

    @on(DataTable.RowSelected)
    def _row_selected(self) -> None:
        self.action_edit()

    def action_edit(self) -> None:
        if self.tab == "tab-history":
            if t := self._current_item():
                def done(text: str | None) -> None:
                    if text is not None:
                        self.store.correct(t, text or None)
                        self.notify("Correction saved." if text else "Correction removed.")
                        self.load_history()
                self.push_screen(CorrectionEditor(t), done)
        elif self.tab == "tab-dictionary":
            if (word := self._current_key("#dict-table")) and not word.startswith("("):
                def done_word(words: list[str] | None) -> None:
                    if words and words[0] != word:
                        self._bridge(lambda: self.store.update_dictionary(add=words, remove=[word]),
                                     lambda: self.load_list("#dict-table"))
                self.push_screen(WordEditor(word), done_word)
        elif self.tab == "tab-snippets":
            if (key := self._current_key("#snip-table")) and key in self._snippets:
                def done_snip(s: dict | None) -> None:
                    if s:
                        remove = [key] if s["trigger"].lower() != key.lower() else []
                        self._bridge(lambda: self.store.update_snippets(add=[s], remove=remove),
                                     lambda: self.load_list("#snip-table"))
                self.push_screen(SnippetEditor(self._snippets[key]), done_snip)

    def action_add(self) -> None:
        if self.tab == "tab-dictionary":
            def done(words: list[str] | None) -> None:
                if words:
                    self._bridge(lambda: self.store.update_dictionary(add=words),
                                 lambda: self.load_list("#dict-table"))
            self.push_screen(WordEditor(), done)
        elif self.tab == "tab-snippets":
            def done_snip(s: dict | None) -> None:
                if s:
                    self._bridge(lambda: self.store.update_snippets(add=[s]),
                                 lambda: self.load_list("#snip-table"))
            self.push_screen(SnippetEditor(), done_snip)

    def action_delete(self) -> None:
        if self.tab == "tab-history":
            t = self._current_item()
            if not t:
                return
            question = f"Delete this transcript from OpenWhispr?\n\n{one_line(t.final, 60)}"
            change, reload = (lambda: self.store.delete_transcription(t)), self.load_history
        elif self.tab == "tab-dictionary":
            word = self._current_key("#dict-table")
            if not word or word.startswith("("):
                return
            question = f"Remove “{word}” from the dictionary?"
            change, reload = (lambda: self.store.update_dictionary(remove=[word])), (lambda: self.load_list("#dict-table"))
        elif self.tab == "tab-snippets":
            key = self._current_key("#snip-table")
            if not key or key not in self._snippets:
                return
            question = f"Delete snippet “{key}”?"
            change, reload = (lambda: self.store.update_snippets(remove=[key])), (lambda: self.load_list("#snip-table"))
        else:
            return

        def done(yes: bool) -> None:
            if yes:
                self._bridge(change, reload)
        self.push_screen(Confirm(question), done)

    # ---- export

    def action_export(self) -> None:
        out = Path.cwd() / f"openwhispr-pairs-{datetime.now():%Y%m%d-%H%M%S}.jsonl"
        self.notify(f"Wrote {export_pairs(self.store, out)} pairs to {out}", timeout=8)


def export_pairs(store: Store, out: Path) -> int:
    count = 0
    with open(out, "w", encoding="utf-8") as fh:
        for pair in store.training_pairs():
            fh.write(json.dumps(pair, ensure_ascii=False) + "\n")
            count += 1
    return count


def main() -> None:
    parser = argparse.ArgumentParser(prog="owtui", description=__doc__)
    parser.add_argument("--export", metavar="FILE", help="write heard→final training pairs as JSONL and exit")
    args = parser.parse_args()
    store = Store()
    if args.export:
        print(f"{export_pairs(store, Path(args.export))} pairs written to {args.export}")
        return
    if not store.db_path.is_file() and not store.bridge.bridge_file.is_file():
        sys.exit(f"No OpenWhispr data found at {store.db_path}")
    OWTui(store).run()


if __name__ == "__main__":
    main()
