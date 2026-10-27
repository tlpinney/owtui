"""OpenWhispr data access for owtui.

Live data goes through OpenWhispr's CLI bridge, a local HTTP API the app
starts on 127.0.0.1 and describes in ``~/.openwhispr/cli-bridge.json``. Going
through it keeps the running app's windows and cloud sync up to date; the
app's own code warns that writing its SQLite tables by hand loses rows on the
next launch. When the app isn't running, history is read from
``transcriptions.db`` read-only and edits are refused.

OpenWhispr has no way to change a transcript's text from outside, so
corrections made here are kept in owtui's own file. They overlay the app's
text in the UI and are the "output" side of exported training pairs.
"""

from __future__ import annotations

import json
import os
import sqlite3
import sys
import tempfile
import urllib.error
import urllib.request
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterator, Sequence

BRIDGE_FILE = Path("~/.openwhispr/cli-bridge.json").expanduser()


def app_data_dir() -> Path:
    """Electron's userData folder for OpenWhispr."""
    if sys.platform == "darwin":
        base = Path("~/Library/Application Support").expanduser()
    elif sys.platform == "win32":
        base = Path(os.environ.get("APPDATA") or Path.home())
    else:
        base = Path(os.environ.get("XDG_CONFIG_HOME") or "~/.config").expanduser()
    return base / "open-whispr"


def owtui_dir() -> Path:
    base = os.environ.get("XDG_DATA_HOME") or "~/.local/share"
    return Path(base).expanduser() / "owtui"


class BridgeError(RuntimeError):
    pass


class Bridge:
    def __init__(self, bridge_file: Path = BRIDGE_FILE, timeout: float = 5):
        self.bridge_file = bridge_file
        self.timeout = timeout

    def _endpoint(self) -> tuple[str, str]:
        # Re-read every call: the app writes a new port and token on each launch.
        try:
            info = json.loads(self.bridge_file.read_text())
            return f"http://127.0.0.1:{int(info['port'])}", str(info["token"])
        except (OSError, ValueError, KeyError) as exc:
            raise BridgeError("OpenWhispr isn't running (no CLI bridge file).") from exc

    def request(self, method: str, path: str, body: Any = None) -> Any:
        base, token = self._endpoint()
        data = json.dumps(body).encode() if body is not None else None
        req = urllib.request.Request(
            base + path, data=data, method=method,
            headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json"},
        )
        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as resp:
                raw = resp.read()
        except urllib.error.HTTPError as exc:
            try:
                message = json.loads(exc.read())["error"]["message"]
            except Exception:
                message = f"HTTP {exc.code}"
            raise BridgeError(message) from exc
        except (urllib.error.URLError, OSError) as exc:
            raise BridgeError("Can't reach OpenWhispr. Is it running?") from exc
        return json.loads(raw)["data"] if raw else None


@dataclass
class Transcription:
    id: int
    timestamp: str
    text: str
    raw_text: str | None
    provider: str | None
    model: str | None
    audio_duration_ms: int | None
    has_audio: bool
    status: str
    client_id: str | None
    correction: str | None = None

    @property
    def final(self) -> str:
        """What you meant: your correction, else the app's text."""
        return self.correction if self.correction is not None else self.text

    @property
    def heard(self) -> str:
        """What the speech engine wrote before AI cleanup."""
        return self.raw_text or self.text

    @property
    def changed(self) -> bool:
        return self.heard.strip() != self.final.strip()

    @classmethod
    def from_row(cls, row: dict[str, Any]) -> "Transcription":
        return cls(
            id=int(row["id"]), timestamp=row.get("timestamp") or row.get("created_at") or "",
            text=row.get("text") or "", raw_text=row.get("raw_text"),
            provider=row.get("provider"), model=row.get("model"),
            audio_duration_ms=row.get("audio_duration_ms"),
            has_audio=bool(row.get("has_audio")), status=row.get("status") or "",
            client_id=row.get("client_transcription_id"),
        )


