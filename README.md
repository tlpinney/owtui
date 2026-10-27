# owtui

A small terminal UI for [OpenWhispr](https://github.com/OpenWhispr/openwhispr):
browse what you've dictated, correct it, and manage the dictionary and
snippets.

Rust + [Ratatui](https://ratatui.rs), on the same toolchain as herdr
(`rust-toolchain.toml` pins 1.96.1; `just` for tasks).

```bash
just install                   # cargo install into ~/.cargo/bin
owtui                          # open the TUI
owtui --export pairs.jsonl     # write training pairs and exit

just test                      # cargo nextest (fake bridge, never real data)
just lint                      # fmt --check + clippy -D warnings
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
| History | search, preview (app text, heard text, your correction), play, correct, delete | `/` search (Esc clears), `p` play/stop, `e`/Enter correct, `d` delete |
| Insights | mistakes that repeat (heard → should be), what share of word errors they cover, dictionary vs fine-tune advice | `a` add the fix to the dictionary, `p` play the example, Enter open it |
| Dictionary | words that steer recognition and cleanup | `a` add (comma-separated), `e` rename, `d` delete |
| Snippets | trigger → replacement text | `a` add, `e` edit, `d` delete |
| Stats | counts, words, audio minutes, models, training pairs | |

`↑↓`/`jk` move, `Tab`/`1`–`5` switch tabs, `x` exports pairs, `r` reloads, `q` quits.
In editors: `Ctrl+S` saves, `Esc` cancels, `Ctrl+R` reverts a correction, `Ctrl+P` plays the recording.
Click a word in the History preview to open the editor with the cursor there;
double-click to select the word, then type to replace it (`Ctrl+D` fixes it
everywhere and adds it to the dictionary). In the editor, click to put the cursor anywhere; `⌥←`/`⌥→` (or `Ctrl+←`/`→`) jump by word, `Ctrl+W`
or `⌥⌫` deletes the previous word. In lists, click a row to select it, click it again
(or double-click) to open it, and scroll with the wheel. Mouse capture means plain drag doesn't select terminal text;
most terminals still select with Shift or Option held.
`Ctrl+D` in the correction editor opens a quick-fix: say what was heard and what it
should be, and it replaces every occurrence in that transcript and adds the word
to the dictionary.

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

## Audio

Recordings are OpenWhispr's `audio/OpenWhispr-<time>-<id>.webm` files (Opus).
owtui plays them with `ffplay`, falling back to `mpv`, then `afplay` (which
can't decode WebM). Install ffmpeg if neither is present: `brew install ffmpeg`.
