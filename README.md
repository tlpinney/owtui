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

## Staying up to date

OpenWhispr's API has no change feed (neither the local bridge nor the cloud
API offers "changed since" or events), so owtui watches the size and modified
time of `transcriptions.db` and its `-wal` file once a second. That's a file
check, not a request; only when they change does it re-fetch, keeping the
selected transcript selected. New dictations, deletions and dictionary changes
made in the app show up on their own; `r` still reloads by hand.

## Tabs and keys

| Tab | What | Keys |
| --- | --- | --- |
| History | search, preview (app text, heard text, your correction), play, correct, delete | `/` search (Esc clears), `p` play/stop, `e`/Enter correct, `d` delete |
| Insights | mistakes that repeat (heard → should be), what share of word errors they cover, dictionary vs fine-tune advice | `a` add the fix to the dictionary, `p` play the example, Enter open it |
| Dictionary | words that steer recognition and cleanup | `a` add (comma-separated), `e` rename, `d` delete |
| Snippets | trigger → replacement text | `a` add, `e` edit, `d` delete |
| Stats | counts, words, audio minutes, models, training pairs | |
| Privacy | signed in?, cloud backup, usage-stats sync, whether speech and cleanup run on this Mac, what has already left the Mac, what would upload if backup were turned on | `r` recheck |

`↑↓`/`jk` move, `Tab`/`1`–`6` switch tabs, `x` exports pairs, `r` reloads, `q` quits.
Dialogs have clickable buttons (Save, Fix word, Play, Revert, Cancel; Yes/No when
deleting) that do the same as their keys. In editors: `Ctrl+S` saves, `Esc` cancels, `Ctrl+R` reverts a correction, `Ctrl+P` plays the recording.
Click a word in the History preview to open the editor with the cursor there;
double-click to select the word, then type to replace it (`Ctrl+D` fixes it
everywhere and adds it to the dictionary). In the editor, click to put the cursor anywhere; `⌥←`/`⌥→` (or `Ctrl+←`/`→`) jump by word, `Ctrl+W`
or `⌥⌫` deletes the previous word. Click a tab to switch to it. In lists, click a row to select it, double-click to open it, and scroll with the
wheel. Mouse capture means plain drag doesn't select terminal text;
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

## Troubleshooting the mouse

Set `OWTUI_MOUSE_LOG=/path/to/file` to log every mouse event with where the
list is drawn. Some hosts (herdr, when its terminal reports pixels) send
positions in pixels instead of cells; owtui detects that and converts them.

## Privacy

The Privacy tab, and the ● badge at the top right, read OpenWhispr's own files
(read-only): per-row `cloud_id` and `sync_status` in `transcriptions.db`, the
`Cookies` store for an account session, and the app's settings from
Chromium's localStorage log. OpenWhispr only uploads when you're signed in
**and** cloud backup is on **and** you have a subscription; the tab turns red
if that or any upload happens, and yellow if a cloud engine is selected.
