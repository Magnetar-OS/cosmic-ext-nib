// SPDX-License-Identifier: MPL-2.0

//! Copy and paste through the system clipboard, with its HTML flavour.

mod common;

use std::borrow::Cow;

use common::{Window, command, style};
use cosmic::iced::Point;
use cosmic::iced::advanced::Clipboard;
use cosmic::iced::advanced::clipboard::Kind;
use cosmic::iced::clipboard::mime::{AsMimeTypes, ClipboardStoreData};
use nib::Action;
use nib_model::basic::{self, marks, nodes};
use nib_model::build::Builder;
use nib_model::node::Node;
use nib_model::nodes;
use nib_model::state::{EditorState, Selection};

type Offer = Box<dyn AsMimeTypes + Send + Sync>;

/// A system clipboard: what this application offered, or what another one
/// left there.
#[derive(Default)]
struct Board {
    offered: Option<Offer>,
    text: Option<String>,
    html: Option<Vec<u8>>,
}

impl Board {
    fn from_elsewhere(text: &str, html: Vec<u8>) -> Self {
        Self {
            offered: None,
            text: Some(text.to_owned()),
            html: Some(html),
        }
    }

    fn offered_as(&self, mime: &str) -> Option<String> {
        let bytes = self.offered.as_ref()?.as_bytes(mime)?;
        Some(String::from_utf8(bytes.into_owned()).expect("UTF-8"))
    }
}

impl Clipboard for Board {
    fn read(&self, _kind: Kind) -> Option<String> {
        self.offered_as("text/plain").or_else(|| self.text.clone())
    }

    fn write(&mut self, _kind: Kind, contents: String) {
        *self = Self {
            text: Some(contents),
            ..Self::default()
        };
    }

    fn read_data(&self, _kind: Kind, mimes: Vec<String>) -> Option<(Vec<u8>, String)> {
        mimes.into_iter().find_map(|mime| {
            let bytes = match &self.offered {
                Some(offer) => offer.as_bytes(&mime).map(Cow::into_owned),
                None => self.html.clone().filter(|_| mime == "text/html"),
            }?;
            Some((bytes, mime))
        })
    }

    fn write_data(&mut self, _kind: Kind, contents: ClipboardStoreData<Offer>) {
        *self = Self {
            offered: Some(contents.0),
            ..Self::default()
        };
    }
}

fn b() -> Builder {
    Builder::new(basic::schema())
}

fn state(doc: Node, selection: (usize, usize)) -> EditorState {
    let selection = Selection::between(&doc, selection.0, selection.1);
    EditorState::with_selection(basic::schema(), doc, selection, Vec::new())
}

/// `plain bold`, with `bold` strong.
fn plain_bold() -> Node {
    let b = b();
    let mut content = nodes![b.text("plain ")];
    content.extend(b.mark(marks::STRONG, None, nodes![b.text("bold")]));
    b.doc(nodes![b.node(nodes::PARAGRAPH, content)])
}

fn empty() -> Node {
    let b = b();
    b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![])])
}

/// The document the widget's edits in `actions` leave `state` holding.
fn after(state: &EditorState, actions: Vec<Action>) -> Node {
    let mut state = state.clone();
    for action in actions {
        if let Action::Edit(tr) = action {
            state = state.applied(*tr);
        }
    }
    state.doc().clone()
}

fn press(state: &EditorState, key: &str, board: &mut Board) -> Vec<Action> {
    let mut window = Window::new(
        nib::editor(state)
            .style(style())
            .autofocus()
            .on_action(|a| a),
    );
    window.send_with(&command(key), Point::ORIGIN, board)
}

#[test]
fn a_copy_offers_html_as_well_as_text() {
    let state = state(plain_bold(), (1, 11));
    let mut board = Board::default();
    press(&state, "c", &mut board);
    assert_eq!(
        board.offered_as("text/plain").as_deref(),
        Some("plain bold")
    );
    assert_eq!(
        board.offered_as("UTF8_STRING").as_deref(),
        Some("plain bold")
    );
    let html = board.offered_as("text/html").expect("an HTML flavour");
    assert!(html.contains("plain <strong>bold</strong>"), "{html}");
}

#[test]
fn a_paste_of_another_applications_html_keeps_its_structure() {
    let state = state(empty(), (1, 1));
    let mut board = Board::from_elsewhere(
        "one\ntwo",
        b"<ul><li>one</li><li><b>two</b></li></ul>".to_vec(),
    );
    let doc = after(&state, press(&state, "v", &mut board));
    assert_eq!(
        doc.to_string(),
        r#"doc(bullet_list(list_item(paragraph("one")), list_item(paragraph(strong("two")))))"#
    );
}

#[test]
fn html_written_as_utf16_is_read_too() {
    // Some browsers put their HTML on the clipboard as UTF-16 behind a
    // byte-order mark.
    let state = state(empty(), (1, 1));
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend("<em>x</em>".encode_utf16().flat_map(u16::to_le_bytes));
    let mut board = Board::from_elsewhere("x", bytes);
    let doc = after(&state, press(&state, "v", &mut board));
    assert_eq!(doc.to_string(), r#"doc(paragraph(em("x")))"#);
}

#[test]
fn pasted_html_goes_through_the_same_allow_list_as_mail() {
    let state = state(empty(), (1, 1));
    let mut board = Board::from_elsewhere(
        "x",
        br#"<p><a href="javascript:alert(1)">x</a><script>alert(2)</script></p>"#.to_vec(),
    );
    let doc = after(&state, press(&state, "v", &mut board));
    assert_eq!(doc.to_string(), r#"doc(paragraph("x"))"#);
}

#[test]
fn a_paste_with_no_html_reads_the_text() {
    let state = state(empty(), (1, 1));
    let mut board = Board::default();
    board.write(Kind::Standard, "one\n\ntwo".to_owned());
    let doc = after(&state, press(&state, "v", &mut board));
    assert_eq!(
        doc.to_string(),
        r#"doc(paragraph("one"), paragraph("two"))"#
    );
}

#[test]
fn a_copy_pasted_into_another_editor_keeps_its_marks() {
    // Another window, or another application: neither has the structured
    // slice the copying widget kept, so the HTML is what carries the marks.
    let copied = state(plain_bold(), (1, 11));
    let mut board = Board::default();
    let mut window = Window::new(
        nib::editor(&copied)
            .style(style())
            .autofocus()
            .on_action(|a| a),
    );
    window.send_with(&command("c"), Point::ORIGIN, &mut board);
    let target = state(empty(), (1, 1));
    let published = {
        let mut window = Window::new(
            nib::editor(&target)
                .style(style())
                .autofocus()
                .on_action(|a| a),
        );
        window.send_with(&command("v"), Point::ORIGIN, &mut board)
    };
    assert_eq!(
        after(&target, published).to_string(),
        r#"doc(paragraph("plain ", strong("bold")))"#
    );
}
