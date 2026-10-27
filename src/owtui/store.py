"""Read and edit OpenWhisper's data files without importing the app.

History lives in ``openwhisper.db`` (table ``transcription_history``);
the dictionary, snippets, cleanup prompt and learned rules live in
``openwhisper_settings.json``. The app re-reads the settings file when it
changes on disk and mutates it from a fresh read, so writing it atomically
here is safe while the app runs.
"""

from __future__ import annotations

import json
import os
import shutil
import sqlite3
import sys
import tempfile
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Iterator

APP_NAME = "OpenWhisper"
SETTINGS_FILENAME = "openwhisper_settings.json"
DB_FILENAME = "openwhisper.db"

# Settings keys, mirrored from OpenWhisper's services/settings.py.
KEY_DICTIONARY = "dictation_dictionary"
KEY_SNIPPETS = "dictation_snippets"
KEY_PROMPT = "transcript_cleanup_prompt"
KEY_RULES = "transcript_cleanup_rules"


def platform_data_dir() -> Path:
    """OpenWhisper's per-user directory, as config.local_app_dir() picks it."""
    if sys.platform == "win32":
        base = os.environ.get("LOCALAPPDATA") or os.path.expanduser("~")
    elif sys.platform == "darwin":
        base = os.path.expanduser("~/Library/Application Support")
    else:
        base = os.environ.get("XDG_DATA_HOME", os.path.expanduser("~/.local/share"))
    return Path(base) / APP_NAME


def candidate_dirs(explicit: str | None = None) -> list[Path]:
    found: list[Path] = []
    if explicit:
        found.append(Path(explicit).expanduser())
    if env := os.environ.get("OPENWHISPER_DATA_DIR"):
        found.append(Path(env).expanduser())
    found.append(platform_data_dir())
    # A source checkout keeps its own data next to the code.
    found.append(Path("~/code/OpenWhisper").expanduser())
    return found


def find_data_dir(explicit: str | None = None) -> Path | None:
    """The first candidate holding a settings file or database."""
    for folder in candidate_dirs(explicit):
        if (folder / SETTINGS_FILENAME).is_file() or (folder / DB_FILENAME).is_file():
            return folder
    return None


# ---------------------------------------------------------------- history


@dataclass
class Entry:
    id: str
    timestamp: str
    text: str
    raw_text: str | None
    cleaned_text: str | None
    title: str | None
    model: str
    app_name: str | None
    entry_kind: str | None
    cleanup_level: str | None
    audio_duration: float | None
    audio_file: str | None

    @property
    def edited(self) -> bool:
        """The saved text differs from what the engine heard."""
        return bool(self.raw_text) and self.raw_text.strip() != self.text.strip()


_ENTRY_COLUMNS = (
    "id, timestamp, text, raw_text, cleaned_text, title, model, app_name, "
    "entry_kind, cleanup_level, audio_duration, audio_file"
)


class History:
    def __init__(self, db_path: Path):
        self.path = db_path

    def _connect(self) -> sqlite3.Connection:
        conn = sqlite3.connect(self.path, timeout=5)
        conn.row_factory = sqlite3.Row
        return conn

    def _columns(self, conn: sqlite3.Connection) -> set[str]:
        return {row[1] for row in conn.execute("PRAGMA table_info(transcription_history)")}

    def entries(self, search: str = "", limit: int = 1000) -> list[Entry]:
        with self._connect() as conn:
            have = self._columns(conn)
            # Older schemas lack the v17 context columns; read them as NULL.
            cols = ", ".join(
                c if c in have else f"NULL AS {c}" for c in _ENTRY_COLUMNS.split(", ")
            )
            sql = f"SELECT {cols} FROM transcription_history"
            args: list[Any] = []
            if search:
                like = f"%{search}%"
                sql += " WHERE text LIKE ? OR raw_text LIKE ? OR title LIKE ?"
                args += [like, like, like]
            sql += " ORDER BY timestamp DESC LIMIT ?"
            args.append(limit)
            return [Entry(**dict(row)) for row in conn.execute(sql, args)]

    def update(self, entry_id: str, *, text: str, title: str | None) -> None:
        with self._connect() as conn:
            conn.execute(
                "UPDATE transcription_history SET text = ?, title = ? WHERE id = ?",
                (text, title or None, entry_id),
            )

    def delete(self, entry_id: str) -> None:
        # Leaves any recording on disk; the app prunes recordings by its own rules.
        with self._connect() as conn:
            conn.execute("DELETE FROM transcription_history WHERE id = ?", (entry_id,))

    def stats(self) -> dict[str, Any]:
        with self._connect() as conn:
            row = conn.execute(
                "SELECT COUNT(*), MIN(timestamp), MAX(timestamp), "
                "COALESCE(SUM(audio_duration), 0) FROM transcription_history"
            ).fetchone()
            by_app = conn.execute(
                "SELECT COALESCE(app_name, '(unknown)'), COUNT(*) FROM transcription_history "
                "GROUP BY 1 ORDER BY 2 DESC LIMIT 10"
            ).fetchall() if "app_name" in self._columns(conn) else []
        return {
            "count": row[0], "first": row[1], "last": row[2],
            "audio_seconds": row[3], "by_app": by_app,
        }

    def training_pairs(self) -> Iterator[dict[str, Any]]:
        """Entries whose saved text differs from what the engine heard."""
        for entry in self.entries(limit=1_000_000):
            if entry.edited:
                yield {
                    "id": entry.id,
                    "timestamp": entry.timestamp,
                    "app": entry.app_name,
                    "input": entry.raw_text,
                    "output": entry.text,
                }


