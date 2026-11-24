use std::collections::HashMap;

use serde::Serialize;

use crate::dict::LocalJmDict;
use crate::vocab::VocabStore;

#[derive(Debug, Clone, Serialize)]
pub struct CalibrationCandidate {
    pub word: String,
    pub reading: String,
    pub gloss: String,
    pub prob: f64,
    pub common: bool,
}

/// Lightweight probabilistic decision model:
/// Given the user's known word corpus K, estimates P(known | word, K, JMdict) in O(1) time.
/// Used for (1) Legendary Purple highlighting on T1-T3 words the user likely already knows, and
/// (2) proactive vocabulary calibration by the Kiku Immersion Agent.
pub struct KnowledgeModel {
    kanji_known: HashMap<char, u32>,
    known_size: usize,
}

fn is_kanji(c: char) -> bool {
    ('\u{4E00}'..='\u{9FFF}').contains(&c) || c == '々'
}

impl KnowledgeModel {
    pub fn build(vocab: &VocabStore) -> Self {
        let mut kanji_known: HashMap<char, u32> = HashMap::new();
        for w in &vocab.known {
            for ch in w.chars() {
                if is_kanji(ch) {
                    *kanji_known.entry(ch).or_insert(0) += 1;
                }
            }
        }
        Self {
            kanji_known,
            known_size: vocab.known.len(),
        }
    }

    pub fn predict_prob(&self, word: &str, dict: Option<&LocalJmDict>) -> f64 {
        let trimmed = word.trim();
        if trimmed.is_empty() {
            return 0.0;
        }
        let is_common = dict.map(|d| d.is_common_word(trimmed)).unwrap_or(false);

        // Kanji familiarity ratio: fraction of kanji characters in `word` already seen in known words
        let mut kanji_total = 0u32;
        let mut kanji_seen = 0u32;
        let mut kanji_depth = 0.0f64;
        for ch in trimmed.chars() {
            if is_kanji(ch) {
                kanji_total += 1;
                if let Some(&cnt) = self.kanji_known.get(&ch) {
                    kanji_seen += 1;
                    kanji_depth += (cnt as f64).min(5.0) / 5.0;
                }
            }
        }
        let kanji_ratio = if kanji_total > 0 {
            (kanji_seen as f64) / (kanji_total as f64)
        } else {
            // Pure kana common words are often known even without kanji signals
            if is_common { 0.55 } else { 0.1 }
        };
        let depth_bonus = if kanji_total > 0 {
            (kanji_depth / (kanji_total as f64)) * 0.45
        } else {
            0.0
        };

        let char_len = trimmed.chars().count();
        let short_bonus = if char_len <= 2 { 0.45 } else if char_len == 3 { 0.15 } else { 0.0 };
        // Smooth prior that scales gently with the size of the user's known corpus
        let corpus_prior = ((self.known_size as f64 + 1.0).ln() / 7.5).min(1.0) * 0.65;

        let z = -2.15
            + if is_common { 2.05 } else { 0.0 }
            + kanji_ratio * 1.90
            + depth_bonus
            + short_bonus
            + corpus_prior;

        1.0 / (1.0 + (-z).exp())
    }

    /// Returns true when an untagged word has high probability (>= 0.72) of already being known.
    pub fn is_likely_known(&self, word: &str, dict: Option<&LocalJmDict>) -> bool {
        self.predict_prob(word, dict) >= 0.72
    }

    /// Surfaces top untagged candidate words from a list of observed corpus words for 1-click calibration.
    pub fn suggest_calibration(
        &self,
        vocab: &VocabStore,
        dict: Option<&LocalJmDict>,
        candidates: &[String],
        limit: usize,
    ) -> Vec<CalibrationCandidate> {
        let mut seen = std::collections::HashSet::new();
        let mut scored: Vec<CalibrationCandidate> = Vec::new();
        for w in candidates {
            let clean = w.trim();
            if clean.is_empty()
                || vocab.known.contains(clean)
                || vocab.learning.contains(clean)
                || vocab.ignored.contains(clean)
                || vocab.priority.contains(clean)
                || !seen.insert(clean.to_string())
            {
                continue;
            }
            let prob = self.predict_prob(clean, dict);
            if prob < 0.62 {
                continue;
            }
            let common = dict.map(|d| d.is_common_word(clean)).unwrap_or(false);
            let (reading, gloss) = dict
                .and_then(|d| {
                    let hits = d.lookup(clean, clean, clean, "", None);
                    hits.first().map(|e| {
                        let g = e
                            .senses
                            .first()
                            .and_then(|s| s.defs.first())
                            .cloned()
                            .unwrap_or_default();
                        (e.reading.clone(), g)
                    })
                })
                .unwrap_or_else(|| (String::new(), String::new()));

            if gloss.is_empty() {
                continue;
            }
            scored.push(CalibrationCandidate {
                word: clean.to_string(),
                reading,
                gloss,
                prob: (prob * 100.0).round() / 100.0,
                common,
            });
        }
        scored.sort_by(|a, b| b.prob.total_cmp(&a.prob));
        scored.truncate(limit);
        scored
    }
}
