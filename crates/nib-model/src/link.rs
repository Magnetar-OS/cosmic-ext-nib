// SPDX-License-Identifier: MPL-2.0

//! Which link targets a document may carry and a click may follow.
//!
//! One predicate, used by every parser that reads a link from outside and by
//! the widget before it reports a click, so the three cannot disagree about
//! what is safe. Most documents that reach an editor came from somewhere
//! else — mail HTML, a downloaded Markdown file — and a link's target is
//! handed to the desktop's URL opener when it is clicked.

/// The schemes a link may carry.
///
/// An allow-list. `javascript:` and `data:` are the familiar dangers; `file:`
/// and the network filesystems (`smb:`, `sftp:`, …) are the ones a desktop
/// adds, opening a local path or mounting a share on a single click. Anything
/// not named here is refused rather than enumerated: a scheme nobody listed is
/// a scheme nobody reviewed.
pub const FOLLOWABLE_SCHEMES: [&str; 5] = ["http", "https", "mailto", "tel", "ftp"];

/// Whether a link target may be kept and followed.
///
/// Read the way a URL parser reads it, not the way it looks: leading and
/// trailing controls and spaces are ignored and tabs and newlines anywhere are
/// removed before the scheme is read, so `java\tscript:` is `javascript:`
/// here exactly as it is to whatever opens it. A target with no scheme —
/// `#section`, `/path`, `page.html?at=12:30` — is relative and kept.
///
/// ```
/// use nib_model::link::is_followable;
///
/// assert!(is_followable("https://example.test/"));
/// assert!(is_followable("#section"));
/// assert!(!is_followable("javascript:alert(1)"));
/// assert!(!is_followable(" file:///etc/passwd"));
/// ```
#[must_use]
pub fn is_followable(href: &str) -> bool {
    let cleaned: String = href
        .trim_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    let Some((scheme, _)) = cleaned.split_once(':') else {
        return true;
    };
    // A scheme is a letter and then letters, digits, `+`, `-` and `.`. A colon
    // after anything else sits in a path, a query or a fragment.
    let is_scheme = scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    !is_scheme
        || FOLLOWABLE_SCHEMES
            .iter()
            .any(|allowed| scheme.eq_ignore_ascii_case(allowed))
}
