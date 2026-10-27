# owtui

A small terminal UI for [OpenWhispr](https://github.com/OpenWhispr/openwhispr):
browse what you've dictated, correct it, and manage the dictionary and
snippets.

```bash
uv run owtui                       # open the TUI
uv run owtui --export pairs.jsonl  # write training pairs and exit
```

## How it talks to OpenWhispr

- **Live:** through the app's CLI bridge, a local HTTP API on 127.0.0.1
  described in `~/.openwhispr/cli-bridge.json`. Dictionary, snippet and delete
  changes show up in the running app right away and sync like edits made there.
  OpenWhispr's code warns against writing its SQLite tables by hand, so owtui never does.
- **App closed:** history is read from
  `~/Library/Application Support/open-whispr/transcriptions.db` read-only;
  changes need the app running.

## Tabs and keys

| Tab | What | Keys |
| --- | --- | --- |
| History | search, preview (app text, heard text, your correction), correct, delete | `/` search, `e`/Enter correct, `d` delete |
| Dictionary | words that steer recognition and cleanup | `a` add (comma-separated), `e` rename, `d` delete |
| Snippets | trigger → replacement text | `a` add, `e` edit, `d` delete |
| Stats | counts, words, audio minutes, models, training pairs | |

`x` exports pairs, `r` reloads, `q` quits.

## Corrections and training

OpenWhispr can't change a transcript's text from outside, so corrections live
in owtui's own file, `~/.local/share/owtui/corrections.json`, and are marked
`✎` in History. The export writes one JSON line per transcript where the
final text differs from what the speech engine heard:

```json
{"id": 22, "timestamp": "...", "model": "orukeet-v0.1.0",
 "input": "<heard>", "output": "<your correction, else the app's cleaned text>",
 "corrected": true}
```

That's the shape for fine-tuning a cleanup model, or for mining new
dictionary words.
