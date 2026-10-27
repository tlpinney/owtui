"""Drives owtui against a fake OpenWhispr bridge; never touches real data."""
import asyncio, json, threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from textual.widgets import DataTable, Input, TextArea
from owtui.app import OWTui, export_pairs
from owtui.store import Bridge, Corrections, Store

TOKEN = "t"


def fake_app():
    state = {
        "transcriptions": [
            {"id": 2, "timestamp": "2026-10-27T22:20:28.953Z", "text": "Run kube control get pods.",
             "raw_text": "run kube control get pods", "model": "parakeet", "has_audio": 1,
             "audio_duration_ms": 3000, "status": "completed", "client_transcription_id": "c2"},
            {"id": 1, "timestamp": "2026-10-26T22:00:00Z", "text": "Hello there.", "raw_text": "Hello there.",
             "model": "parakeet", "status": "completed", "client_transcription_id": "c1"},
        ],
        "words": ["OpenWhispr"],
        "snippets": [{"trigger": "my link", "replacement": "https://example.com"}],
    }

    class H(BaseHTTPRequestHandler):
        def log_message(self, *a): pass

        def _send(self, data, code=200):
            body = b"" if data is None else json.dumps({"data": data}).encode()
            self.send_response(204 if data is None else code)
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            assert self.headers["Authorization"] == f"Bearer {TOKEN}"
            if self.path.startswith("/v1/transcriptions/list"):
                self._send(state["transcriptions"])
            elif self.path == "/v1/dictionary/list":
                self._send(state["words"])
            elif self.path == "/v1/snippets/list":
                self._send(state["snippets"])

        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            if self.path == "/v1/dictionary/update":
                state["words"] = [w for w in state["words"] if w not in body["remove"]] + body["add"]
                self._send({"words": state["words"]})
            elif self.path == "/v1/snippets/update":
                gone = {t.lower() for t in body["remove"]} | {s["trigger"].lower() for s in body["add"]}
                state["snippets"] = [s for s in state["snippets"] if s["trigger"].lower() not in gone] + body["add"]
                self._send({})

        def do_DELETE(self):
            tid = int(self.path.rsplit("/", 1)[1])
            state["transcriptions"] = [t for t in state["transcriptions"] if t["id"] != tid]
            self._send(None)

    server = ThreadingHTTPServer(("127.0.0.1", 0), H)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server, state


async def drive(app):
    async with app.run_test(size=(160, 45)) as pilot:
        assert app.query_one("#history-table", DataTable).row_count == 2
        app.query_one("#history-search", Input).value = "kube"
        await pilot.pause()
        assert app.query_one("#history-table", DataTable).row_count == 1
        app.query_one("#history-table", DataTable).focus()
        await pilot.press("e"); await pilot.pause()
        app.screen.query_one("#text", TextArea).text = "Run kubectl get pods."
        await pilot.press("ctrl+s"); await pilot.pause()

        app.query_one("#tabs").active = "tab-dictionary"; await pilot.pause()
        app.query_one("#dict-table", DataTable).focus()
        await pilot.press("a"); await pilot.pause()
        app.screen.query_one("#word", Input).value = "kubectl, Textual"
        await pilot.press("ctrl+s"); await pilot.pause()
        assert app.query_one("#dict-table", DataTable).row_count == 3
        await pilot.press("d"); await pilot.pause(); await pilot.press("y"); await pilot.pause()

        app.query_one("#tabs").active = "tab-snippets"; await pilot.pause()
        app.query_one("#snip-table", DataTable).focus()
        await pilot.press("e"); await pilot.pause()
        app.screen.query_one("#trigger", Input).value = "my site"
        await pilot.press("ctrl+s"); await pilot.pause()

        app.query_one("#history-search", Input).value = ""
        app.query_one("#tabs").active = "tab-history"; await pilot.pause()
        app.query_one("#history-table", DataTable).focus()
        app.query_one("#history-table", DataTable).move_cursor(row=1)
        await pilot.press("d"); await pilot.pause(); await pilot.press("y"); await pilot.pause()


def test_end_to_end(tmp_path):
    server, state = fake_app()
    (tmp_path / "bridge.json").write_text(json.dumps({"port": server.server_port, "token": TOKEN}))
    store = Store(Bridge(tmp_path / "bridge.json"), tmp_path / "missing.db", Corrections(tmp_path / "c.json"))
    asyncio.run(drive(OWTui(store)))
    server.shutdown()

    fixes = json.loads((tmp_path / "c.json").read_text())
    assert fixes["2"]["text"] == "Run kubectl get pods."
    assert fixes["2"]["raw_text"] == "run kube control get pods"
    assert state["words"] == ["kubectl", "Textual"]           # OpenWhispr deleted (top row)
    assert state["snippets"] == [{"trigger": "my site", "replacement": "https://example.com"}]
    assert [t["id"] for t in state["transcriptions"]] == [2]


def test_offline_reads_db_and_exports(tmp_path):
    import sqlite3
    db = tmp_path / "transcriptions.db"
    conn = sqlite3.connect(db)
    conn.execute("CREATE TABLE transcriptions (id INTEGER PRIMARY KEY, text TEXT, raw_text TEXT, timestamp TEXT,"
                 " model TEXT, status TEXT, deleted_at TEXT, client_transcription_id TEXT)")
    conn.execute("INSERT INTO transcriptions VALUES (1,'Hi Sam.','hi sam','2026-10-27T00:00:00Z','p','completed',NULL,'c1')")
    conn.execute("INSERT INTO transcriptions VALUES (2,'gone','gone','2026-10-27T00:00:00Z','p','completed','x','c2')")
    conn.commit(); conn.close()
    store = Store(Bridge(tmp_path / "nope.json"), db, Corrections(tmp_path / "c.json"))
    items = store.transcriptions()
    assert [t.id for t in items] == [1] and not store.live
    n = export_pairs(store, tmp_path / "p.jsonl")
    assert n == 1
    assert json.loads((tmp_path / "p.jsonl").read_text()) | {} == {
        "id": 1, "timestamp": "2026-10-27T00:00:00Z", "model": "p",
        "input": "hi sam", "output": "Hi Sam.", "corrected": False}
