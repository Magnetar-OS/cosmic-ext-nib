// SPDX-License-Identifier: MPL-2.0

//! The one predicate every parser and the widget ask about a link's target.

use nib_model::link::is_followable;

#[test]
fn a_target_that_would_run_script_or_open_a_local_file_is_refused() {
    for href in [
        "javascript:alert(1)",
        " JavaScript:alert(1)",
        "java\tscript:alert(1)",
        "java\nscript:alert(1)",
        "\u{1}javascript:alert(1)",
        "vbscript:msgbox(1)",
        "data:text/html,<script>alert(1)</script>",
        "file:///home/user/.bashrc",
        "smb://attacker.example/share",
        "sftp://attacker.example/",
    ] {
        assert!(!is_followable(href), "{href:?} was allowed");
    }
}

#[test]
fn web_mail_phone_and_relative_targets_are_kept() {
    for href in [
        "https://example.test/",
        "HTTP://example.test/",
        "mailto:ada@example.test",
        "tel:+15550100",
        "ftp://example.test/file",
        "#section",
        "/relative/path",
        "page.html?at=12:30",
        "",
    ] {
        assert!(is_followable(href), "{href:?} was refused");
    }
}
