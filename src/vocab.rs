use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

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
}
