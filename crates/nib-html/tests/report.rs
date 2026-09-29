// SPDX-License-Identifier: MPL-2.0

//! What a message's styling tried that the document does not do.

use nib_css::{Hiding, Refusal};
use nib_html::{Html, Report};
use nib_model::basic;

fn html() -> Html {
    Html::new(&basic::schema())
}

#[test]
fn every_attempt_to_hide_text_is_reported_and_the_text_is_shown_anyway() {
    let source = r#"<p style="display: none">a</p>
        <p><span style="opacity: 0">b</span><span style="font-size: 0 !important">c</span></p>"#;
    let (doc, report) = html().parse_with_report(source);
    assert_eq!(doc.text_content(), "abc");
    assert_eq!(
        report.hiding,
        [Hiding::Removed, Hiding::Transparent, Hiding::Shrunk]
    );
    assert!(report.tried_to_hide());
}

#[test]
fn a_fetch_or_a_position_is_reported_as_refused() {
    let (_, report) = html().parse_with_report(
        r#"<div style="position: absolute; background: url(https://tracker.test/p.gif)">x</div>"#,
    );
    assert_eq!(
        report.refused,
        [
            Refusal::Positions {
                property: "position".into()
            },
            Refusal::Fetches {
                property: "background".into()
            },
        ]
    );
    assert!(!report.tried_to_hide());
}

#[test]
fn an_ordinary_message_reports_nothing_and_parses_the_same_either_way() {
    let source = r#"<p style="color: #c00">Hello <b>there</b></p>"#;
    let (doc, report) = html().parse_with_report(source);
    assert_eq!(report, Report::default());
    assert_eq!(doc, html().parse(source));
}