# --------------------------------------------------------------- settings


class Settings:
    """Read-modify-write access to openwhisper_settings.json."""

    def __init__(self, path: Path):
        self.path = path
        self._backed_up = False

    def load(self) -> dict[str, Any]:
        if not self.path.is_file():
            return {}
        with open(self.path, encoding="utf-8") as fh:
            data = json.load(fh)
        return data if isinstance(data, dict) else {}

    def mutate(self, change: Callable[[dict[str, Any]], None]) -> dict[str, Any]:
        """Apply ``change`` to a fresh read and replace the file atomically."""
        settings = self.load()
        change(settings)
        if self.path.is_file() and not self._backed_up:
            # One backup per session, taken before the first change.
            shutil.copy2(self.path, self.path.with_name(self.path.name + ".owtui.bak"))
            self._backed_up = True
        fd, tmp = tempfile.mkstemp(dir=self.path.parent, prefix=".owtui-", suffix=".json")
        try:
            with os.fdopen(fd, "w", encoding="utf-8") as fh:
                json.dump(settings, fh, indent=2, ensure_ascii=False)
            os.replace(tmp, self.path)
        except BaseException:
            if os.path.exists(tmp):
                os.unlink(tmp)
            raise
        return settings

    # Dictionary: newest first, {id, term, starred, heard, learned, new[, protected]}.

    def dictionary(self) -> list[dict[str, Any]]:
        raw = self.load().get(KEY_DICTIONARY)
        return [t for t in raw if isinstance(t, dict) and t.get("term")] if isinstance(raw, list) else []

    def save_term(self, term: dict[str, Any]) -> None:
        def change(settings: dict[str, Any]) -> None:
            terms = [t for t in settings.get(KEY_DICTIONARY) or [] if isinstance(t, dict)]
            for i, existing in enumerate(terms):
                if existing.get("id") == term.get("id"):
                    terms[i] = {**existing, **term}
                    break
            else:
                terms.insert(0, {
                    "id": uuid.uuid4().hex[:12], "starred": False, "heard": [],
                    "learned": False, "new": False, **term,
                })
            settings[KEY_DICTIONARY] = terms
        self.mutate(change)

    def delete_term(self, term_id: str) -> None:
        self.mutate(lambda s: s.__setitem__(
            KEY_DICTIONARY,
            [t for t in s.get(KEY_DICTIONARY) or [] if not (isinstance(t, dict) and t.get("id") == term_id)],
        ))

    # Snippets: saved order, {id, trigger, text, formatted}.

    def snippets(self) -> list[dict[str, Any]]:
        raw = self.load().get(KEY_SNIPPETS)
        return [s for s in raw if isinstance(s, dict)] if isinstance(raw, list) else []

    def save_snippet(self, snippet: dict[str, Any]) -> None:
        def change(settings: dict[str, Any]) -> None:
            snippets = [s for s in settings.get(KEY_SNIPPETS) or [] if isinstance(s, dict)]
            for i, existing in enumerate(snippets):
                if existing.get("id") == snippet.get("id"):
                    snippets[i] = {**existing, **snippet}
                    break
            else:
                snippets.append({"id": uuid.uuid4().hex[:12], "formatted": False, **snippet})
            settings[KEY_SNIPPETS] = snippets
        self.mutate(change)

    def delete_snippet(self, snippet_id: str) -> None:
        self.mutate(lambda s: s.__setitem__(
            KEY_SNIPPETS,
            [x for x in s.get(KEY_SNIPPETS) or [] if not (isinstance(x, dict) and x.get("id") == snippet_id)],
        ))

    # Cleanup prompt ("" means the built-in preset for the chosen level) and rules.

    def prompt(self) -> str:
        value = self.load().get(KEY_PROMPT)
        return value if isinstance(value, str) else ""

    def rules(self) -> list[str]:
        raw = self.load().get(KEY_RULES)
        return [r for r in raw if isinstance(r, str) and r.strip()] if isinstance(raw, list) else []

    def save_prompt_and_rules(self, prompt: str, rules: list[str]) -> None:
        def change(settings: dict[str, Any]) -> None:
            settings[KEY_PROMPT] = prompt.strip()
            settings[KEY_RULES] = [r.strip() for r in rules if r.strip()]
        self.mutate(change)
