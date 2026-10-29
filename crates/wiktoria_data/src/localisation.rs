//! `localisation/*.csv`: `KEY;English;French;German;;Spanish;...;x`, Windows-1252.
//!
//! Files are read in file-name order and the first definition of a key wins.
//! That precedence is a hypothesis until checked in game.

use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct Localisation {
    english: HashMap<String, String>,
}

impl Localisation {
    /// Adds one file's entries; keys already present are kept.
    pub fn add_file(&mut self, text: &str) {
        for line in text.lines() {
            if line.starts_with('#') {
                continue;
            }
            let mut cols = line.split(';');
            let (Some(key), Some(en)) = (cols.next(), cols.next()) else { continue };
            if key.is_empty() {
                continue;
            }
            self.english.entry(key.to_string()).or_insert_with(|| en.to_string());
        }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.english.get(key).map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.english.len()
    }

    pub fn is_empty(&self) -> bool {
        self.english.is_empty()
    }
}
