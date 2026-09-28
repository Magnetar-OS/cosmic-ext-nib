// SPDX-License-Identifier: MPL-2.0

//! The widget under a pointer, driven the way the runtime drives it.
//!
//! Each test lays the editor out and feeds `update` the same events a window
//! would, then reads what the widget published. The renderer is headless: it
//! draws nothing, but its paragraphs are the real shaped ones, so a hit test
//! lands on the glyph it would land on in a window.

use cosmic::iced::advanced::graphics::text::{Editor as TextEditor, Paragraph, Raw};
use cosmic::iced::advanced::widget::Tree;
use cosmic::iced::advanced::{
    Layout, Shell, Widget, clipboard, image, layout, mouse, renderer, text,
};
use cosmic::iced::{
    Background, Color, Event, Font, Pixels, Point, Rectangle, Size, Transformation, keyboard,
};
use nib::{Action, Editor, Style};
use nib_model::basic::{self, marks, nodes};
use nib_model::build::Builder;
use nib_model::node::Node;
use nib_model::state::{EditorState, Selection};
use nib_model::{attrs, nodes};

/// A renderer that shapes text and draws nothing.
struct Headless;

impl renderer::Renderer for Headless {
    fn start_layer(&mut self, _bounds: Rectangle) {}
    fn end_layer(&mut self) {}
    fn start_transformation(&mut self, _transformation: Transformation) {}
    fn end_transformation(&mut self) {}
    fn reset(&mut self, _new_bounds: Rectangle) {}
    fn fill_quad(&mut self, _quad: renderer::Quad, _background: impl Into<Background>) {}
    fn allocate_image(
        &mut self,
        _handle: &image::Handle,
        callback: impl FnOnce(Result<image::Allocation, image::Error>) + Send + 'static,
    ) {
        callback(Err(image::Error::Unsupported));
    }
}

impl text::Renderer for Headless {
    type Font = Font;
    type Paragraph = Paragraph;
    type Editor = TextEditor;
    type Raw = Raw;

    const ICON_FONT: Font = Font::DEFAULT;
    const CHECKMARK_ICON: char = '0';
    const ARROW_DOWN_ICON: char = '0';
    const SCROLL_UP_ICON: char = '0';
    const SCROLL_DOWN_ICON: char = '0';
    const SCROLL_LEFT_ICON: char = '0';
    const SCROLL_RIGHT_ICON: char = '0';
    const ICED_LOGO: char = '0';

    fn default_font(&self) -> Font {
        Font::DEFAULT
    }
    fn default_size(&self) -> Pixels {
        Pixels(14.0)
    }
    fn fill_paragraph(&mut self, _: &Paragraph, _: Point, _: Color, _: Rectangle) {}
    fn fill_editor(&mut self, _: &TextEditor, _: Point, _: Color, _: Rectangle) {}
    fn fill_raw(&mut self, _: Raw) {}
    fn fill_text(&mut self, _: text::Text, _: Point, _: Color, _: Rectangle) {}
}

type Widgets<'a> = dyn Widget<Action, cosmic::Theme, Headless> + 'a;

/// One editor, laid out at a fixed width, and the events fed to it.
struct Window<'a> {
    editor: Editor<'a, Action>,
    tree: Tree,
    node: layout::Node,
}

impl<'a> Window<'a> {
    fn new(editor: Editor<'a, Action>) -> Self {
        let mut editor = editor;
        let mut tree = Tree::new(&editor as &Widgets<'_>);
        let limits = layout::Limits::new(Size::ZERO, Size::new(400.0, 1_000.0));
        let node = Widget::<Action, cosmic::Theme, Headless>::layout(
            &mut editor,
            &mut tree,
            &Headless,
            &limits,
        );
        Self { editor, tree, node }
    }

    /// Delivers one event with the pointer at `at`, and returns what the
    /// widget published.
    fn send(&mut self, event: &Event, at: Point) -> Vec<Action> {
        let mut published = Vec::new();
        let mut shell = Shell::new(&mut published);
        let bounds = self.node.bounds();
        Widget::<Action, cosmic::Theme, Headless>::update(
            &mut self.editor,
            &mut self.tree,
            event,
            Layout::new(&self.node),
            mouse::Cursor::Available(at),
            &Headless,
            &mut clipboard::Null,
            &mut shell,
            &bounds,
        );
        published
    }

    fn click(&mut self, at: Point) -> Vec<Action> {
        let mut out = self.send(&left(true), at);
        out.extend(self.send(&left(false), at));
        out
    }

    fn pointer_at(&self, at: Point) -> mouse::Interaction {
        Widget::<Action, cosmic::Theme, Headless>::mouse_interaction(
            &self.editor,
            &self.tree,
            Layout::new(&self.node),
            mouse::Cursor::Available(at),
            &self.node.bounds(),
            &Headless,
        )
    }
}

fn left(pressed: bool) -> Event {
    let button = mouse::Button::Left;
    Event::Mouse(if pressed {
        mouse::Event::ButtonPressed(button)
    } else {
        mouse::Event::ButtonReleased(button)
    })
}

fn modifiers(held: keyboard::Modifiers) -> Event {
    Event::Keyboard(keyboard::Event::ModifiersChanged(held))
}

fn links(actions: &[Action]) -> Vec<String> {
    actions
        .iter()
        .filter_map(|action| match action {
            Action::Link(href) => Some(href.clone()),
            _ => None,
        })
        .collect()
}

fn style() -> Style {
    Style::from_theme(&cosmic::Theme::default())
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
    assert!(links(&window.click(on_plain())).is_empty());
    // Past the end of the line is not on the link either.
    let s = style();
    let beyond = Point::new(395.0, s.padding + s.text_size * 0.6);
    assert!(links(&window.click(beyond)).is_empty());
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
    assert!(links(&published).is_empty());
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
    assert!(links(&published).is_empty());
}

#[test]
fn in_an_editor_a_plain_click_places_the_caret_and_a_command_click_follows() {
    // A click in text being written is for putting the caret there; the
    // link is one modifier away.
    let state = state("https://example.test/");
    let mut window = Window::new(nib::editor(&state).style(style()).on_action(|a| a));

    let plain = window.click(on_link());
    assert!(links(&plain).is_empty());
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
    assert!(links(&window.click(on_link())).is_empty());
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
    assert!(links(&window.click(on_link())).is_empty());
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
