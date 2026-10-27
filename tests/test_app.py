import asyncio, json, shutil, sqlite3
from pathlib import Path
from textual.widgets import DataTable, Input, TextArea
from owtui.app import OWTui, export_pairs
from owtui.store import History

SAMPLE = Path(__file__).parent.parent / "sample-data"

async def run(tmp: Path):
    app = OWTui(tmp)
    async with app.run_test(size=(160, 45)) as pilot:
        assert app.query_one("#history-table", DataTable).row_count == 3
        # search
        app.query_one("#history-search", Input).value = "kubectl"
        await pilot.pause()
        assert app.query_one("#history-table", DataTable).row_count == 1
        # edit history entry
        app.query_one("#history-table", DataTable).focus()
        await pilot.press("e"); await pilot.pause()
        app.screen.query_one("#text", TextArea).text = "Run kubectl get pods in staging."
        await pilot.press("ctrl+s"); await pilot.pause()
        # dictionary: add word
        app.query_one("#tabs").active = "tab-dictionary"; await pilot.pause()
        app.query_one("#dict-table", DataTable).focus()
        await pilot.press("a"); await pilot.pause()
        app.screen.query_one("#term", Input).value = "OpenWhisper"
        app.screen.query_one("#heard", Input).value = "open whisper, open whispers"
        await pilot.press("ctrl+s"); await pilot.pause()
        assert app.query_one("#dict-table", DataTable).row_count == 2
        await pilot.press("s"); await pilot.pause()   # star top row
        # snippets: delete
        app.query_one("#tabs").active = "tab-snippets"; await pilot.pause()
        app.query_one("#snip-table", DataTable).focus()
        await pilot.press("d"); await pilot.pause(); await pilot.press("y"); await pilot.pause()
        # prompt
        app.query_one("#tabs").active = "tab-prompt"; await pilot.pause()
        app.query_one("#prompt-text", TextArea).text = "Fix punctuation only."
        await pilot.click("#save-prompt"); await pilot.pause()

def test_end_to_end(tmp_path):
    shutil.copytree(SAMPLE, tmp_path, dirs_exist_ok=True)
    asyncio.run(run(tmp_path))
    s = json.loads((tmp_path / "openwhisper_settings.json").read_text())
    assert s["other_setting"] == 42
    assert s["dictation_dictionary"][0]["term"] == "OpenWhisper"
    assert s["dictation_dictionary"][0]["heard"] == ["open whisper", "open whispers"]
    assert s["dictation_dictionary"][0]["starred"] is True
    assert s["dictation_snippets"] == []
    assert s["transcript_cleanup_prompt"] == "Fix punctuation only."
    assert (tmp_path / "openwhisper_settings.json.owtui.bak").exists()
    text = sqlite3.connect(tmp_path / "openwhisper.db").execute(
        "SELECT text FROM transcription_history WHERE raw_text LIKE 'run cube%'").fetchone()[0]
    assert text == "Run kubectl get pods in staging."
    n = export_pairs(History(tmp_path / "openwhisper.db"), tmp_path / "p.jsonl")
    assert n == 2
