// SPDX-License-Identifier: MPL-2.0

//! The widget under a pointer: which clicks follow a link, and which select.

mod common;

use common::{Window, left, modifiers, style};
use cosmic::iced::advanced::mouse;
use cosmic::iced::{Event, Point, keyboard};
use nib::Action;
use nib_model::basic::{self, marks, nodes};
use nib_model::build::Builder;
use nib_model::node::Node;
use nib_model::state::{EditorState, Selection};
use nib_model::{attrs, nodes};

/// What [`links`] returns when nothing was followed, typed so a failing
/// assertion can print the link that was.
const NO_LINKS: [&str; 0] = [];

fn links(actions: &[Action]) -> Vec<String> {
    actions
        .iter()
        .filter_map(|action| match action {
            Action::Link(href) => Some(href.clone()),
            _ => None,
        })
        .collect()
}

/// One paragraph: `go` linked to `href`, then plain text long enough to
/// click well clear of the link.
fn state(href: &str) -> EditorState {
    let b = Builder::new(basic::schema());
    let mut content: Vec<Node> = b.mark(
        marks::LINK,
        Some(&attrs! { "href" => href }),
        nodes![b.text("go")],
    );
    content.push(b.text(" and then a good deal of plain text after it"));
    let doc = b.doc(nodes![b.node(nodes::PARAGRAPH, content)]);
    EditorState::with_selection(basic::schema(), doc, Selection::cursor(1), Vec::new())
}

/// On the first letter of the link.
fn on_link() -> Point {
    let s = style();
    Point::new(s.padding + 2.0, s.padding + s.text_size * 0.6)
}

/// On the plain text after it.
fn on_plain() -> Point {
    let s = style();
    Point::new(s.padding + 150.0, s.padding + s.text_size * 0.6)
}

#[test]
fn a_click_on_a_link_in_a_reader_reports_its_target() {
    let state = state("https://example.test/");
    let mut window = Window::new(
        nib::editor(&state)
            .style(style())
            .read_only()
            .on_action(|a| a),
    );
    assert_eq!(links(&window.click(on_link())), ["https://example.test/"]);
}

#[test]
fn a_click_beside_a_link_reports_nothing() {
    let state = state("https://example.test/");
    let mut window = Window::new(
        nib::editor(&state)
            .style(style())
            .read_only()
            .on_action(|a| a),
    );
    assert_eq!(links(&window.click(on_plain())), NO_LINKS);
    // Past the end of the line is not on the link either.
    let s = style();
    let beyond = Point::new(395.0, s.padding + s.text_size * 0.6);
    assert_eq!(links(&window.click(beyond)), NO_LINKS);
}

#[test]
fn a_drag_that_starts_on_a_link_selects_rather_than_follows() {
    // Selecting a link's text to copy it is the other thing a reader does
    // with one.
    let state = state("https://example.test/");
    let mut window = Window::new(
        nib::editor(&state)
            .style(style())
            .read_only()
            .on_action(|a| a),
    );
    let mut published = window.send(&left(true), on_link());
    published.extend(window.send(
        &Event::Mouse(mouse::Event::CursorMoved {
            position: on_plain(),
        }),
        on_plain(),
    ));
    published.extend(window.send(&left(false), on_link()));
    assert_eq!(links(&published), NO_LINKS);
}

#[test]
fn a_press_on_a_link_released_elsewhere_follows_nothing() {
    let state = state("https://example.test/");
    let mut window = Window::new(
        nib::editor(&state)
            .style(style())
            .read_only()
            .on_action(|a| a),
    );
    let mut published = window.send(&left(true), on_link());
    published.extend(window.send(&left(false), on_plain()));
    assert_eq!(links(&published), NO_LINKS);
}

#[test]
fn in_an_editor_a_plain_click_places_the_caret_and_a_command_click_follows() {
    // A click in text being written is for putting the caret there; the
    // link is one modifier away.
    let state = state("https://example.test/");
    let mut window = Window::new(nib::editor(&state).style(style()).on_action(|a| a));

    let plain = window.click(on_link());
    assert_eq!(links(&plain), NO_LINKS);
    assert!(
        plain.iter().any(|a| matches!(a, Action::Edit(_))),
        "the caret moved"
    );

    window.send(&modifiers(keyboard::Modifiers::COMMAND), on_link());
    let followed = window.click(on_link());
    assert_eq!(links(&followed), ["https://example.test/"]);
    assert!(
        !followed.iter().any(|a| matches!(a, Action::Edit(_))),
        "following a link leaves the caret where it was"
    );

    window.send(&modifiers(keyboard::Modifiers::empty()), on_link());
    assert_eq!(links(&window.click(on_link())), NO_LINKS);
}

#[test]
fn a_target_the_policy_refuses_is_never_reported() {
    // A document built by an application rather than parsed can still carry
    // one, and the widget is the last place to stop it.
    let state = state("javascript:alert(1)");
    let mut window = Window::new(
        nib::editor(&state)
            .style(style())
            .read_only()
            .on_action(|a| a),
    );
    assert_eq!(links(&window.click(on_link())), NO_LINKS);
    assert_eq!(window.pointer_at(on_link()), mouse::Interaction::Text);
}

#[test]
fn the_pointer_says_where_a_click_would_follow_a_link() {
    let state = state("https://example.test/");
    let reader = Window::new(
        nib::editor(&state)
            .style(style())
            .read_only()
            .on_action(|a| a),
    );
    assert_eq!(reader.pointer_at(on_link()), mouse::Interaction::Pointer);
    assert_eq!(reader.pointer_at(on_plain()), mouse::Interaction::Text);

    let mut editor = Window::new(nib::editor(&state).style(style()).on_action(|a| a));
    assert_eq!(editor.pointer_at(on_link()), mouse::Interaction::Text);
    editor.send(&modifiers(keyboard::Modifiers::COMMAND), on_link());
    assert_eq!(editor.pointer_at(on_link()), mouse::Interaction::Pointer);
}
