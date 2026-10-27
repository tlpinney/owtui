# owtui

A small terminal UI for OpenWhisper's local data: browse and edit what you've
dictated, and manage the dictionary, snippets and cleanup prompt.

```bash
uv run owtui                      # auto-detects the data folder
uv run owtui --data-dir PATH      # or point at one
uv run owtui --export pairs.jsonl # dump raw→final pairs and exit
uv run owtui --data-dir sample-data   # try it on the bundled sample
```

Data folder lookup: `--data-dir`, `$OPENWHISPER_DATA_DIR`,
`~/Library/Application Support/OpenWhisper` (macOS; `%LOCALAPPDATA%\OpenWhisper`
on Windows, `~/.local/share/OpenWhisper` on Linux), then `~/code/OpenWhisper`
(source checkout).

## Tabs and keys

| Tab | What | Keys |
| --- | --- | --- |
| History | search, preview, edit text/title, delete | `/` search, `e`/Enter edit, `d` delete |
| Dictionary | words, "sounds like" variants, star, protect | `a` add, `e` edit, `s` star, `d` delete |
| Snippets | trigger → text | `a` add, `e` edit, `d` delete |
| Prompt | custom cleanup prompt + learned rules | Save button |
| Stats | counts, top apps, number of edited pairs | |

`x` exports training pairs, `r` reloads, `q` quits. `✎` marks history
entries whose saved text differs from what the engine heard.

## How edits are written

- History: `UPDATE` on `transcription_history` in `openwhisper.db` (`text`,
  `title`). `raw_text` is never touched, so each edit becomes a better
  raw→final training pair.
- Settings: read-modify-write of `openwhisper_settings.json` with an atomic
  replace. The app re-reads the file on change, so this is safe while it
  runs. The first change in a session copies the file to
  `openwhisper_settings.json.owtui.bak`.

## Training (later)

`--export` / `x` writes JSONL with `{id, timestamp, app, input, output}` where
`input` is the raw engine text and `output` the final text, which is the shape for
fine-tuning a cleanup model or mining new dictionary words.
