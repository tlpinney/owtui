//! Which words the speech engine keeps getting wrong.
//!
//! Each transcript's heard text is compared word by word with its final text
//! (your correction, else the AI-cleaned text). Case and surrounding
//! punctuation are ignored, so cleanup that only adds capitals and full stops
//! doesn't count. Replacements that repeat are dictionary candidates; how much
//! of all word errors they cover says whether the dictionary is still enough.

use crate::store::Transcription;
use std::collections::HashMap;

/// Corrections by you before a fine-tune has enough to learn from.
pub const FINE_TUNE_MIN_CORRECTIONS: usize = 200;
/// Share of word errors that repeated mistakes must cover to keep using the dictionary.
pub const DICTIONARY_COVERAGE: f64 = 0.6;
const MAX_FIXED_WORDS: usize = 4;
const MAX_HEARD_WORDS: usize = 6;

struct Token {
    norm: String,
    shown: String,
}

fn tokens(text: &str) -> Vec<Token> {
    text.split_whitespace()
        .filter_map(|w| {
            let shown = w.trim_matches(|c: char| !c.is_alphanumeric());
            (!shown.is_empty()).then(|| Token {
                norm: shown.to_lowercase(),
                shown: shown.to_string(),
            })
        })
        .collect()
}

#[derive(Default, Debug, PartialEq)]
pub struct Hunk {
    pub heard: Vec<String>,
    pub fixed: Vec<String>,
}

/// The runs of words that differ between `heard` and `fixed`.
pub fn hunks(heard: &str, fixed: &str) -> Vec<Hunk> {
    let (a, b) = (tokens(heard), tokens(fixed));
    let (n, m) = (a.len(), b.len());
    if n * m > 1_000_000 {
        return Vec::new();
    }
    // Longest common subsequence, then walk it collecting the gaps.
    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i].norm == b[j].norm {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut out = Vec::new();
    let mut current = Hunk::default();
    let flush = |current: &mut Hunk, out: &mut Vec<Hunk>| {
        if !current.heard.is_empty() || !current.fixed.is_empty() {
            out.push(std::mem::take(current));
        }
    };
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && a[i].norm == b[j].norm {
            flush(&mut current, &mut out);
            i += 1;
            j += 1;
        } else if j < m && (i == n || lcs[i][j + 1] >= lcs[i + 1][j]) {
            current.fixed.push(b[j].shown.clone());
            j += 1;
        } else {
            current.heard.push(a[i].shown.clone());
            i += 1;
        }
    }
    flush(&mut current, &mut out);
    out
}

#[derive(Debug, Clone)]
pub struct Pattern {
    pub heard: String,
    pub fixed: String,
    pub count: usize,
    /// Times it was fixed by your correction rather than by AI cleanup.
    pub by_you: usize,
    /// Words this mistake costs each time it happens.
    weight: usize,
    /// Newest transcript showing it.
    pub example: i64,
    pub in_dictionary: bool,
}

#[derive(Debug, Default)]
pub struct Insights {
    pub patterns: Vec<Pattern>,
    /// Transcripts whose final text differs from what was heard.
    pub compared: usize,
    pub corrected_by_you: usize,
    pub word_errors: usize,
    pub repeated_errors: usize,
}

impl Insights {
    pub fn coverage(&self) -> f64 {
        if self.word_errors == 0 {
            0.0
        } else {
            self.repeated_errors as f64 / self.word_errors as f64
        }
    }

    /// The recommendation, and whether it points at fine-tuning.
    pub fn verdict(&self) -> (String, bool) {
        let pct = (self.coverage() * 100.0).round();
        if self.word_errors == 0 {
            (
                "No word differences yet. Correct some transcripts on History (e).".into(),
                false,
            )
        } else if self.corrected_by_you < FINE_TUNE_MIN_CORRECTIONS {
            (
                format!(
                    "Use the dictionary. {} of ~{FINE_TUNE_MIN_CORRECTIONS} corrections needed before a fine-tune is worth it.",
                    self.corrected_by_you
                ),
                false,
            )
        } else if self.coverage() >= DICTIONARY_COVERAGE {
            (
                format!("Use the dictionary: repeated mistakes cover {pct}% of word errors."),
                false,
            )
        } else {
            (
                format!("Consider a fine-tune: only {pct}% of word errors repeat, so a dictionary can't list them."),
                true,
            )
        }
    }
}

