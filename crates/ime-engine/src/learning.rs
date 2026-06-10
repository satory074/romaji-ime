//! Usage learning: remembers which candidate the user committed for a given
//! input (the raw romaji "reading") and promotes it in future candidate lists.
//!
//! Persisted as a small JSON file under the per-user data dir. Pure Rust (no
//! SQLite) so it cross-compiles everywhere without a C toolchain.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Default, Serialize, Deserialize)]
pub struct Learning {
    /// reading (raw romaji) -> (candidate -> times chosen)
    counts: HashMap<String, HashMap<String, u32>>,
    #[serde(skip)]
    path: Option<PathBuf>,
}

impl Learning {
    /// Load from `path` (if it exists / parses); future writes go there.
    pub fn load(path: Option<PathBuf>) -> Self {
        let mut store = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str::<Learning>(&t).ok())
            .unwrap_or_default();
        store.path = path;
        store
    }

    /// Record that `chosen` was committed for `reading`, and persist.
    pub fn record(&mut self, reading: &str, chosen: &str) {
        if reading.is_empty() || chosen.is_empty() {
            return;
        }
        *self
            .counts
            .entry(reading.to_string())
            .or_default()
            .entry(chosen.to_string())
            .or_insert(0) += 1;
        self.save();
    }

    /// Reorder `candidates` so previously-chosen ones for `reading` come first
    /// (most-chosen first); the rest keep their original relative order.
    pub fn reorder(&self, reading: &str, candidates: &mut Vec<String>) {
        let Some(counts) = self.counts.get(reading) else {
            return;
        };
        let mut learned: Vec<String> = Vec::new();
        let mut rest: Vec<String> = Vec::new();
        for c in candidates.drain(..) {
            if counts.contains_key(&c) {
                learned.push(c);
            } else {
                rest.push(c);
            }
        }
        learned.sort_by(|a, b| counts[b].cmp(&counts[a])); // count desc, stable otherwise
        candidates.extend(learned);
        candidates.extend(rest);
    }

    fn save(&self) {
        if let Some(p) = &self.path {
            if let Ok(json) = serde_json::to_string(self) {
                let _ = std::fs::write(p, json);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn promotes_chosen_candidate() {
        let mut l = Learning::load(None); // in-memory (no persistence)
        let mut cands = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        l.reorder("x", &mut cands);
        assert_eq!(cands, ["A", "B", "C"]); // nothing learned yet

        l.record("x", "B");
        let mut cands = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        l.reorder("x", &mut cands);
        assert_eq!(cands, ["B", "A", "C"]); // B promoted, rest keep order
    }

    #[test]
    fn most_chosen_first() {
        let mut l = Learning::load(None);
        l.record("x", "B");
        l.record("x", "C");
        l.record("x", "C"); // C chosen twice, B once
        let mut cands = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        l.reorder("x", &mut cands);
        assert_eq!(cands, ["C", "B", "A"]);
    }

    #[test]
    fn unknown_reading_is_unchanged() {
        let mut l = Learning::load(None);
        l.record("x", "B");
        let mut cands = vec!["P".to_string(), "Q".to_string()];
        l.reorder("y", &mut cands);
        assert_eq!(cands, ["P", "Q"]);
    }
}
