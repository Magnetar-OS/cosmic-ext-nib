// SPDX-License-Identifier: MPL-2.0

//! The widget driven the way the runtime drives it, without a window.
//!
//! A test lays the editor out and feeds `update` the same events a window
//! would, then reads what the widget published. The renderer is headless: it
//! draws nothing, but its paragraphs are the real shaped ones, so a hit test
//! lands on the glyph it would land on in a window.

#![allow(dead_code, reason = "each test file uses its own part of the harness")]

use cosmic::iced::advanced::graphics::text::{Editor as TextEditor, Paragraph, Raw};
use cosmic::iced::advanced::widget::Tree;
use cosmic::iced::advanced::{
    Layout, Shell, Widget, clipboard, image, layout, mouse, renderer, text,
};
use cosmic::iced::{
    Background, Color, Event, Font, Pixels, Point, Rectangle, Size, Transformation, keyboard,
};
use nib::{Action, Editor, Style};

/// A renderer that shapes text and draws nothing.
pub struct Headless;

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
pub struct Window<'a> {
    editor: Editor<'a, Action>,
    tree: Tree,
    node: layout::Node,
}

impl<'a> Window<'a> {
    pub fn new(editor: Editor<'a, Action>) -> Self {
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
    pub fn send(&mut self, event: &Event, at: Point) -> Vec<Action> {
        self.send_with(event, at, &mut clipboard::Null)
    }

    /// Delivers one event with the pointer at `at` and a clipboard to hand.
    pub fn send_with(
        &mut self,
        event: &Event,
        at: Point,
        board: &mut dyn cosmic::iced::advanced::Clipboard,
    ) -> Vec<Action> {
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
            board,
            &mut shell,
            &bounds,
        );
        published
    }

    pub fn click(&mut self, at: Point) -> Vec<Action> {
        let mut out = self.send(&left(true), at);
        out.extend(self.send(&left(false), at));
        out
    }

    pub fn pointer_at(&self, at: Point) -> mouse::Interaction {
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

pub fn left(pressed: bool) -> Event {
    let button = mouse::Button::Left;
    Event::Mouse(if pressed {
        mouse::Event::ButtonPressed(button)
    } else {
        mouse::Event::ButtonReleased(button)
    })
}

pub fn modifiers(held: keyboard::Modifiers) -> Event {
    Event::Keyboard(keyboard::Event::ModifiersChanged(held))
}

/// A key pressed with the command modifier held, as a shortcut arrives.
pub fn command(key: &str) -> Event {
    Event::Keyboard(keyboard::Event::KeyPressed {
        key: keyboard::Key::Character(key.into()),
        modified_key: keyboard::Key::Character(key.into()),
        physical_key: keyboard::key::Physical::Unidentified(
            keyboard::key::NativeCode::Unidentified,
        ),
        location: keyboard::Location::Standard,
        modifiers: keyboard::Modifiers::COMMAND,
        text: None,
        repeat: false,
    })
}

pub fn style() -> Style {
    Style::from_theme(&cosmic::Theme::default())
}