class Corrections:
    """owtui's own edits, keyed by transcription id, in a JSON file."""

    def __init__(self, path: Path | None = None):
        self.path = path or owtui_dir() / "corrections.json"

    def load(self) -> dict[str, dict[str, Any]]:
        try:
            data = json.loads(self.path.read_text(encoding="utf-8"))
            return data if isinstance(data, dict) else {}
        except (OSError, ValueError):
            return {}

    def _write(self, data: dict[str, Any]) -> None:
        self.path.parent.mkdir(parents=True, exist_ok=True)
        fd, tmp = tempfile.mkstemp(dir=self.path.parent, suffix=".json")
        with os.fdopen(fd, "w", encoding="utf-8") as fh:
            json.dump(data, fh, indent=2, ensure_ascii=False)
        os.replace(tmp, self.path)

    def set(self, t: Transcription, text: str | None) -> None:
        data = self.load()
        if text is None or text.strip() == t.text.strip():
            data.pop(str(t.id), None)
        else:
            data[str(t.id)] = {
                "text": text, "app_text": t.text, "raw_text": t.raw_text,
                "client_id": t.client_id, "timestamp": t.timestamp,
                "edited_at": datetime.now(timezone.utc).isoformat(),
            }
        self._write(data)


class Store:
    def __init__(self, bridge: Bridge | None = None, db_path: Path | None = None,
                 corrections: Corrections | None = None):
        self.bridge = bridge or Bridge()
        self.db_path = db_path or app_data_dir() / "transcriptions.db"
        self.corrections = corrections or Corrections()
        self.live = False

    # ---- history

    def _rows_from_db(self, limit: int) -> list[dict[str, Any]]:
        if not self.db_path.is_file():
            return []
        conn = sqlite3.connect(f"file:{self.db_path}?mode=ro", uri=True, timeout=5)
        conn.row_factory = sqlite3.Row
        try:
            return [dict(r) for r in conn.execute(
                "SELECT * FROM transcriptions WHERE deleted_at IS NULL AND status != 'discarded' "
                "ORDER BY timestamp DESC LIMIT ?", (limit,))]
        finally:
            conn.close()

    def transcriptions(self, search: str = "", limit: int = 5000) -> list[Transcription]:
        try:
            rows = self.bridge.request("GET", f"/v1/transcriptions/list?limit={limit}")
            self.live = True
        except BridgeError:
            rows = self._rows_from_db(limit)
            self.live = False
        fixes = self.corrections.load()
        items = []
        for row in rows:
            t = Transcription.from_row(row)
            # The client id guards against a recreated database reusing ids.
            if (fix := fixes.get(str(t.id))) and fix.get("client_id") in (None, t.client_id):
                t.correction = fix["text"]
            items.append(t)
        if search:
            needle = search.casefold()
            items = [t for t in items if needle in f"{t.final}\n{t.heard}".casefold()]
        return items

    def correct(self, t: Transcription, text: str | None) -> None:
        self.corrections.set(t, text)

    def delete_transcription(self, t: Transcription) -> None:
        self.bridge.request("DELETE", f"/v1/transcriptions/{t.id}")
        self.corrections.set(t, None)

    def training_pairs(self) -> Iterator[dict[str, Any]]:
        for t in self.transcriptions():
            if t.changed and t.status == "completed":
                yield {
                    "id": t.id, "timestamp": t.timestamp, "model": t.model,
                    "input": t.heard, "output": t.final,
                    "corrected": t.correction is not None,
                }

    # ---- dictionary: a list of words

    def dictionary(self) -> list[str]:
        return list(self.bridge.request("GET", "/v1/dictionary/list"))

    def update_dictionary(self, add: Sequence[str] = (), remove: Sequence[str] = ()) -> list[str]:
        return self.bridge.request("POST", "/v1/dictionary/update",
                                   {"add": list(add), "remove": list(remove)})["words"]

    # ---- snippets: {trigger, replacement}

    def snippets(self) -> list[dict[str, Any]]:
        return list(self.bridge.request("GET", "/v1/snippets/list"))

    def update_snippets(self, add: Sequence[dict[str, str]] = (), remove: Sequence[str] = ()) -> None:
        self.bridge.request("POST", "/v1/snippets/update",
                            {"add": list(add), "remove": list(remove)})
