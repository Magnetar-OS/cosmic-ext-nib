// SPDX-License-Identifier: MPL-2.0

//! Spell checking for a document.
//!
//! # The dictionaries are the system's
//!
//! This crate ships no words. Every Linux distribution packages Hunspell
//! dictionaries already, in every language its users speak, kept current by
//! people who know the language — and an editor that bundled its own would be
//! shipping one stale copy of one of them, in a licence it would have to
//! account for, that could not be updated without a release.
//!
//! So [`Speller::system`] looks where dictionaries live and uses what is
//! there. When there is nothing there, spell checking is simply off: no
//! squiggles, no error, and a settings pane that says which package would turn
//! it on. That is the honest failure, and it is the one a distribution can fix
//! by declaring a recommended dependency.
//!
//! # What is not checked
//!
//! Code, and anything marked as code. A variable name is not a misspelling,
//! and an editor that says it is teaches people to ignore the squiggles —
//! which costs more than the feature is worth.
//!
//! Words with digits in them, and words of one letter, for the same reason.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use nib_model::decoration::{Decoration, DecorationSet, Style};
use nib_model::node::Node;
use spellbook::Dictionary;
use unicode_segmentation::UnicodeSegmentation;

/// The class a misspelling is decorated with.
pub const MISSPELLED: &str = "spelling-error";

/// Where Hunspell dictionaries live, in the order they are looked for.
///
/// The user's own first, so a dictionary they installed for themselves wins
/// over the system's.
fn search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        paths.push(home.join(".local/share/hunspell"));
        paths.push(home.join(".hunspell"));
    }
    paths.extend(
        [
            "/usr/share/hunspell",
            "/usr/share/myspell",
            "/usr/share/myspell/dicts",
            "/usr/local/share/hunspell",
        ]
        .iter()
        .map(PathBuf::from),
    );
    paths
}

/// A spell checker over one language.
pub struct Speller {
    dictionary: Dictionary,
    language: String,
    /// Words the user has said are words.
    personal: BTreeSet<String>,
}

impl std::fmt::Debug for Speller {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Speller")
            .field("language", &self.language)
            .field("personal", &self.personal.len())
            .finish_non_exhaustive()
    }
}

/// Why a dictionary could not be loaded.
#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error("no dictionary for {0} in any of the usual places")]
    NotFound(String),
    #[error("{path}: {message}")]
    Unreadable { path: String, message: String },
    #[error("{path} is not a dictionary this can read: {message}")]
    Malformed { path: String, message: String },
}

impl Speller {
    /// Loads the system's dictionary for a language tag such as `en_US`.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] when no dictionary for the language is installed,
    /// which is a normal state rather than a fault.
    pub fn system(language: &str) -> Result<Self, Error> {
        for directory in search_paths() {
            let aff = directory.join(format!("{language}.aff"));
            let dic = directory.join(format!("{language}.dic"));
            if aff.is_file() && dic.is_file() {
                return Self::from_files(&aff, &dic, language);
            }
        }
        Err(Error::NotFound(language.to_owned()))
    }