pub fn analyze(items: &[Transcription], dictionary: &[String]) -> Insights {
    let known: Vec<String> = dictionary.iter().map(|w| w.to_lowercase()).collect();
    let mut insights = Insights::default();
    let mut by_key: HashMap<(String, String), usize> = HashMap::new();

    // Items are newest first, so the first sighting is the newest example.
    for t in items.iter().filter(|t| t.changed()) {
        let found = hunks(t.heard(), t.final_text());
        if found.is_empty() {
            continue;
        }
        insights.compared += 1;
        let by_you = t.correction.is_some();
        if by_you {
            insights.corrected_by_you += 1;
        }
        for hunk in found {
            let weight = hunk.heard.len().max(hunk.fixed.len());
            insights.word_errors += weight;
            if hunk.heard.is_empty()
                || hunk.fixed.is_empty()
                || hunk.heard.len() > MAX_HEARD_WORDS
                || hunk.fixed.len() > MAX_FIXED_WORDS
            {
                continue;
            }
            let heard = hunk.heard.join(" ");
            let fixed = hunk.fixed.join(" ");
            let key = (heard.to_lowercase(), fixed.to_lowercase());
            let index = *by_key.entry(key).or_insert_with(|| {
                insights.patterns.push(Pattern {
                    in_dictionary: known.contains(&fixed.to_lowercase()),
                    heard,
                    fixed,
                    count: 0,
                    by_you: 0,
                    weight,
                    example: t.id,
                });
                insights.patterns.len() - 1
            });
            let pattern = &mut insights.patterns[index];
            pattern.count += 1;
            pattern.by_you += usize::from(by_you);
        }
    }
    insights.repeated_errors = insights
        .patterns
        .iter()
        .filter(|p| p.count >= 2)
        .map(|p| p.count * p.weight)
        .sum();
    insights
        .patterns
        .sort_by(|a, b| b.count.cmp(&a.count).then(b.by_you.cmp(&a.by_you)));
    insights
}

/// Replaces whole-word, case-insensitive occurrences of `phrase` in `text`.
/// Spaces in the phrase match any run of whitespace. Returns the count.
pub fn replace_phrase(text: &str, phrase: &str, with: &str) -> (String, usize) {
    fn low(c: char) -> char {
        c.to_lowercase().next().unwrap_or(c)
    }
    fn word_char(c: char) -> bool {
        c.is_alphanumeric() || c == '\''
    }
    let words: Vec<Vec<char>> = phrase
        .split_whitespace()
        .map(|w| w.chars().map(low).collect())
        .collect();
    if words.is_empty() {
        return (text.to_string(), 0);
    }
    let chars: Vec<char> = text.chars().collect();
    let lower: Vec<char> = chars.iter().copied().map(low).collect();
    let match_at = |mut i: usize| -> Option<usize> {
        for (k, word) in words.iter().enumerate() {
            if k > 0 {
                let start = i;
                while i < lower.len() && lower[i].is_whitespace() {
                    i += 1;
                }
                if i == start {
                    return None;
                }
            }
            if lower.get(i..i + word.len())? != word.as_slice() {
                return None;
            }
            i += word.len();
        }
        Some(i)
    };
    let (mut out, mut count, mut i) = (String::new(), 0, 0);
    while i < chars.len() {
        if i == 0 || !word_char(chars[i - 1]) {
            if let Some(end) = match_at(i).filter(|&e| e == chars.len() || !word_char(chars[e])) {
                out.push_str(with);
                count += 1;
                i = end;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    (out, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: i64, heard: &str, text: &str, correction: Option<&str>) -> Transcription {
        Transcription {
            id,
            text: text.into(),
            raw_text: Some(heard.into()),
            status: "completed".into(),
            correction: correction.map(String::from),
            ..Default::default()
        }
    }

    #[test]
    fn finds_replacements_and_ignores_punctuation() {
        assert_eq!(hunks("hi sam how are you", "Hi Sam, how are you?"), vec![]);
        assert_eq!(
            hunks("open the dewy to the right", "Open the TUI to the right."),
            vec![Hunk {
                heard: vec!["dewy".into()],
                fixed: vec!["TUI".into()]
            }]
        );
        assert_eq!(
            hunks("written in red at two e", "written in Ratatui"),
            vec![Hunk {
                heard: vec!["red".into(), "at".into(), "two".into(), "e".into()],
                fixed: vec!["Ratatui".into()]
            }]
        );
    }

    #[test]
    fn ranks_repeats_and_scores_coverage() {
        let items = vec![
            item(3, "open the dewy", "Open the dewy.", Some("Open the TUI.")),
            item(2, "a dewy app", "A dewy app.", Some("A TUI app.")),
            item(1, "I use quinn", "I use Qwen.", None),
        ];
        let insights = analyze(&items, &["TUI".into()]);
        assert_eq!(insights.compared, 3);
        assert_eq!(insights.corrected_by_you, 2);
        let top = &insights.patterns[0];
        assert_eq!(
            (
                top.heard.as_str(),
                top.fixed.as_str(),
                top.count,
                top.by_you
            ),
            ("dewy", "TUI", 2, 2)
        );
        assert!(top.in_dictionary);
        assert_eq!(top.example, 3);
        assert_eq!(insights.patterns[1].fixed, "Qwen");
        assert!(!insights.patterns[1].in_dictionary);
        assert_eq!((insights.word_errors, insights.repeated_errors), (3, 2));
        assert!(!insights.verdict().1);
    }

    #[test]
    fn replaces_whole_phrases_only() {
        assert_eq!(
            replace_phrase("Red at two e is red  at Two E.", "red at two e", "Ratatui"),
            ("Ratatui is Ratatui.".into(), 2)
        );
        assert_eq!(
            replace_phrase("dewy dewyish dewy", "dewy", "TUI"),
            ("TUI dewyish TUI".into(), 2)
        );
        assert_eq!(replace_phrase("nothing", "", "x"), ("nothing".into(), 0));
    }
}
