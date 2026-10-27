# Changelog

## 0.1.0 — 2026-10-27

First release. An unofficial terminal UI (Rust + Ratatui) for
[OpenWhispr](https://github.com/OpenWhispr/openwhispr) dictation history, not
affiliated with or endorsed by OpenWhispr.

- **History:** search, preview, play recordings, correct transcripts, delete.
  Corrections are kept in owtui's own file and never change OpenWhispr's text.
- **Editing:** click a word in the preview to edit there, double-click to
  select it, word jumps (⌥←/→), Ctrl+W, and a quick-fix (Ctrl+D) that replaces
  a misheard phrase everywhere in the transcript and adds the right word to the
  dictionary. Clickable Save/Cancel/Play/Revert buttons.
- **Insights:** ranks repeated mistakes (heard → should be), adds fixes to the
  dictionary with one key, and says whether the dictionary still covers your
  errors or a fine-tune is worth trying.
- **Dictionary and snippets:** add, edit and delete through OpenWhispr's local
  CLI bridge, so the running app updates right away.
- **Privacy:** shows whether you're signed in, cloud backup and usage-stats
  sync, whether speech and cleanup run on this Mac, what has already left it,
  and what would upload if backup were turned on. A badge in the header turns
  yellow or red when that changes.
- **Stats** and **training export** (`x` or `owtui --export FILE`): JSONL pairs
  of what the engine heard and the final text.
- **Live updates:** watches OpenWhispr's database files and reloads when they
  change, keeping the selected transcript selected.
- **Mouse:** tabs, rows, preview words, editor text and dialog buttons are all
  clickable; works around herdr passing mouse positions in pixels.