    /// Every language a dictionary is installed for.
    ///
    /// For a settings pane: a list of what can be chosen is worth more than a
    /// text field that silently does nothing when it is wrong.
    #[must_use]
    pub fn installed() -> Vec<String> {
        let mut found = BTreeSet::new();
        for directory in search_paths() {
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("dic") {
                    continue;
                }
                let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                if path.with_extension("aff").is_file() {
                    found.insert(stem.to_owned());
                }
            }
        }
        found.into_iter().collect()
    }

    /// Loads a dictionary from a specific pair of files.
    ///
    /// # Errors
    ///
    /// [`Error`] when either file cannot be read or parsed.
    pub fn from_files(aff: &Path, dic: &Path, language: &str) -> Result<Self, Error> {
        let read = |path: &Path| {
            std::fs::read_to_string(path).map_err(|e| Error::Unreadable {
                path: path.display().to_string(),
                message: e.to_string(),
            })
        };
        Self::from_strings(&read(aff)?, &read(dic)?, language).map_err(|message| {
            Error::Malformed {
                path: dic.display().to_string(),
                message,
            }
        })
    }

    /// Loads a dictionary from the contents of the two files.
    ///
    /// # Errors
    ///
    /// The parse error, as a string, when the pair does not parse.
    pub fn from_strings(aff: &str, dic: &str, language: &str) -> Result<Self, String> {
        let dictionary = Dictionary::new(aff, dic).map_err(|e| e.to_string())?;
        Ok(Self {
            dictionary,
            language: language.to_owned(),
            personal: BTreeSet::new(),
        })
    }

    #[must_use]
    pub fn language(&self) -> &str {
        &self.language
    }

    /// Adds a word to the personal dictionary.
    pub fn learn(&mut self, word: &str) {
        self.personal.insert(word.to_owned());
    }

    /// Forgets a word previously learnt.
    pub fn unlearn(&mut self, word: &str) {
        self.personal.remove(word);
    }

    /// The words the user has taught it.
    pub fn learnt(&self) -> impl Iterator<Item = &str> {
        self.personal.iter().map(String::as_str)
    }

    /// True when the word is spelled correctly, or is not the kind of thing to
    /// check.
    #[must_use]
    pub fn is_correct(&self, word: &str) -> bool {
        if !worth_checking(word) {
            return true;
        }
        if self.personal.contains(word) {
            return true;
        }
        self.dictionary.check(word)
    }

    /// What the word might have been meant to be.
    #[must_use]
    pub fn suggest(&self, word: &str) -> Vec<String> {
        let mut out = Vec::new();
        self.dictionary.suggest(word, &mut out);
        out
    }

    /// Decorations marking every misspelling in a document.
    #[must_use]
    pub fn decorate(&self, doc: &Node) -> DecorationSet {
        let mut out = Vec::new();
        doc.descendants(&mut |node, pos, _, _| {
            // Code is not prose. A variable name is not a misspelling, and an
            // editor that says it is teaches people to ignore the squiggles.
            if node.typ().spec().code {
                return false;
            }
            if !node.is_textblock() {
                return true;
            }
            self.check_block(node, pos + 1, &mut out);
            false
        });
        DecorationSet::new(out)
    }

    /// The misspelled word at a position, and where it is.
    ///
    /// For a menu that offers to correct or learn the word under the pointer.
    #[must_use]
    pub fn word_at(&self, doc: &Node, position: usize) -> Option<(String, usize, usize)> {
        let at = doc.resolve(position);
        if !at.parent().is_textblock() || at.parent().typ().spec().code {
            return None;
        }
        let start = at.start(at.depth());
        let text = at.parent().text_content();
        let offset = position.saturating_sub(start).min(text.len());

        text.unicode_word_indices().find_map(|(index, word)| {
            let (from, to) = (index, index + word.len());
            (offset >= from && offset <= to && !self.is_correct(word))
                .then(|| (word.to_owned(), start + from, start + to))
        })
    }

    /// Scans one textblock, skipping runs marked as code.
    fn check_block(&self, node: &Node, start: usize, out: &mut Vec<Decoration>) {
        let mut offset = start;
        for child in node.content() {
            let size = child.node_size();
            let Some(text) = child.text() else {
                offset += size;
                continue;
            };
            // Inline code is code too.
            let is_code = child
                .marks()
                .iter()
                .any(|mark| mark.typ().spec().code);
            if is_code {
                offset += size;
                continue;
            }
            for (index, word) in text.unicode_word_indices() {
                if self.is_correct(word) {
                    continue;
                }
                out.push(Decoration::inline(
                    offset + index,
                    offset + index + word.len(),
                    Style {
                        class: Some(MISSPELLED.into()),
                        underline: Some(true),
                        ..Style::default()
                    },
                ));
            }
            offset += size;
        }
    }
}

/// Whether a word is the kind of thing a dictionary has an opinion about.
///
/// Not: single letters, anything with a digit in it, and anything that looks
/// like a URL or a path. Each of those produces squiggles nobody wants, and a
/// squiggle nobody wants is one that teaches people to ignore all of them.
#[must_use]
pub fn worth_checking(word: &str) -> bool {
    if word.chars().count() < 2 {
        return false;
    }
    if word.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }
    if word.contains('_') || word.contains('/') || word.contains('\\') {
        return false;
    }
    // A word in the middle of a camelCase identifier is not prose either, but
    // `unicode_word_indices` has already split those apart, so what arrives
    // here is a fragment — checking it would flag half of every identifier.
    true
}
