use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::dict::LocalJmDict;
use crate::timeutil;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VocabStore {
    #[serde(default)]
    pub known: BTreeSet<String>,
    #[serde(default)]
    pub learning: BTreeSet<String>,
    #[serde(default)]
    pub ignored: BTreeSet<String>,
    #[serde(default)]
    pub priority: BTreeSet<String>,
    #[serde(default)]
    pub updated_at: String,
}

impl VocabStore {
    pub fn load(path: &Path) -> Self {
        if path.exists() {
            if let Ok(raw) = fs::read_to_string(path) {
                if let Ok(v) = serde_json::from_str::<VocabStore>(&raw) {
                    return v;
                }
            }
        }
        Self::default()
    }

    pub fn save(&mut self, path: &Path) {
        self.updated_at = timeutil::now_utc_rfc3339();
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let tmp = path.with_extension("json.tmp");
            if fs::write(&tmp, &json).is_ok() {
                let _ = fs::rename(&tmp, path);
            } else {
                let _ = fs::write(path, &json);
            }
        }
    }

    pub fn set_status(&mut self, word: &str, status: &str) {
        let w = word.trim().to_string();
        if w.is_empty() {
            return;
        }
        self.known.remove(&w);
        self.learning.remove(&w);
        self.ignored.remove(&w);
        self.priority.remove(&w);
        match status {
            "known" => {
                self.known.insert(w);
            }
            "learning" => {
                self.learning.insert(w);
            }
            "ignore" | "ignored" => {
                self.ignored.insert(w);
            }
            "priority" => {
                self.priority.insert(w);
            }
            _ => {
                // "unknown" is default (not stored in any set)
            }
        }
    }

    pub fn import_bulk(&mut self, text: &str, default_status: &str) -> usize {
        // Support full exported JSON format first
        if let Ok(parsed) = serde_json::from_str::<VocabStore>(text) {
            let mut count = 0;
            for w in parsed.known {
                self.set_status(&w, "known");
                count += 1;
            }
            for w in parsed.learning {
                self.set_status(&w, "learning");
                count += 1;
            }
            for w in parsed.ignored {
                self.set_status(&w, "ignore");
                count += 1;
            }
            return count;
        }

        // Otherwise newline / comma / tab separated words
        let mut count = 0;
        for line in text.lines() {
            for part in line.split([',', '\t']) {
                let w = part.trim().trim_matches('"');
                if !w.is_empty() && !w.starts_with('#') {
                    self.set_status(w, default_status);
                    count += 1;
                }
            }
        }
        count
    }

    pub fn export_txt(&self) -> String {
        self.known.iter().cloned().collect::<Vec<_>>().join("\n")
    }

    /// Deduplicates and canonicalizes all stored vocab sets against JMdict (e.g., folding
    /// verb masu-stems like 認め -> 認める, -く adverbials like 多く -> 多い, godan potentials
    /// like 打てる -> 打つ, and okurigana/kana duplicates like 分る/わかる -> 分かる) while
    /// preserving precedence: known > learning > priority > ignored.
    pub fn canonicalize_with_dict(&mut self, dict: &LocalJmDict) -> usize {
        let mut new_known = BTreeSet::new();
        let mut new_learning = BTreeSet::new();
        let mut new_priority = BTreeSet::new();
        let mut new_ignored = BTreeSet::new();

        for w in &self.known {
            let c = dict.canonicalize_lemma(w, "");
            if !c.is_empty() {
                new_known.insert(c);
            }
        }
        // Remove pure-kana duplicates from known if their kanji headword is already in known
        let kana_dups: Vec<String> = new_known
            .iter()
            .filter(|w| dict.find_known_kanji_alias(w, &new_known).is_some())
            .cloned()
            .collect();
        for d in kana_dups {
            new_known.remove(&d);
        }

        for w in &self.learning {
            let c = dict.canonicalize_lemma(w, "");
            if !c.is_empty()
                && !new_known.contains(&c)
                && dict.find_known_kanji_alias(&c, &new_known).is_none()
            {
                new_learning.insert(c);
            }
        }
        for w in &self.priority {
            let c = dict.canonicalize_lemma(w, "");
            if !c.is_empty()
                && !new_known.contains(&c)
                && !new_learning.contains(&c)
                && dict.find_known_kanji_alias(&c, &new_known).is_none()
            {
                new_priority.insert(c);
            }
        }
        for w in &self.ignored {
            let c = dict.canonicalize_lemma(w, "");
            if !c.is_empty()
                && !new_known.contains(&c)
                && !new_learning.contains(&c)
                && !new_priority.contains(&c)
            {
                new_ignored.insert(c);
            }
        }

        let changed = (new_known != self.known)
            || (new_learning != self.learning)
            || (new_priority != self.priority)
            || (new_ignored != self.ignored);
        if !changed {
            return 0;
        }
        let before = self.known.len() + self.learning.len() + self.priority.len() + self.ignored.len();
        self.known = new_known;
        self.learning = new_learning;
        self.priority = new_priority;
        self.ignored = new_ignored;
        let after = self.known.len() + self.learning.len() + self.priority.len() + self.ignored.len();
        before.abs_diff(after).max(1)
    }
}
