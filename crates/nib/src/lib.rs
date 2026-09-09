// SPDX-License-Identifier: MPL-2.0

//! A rich text editor for libcosmic.
//!
//! # One widget, not a tree of them
//!
//! The document is flattened into a list of boxes and drawn by a single
//! widget. The obvious alternative — a widget per block, nested the way the
//! document is — founders on the selection: a selection runs from a position
//! in one block to a position in another, and in a tree of widgets that is a
//! conversation between widgets that cannot see each other. Flattened, it is
//! arithmetic.
//!
//! # What is borrowed from iced, and what is not
//!
//! Text layout, shaping, hit testing and caret geometry come from iced's
//! [`Paragraph`](cosmic::iced::advanced::text::Paragraph), which is
//! `cosmic-text` underneath, so the editor uses the
//! same shaper as the rest of the desktop, gets bidirectional and complex
//! scripts for free, and never disagrees with a neighbouring label about how
//! wide a word is.
//!
//! What is not borrowed is the editing: iced's `text_editor` is a *code* editor whose
//! highlighter can set a colour and a font and nothing else. Everything above
//! that — block structure, per-span sizes, selection across blocks, marks,
//! decorations — is here.
//!
//! # Using it
//!
//! ```no_run
//! # use nib_model::{EditorState, basic};
//! # use nib::{editor, Action};
//! # #[derive(Clone)] enum Message { Edit(Action) }
//! # fn view(state: &EditorState) -> cosmic::Element<'_, Message> {
//! editor(state).on_action(Message::Edit).into()
//! # }
//! ```

pub mod blocks;
pub mod caret;
pub mod style;

pub use blocks::Block;
pub use style::{Caret, Colors, Style};

use std::time::Instant;

use cosmic::iced::advanced::text::Renderer as TextRenderer;
use cosmic::iced::advanced::widget::{Id, Operation, Tree, operation, tree};
use cosmic::iced::advanced::{Clipboard, Layout, Shell, Widget, layout, mouse, renderer};
use cosmic::iced::{
    Background, Border, Color, Element, Event, Length, Point, Rectangle, Size, Vector, keyboard,
    window,
};
use cosmic::iced::advanced::text::{Affinity, LineHeight, Text, Wrapping};
use nib_model::commands;
use nib_model::decoration::DecorationSet;
use nib_model::input_rules::InputRule;
use nib_model::keymap::{Binding, Key, Keymap, Mods};
use nib_model::node::Node;
use nib_model::slice::Slice;
use nib_model::state::{EditorState, Selection, Transaction};

/// What the editor asks its application to do.
///
/// The widget never changes the state itself: it hands back a transaction and
/// the application decides. That is not ceremony — it is what lets the
/// application veto an edit, log it, send it to a collaborator, or apply it to
/// a state the widget has never seen.
#[derive(Debug, Clone)]
pub enum Action {
    /// Apply this to the editor state.
    Edit(Box<Transaction>),
    /// The editor took keyboard focus.
    Focused,
    /// The editor lost it.
    Blurred,
    /// A link was activated, with its target.
    Link(String),
}

/// Builds the editor widget.
#[must_use]
pub fn editor(state: &EditorState) -> Editor<'_, ()> {
    Editor::new(state)
}

/// The editor widget.
pub struct Editor<'a, Message> {
    state: &'a EditorState,
    decorations: Option<&'a DecorationSet>,
    keymap: Option<&'a Keymap>,
    input_rules: Option<&'a [InputRule]>,
    style: Option<Style>,
    on_action: Option<Box<dyn Fn(Action) -> Message + 'a>>,
    id: Option<Id>,
    autofocus: bool,
    width: Length,
    height: Length,
}

impl<'a> Editor<'a, ()> {
    /// An editor over a state, with no handler yet.
    #[must_use]
    pub fn new(state: &'a EditorState) -> Self {
        Self {
            state,
            decorations: None,
            keymap: None,
            input_rules: None,
            style: None,
            on_action: None,
            id: None,
            autofocus: false,
            width: Length::Fill,
            height: Length::Shrink,
        }
    }
}

impl<'a, Message> Editor<'a, Message> {
    /// Sets what the editor does with the changes it produces.
    #[must_use]
    pub fn on_action<M>(self, f: impl Fn(Action) -> M + 'a) -> Editor<'a, M> {
        Editor {
            state: self.state,
            decorations: self.decorations,
            keymap: self.keymap,
            input_rules: self.input_rules,
            style: self.style,
            on_action: Some(Box::new(f)),
            id: self.id,
            autofocus: self.autofocus,
            width: self.width,
            height: self.height,
        }
    }

    /// Styling that is not in the document: syntax colours, search hits, a
    /// collaborator's caret.
    #[must_use]
    pub fn decorations(mut self, decorations: &'a DecorationSet) -> Self {
        self.decorations = Some(decorations);
        self
    }

    /// The keys. Without one, [`Keymap::base`] is built for the state's
    /// schema on every event, which is fine for a demo and wasteful for an
    /// application — hold one and pass it.
    #[must_use]
    pub fn keymap(mut self, keymap: &'a Keymap) -> Self {
        self.keymap = Some(keymap);
        self
    }

    /// The rules that fire while typing.
    #[must_use]
    pub fn input_rules(mut self, rules: &'a [InputRule]) -> Self {
        self.input_rules = Some(rules);
        self
    }

    /// Names the editor, so focus can be moved to it — with Tab, or when a
    /// window opens on a document the user is expected to start typing in.
    #[must_use]
    pub fn id(mut self, id: Id) -> Self {
        self.id = Some(id);
        self
    }

    /// Takes keyboard focus the first time it is laid out.
    ///
    /// For a window that opens on a document the user is expected to start
    /// typing in. Off by default, because a page with an editor *and* a search
    /// box has to decide which one, and the widget is not the one that knows.
    #[must_use]
    pub fn autofocus(mut self) -> Self {
        self.autofocus = true;
        self
    }

    /// How it looks. Without one, [`Style::from_theme`] follows the desktop.
    #[must_use]
    pub fn style(mut self, style: Style) -> Self {
        self.style = Some(style);
        self
    }

    #[must_use]
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    #[must_use]
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }
}

/// What the widget keeps between frames.
struct Internal<P> {
    /// One per block, index-aligned with `blocks`.
    paragraphs: Vec<P>,
    blocks: Vec<Block>,
    /// Where each block was laid out, relative to the widget's origin.
    bounds: Vec<Rectangle>,
    /// Whether the first layout has happened, so autofocus fires once.
    laid_out: bool,
    /// The document these were built from, so an unchanged one is not rebuilt.
    /// Comparing two `Node`s is a handful of pointer comparisons.
    built_from: Option<Node>,
    built_width: f32,
    caret: caret::Animation,
    focused: bool,
    /// The document position a drag started at.
    drag_anchor: Option<usize>,
    /// The x the caret should try to keep when moving between lines.
    goal_x: Option<f32>,
    /// The slice most recently copied, so a paste inside the application keeps
    /// its structure. The system clipboard carries the plain text.
    clipboard: Option<Slice>,
    /// The state as of the last transaction this widget emitted.
    ///
    /// A frame can carry several key events, and the application does not see
    /// any of them until the frame is over — so without this, every keystroke
    /// in a burst would be built against the same stale document and the
    /// positions of all but the first would be wrong. Cleared as soon as the
    /// application hands back a state that differs from the one this was
    /// derived from.
    working: Option<EditorState>,
    /// What `self.state` held when `working` was last reconciled.
    seen: Option<(Node, Selection)>,
    now: Instant,
}

impl<P> Default for Internal<P> {
    fn default() -> Self {
        Self {
            paragraphs: Vec::new(),
            blocks: Vec::new(),
            bounds: Vec::new(),
            laid_out: false,
            built_from: None,
            built_width: 0.0,
            caret: caret::Animation::new(Instant::now()),
            focused: false,
            drag_anchor: None,
            goal_x: None,
            clipboard: None,
            working: None,
            seen: None,
            now: Instant::now(),
        }
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for Editor<'_, Message>
where
    Renderer: TextRenderer<Font = cosmic::iced::Font> + cosmic::iced::advanced::Renderer,
    Theme: 'static,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Internal<Renderer::Paragraph>>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(Internal::<Renderer::Paragraph>::default())
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let internal = tree.state.downcast_mut::<Internal<Renderer::Paragraph>>();
        let style = self.resolved_style_without_theme();
        let width = limits.max().width;

        self.rebuild(internal, &style, width);
        if self.autofocus && !internal.laid_out {
            internal.laid_out = true;
            internal.focused = true;
            internal.caret.touch(internal.now);
        }
        let height = internal
            .bounds
            .last()
            .map_or(style.padding * 2.0, |last| last.y + last.height + style.padding);

        layout::Node::new(limits.resolve(
            self.width,
            self.height,
            Size::new(width, height),
        ))
    }

    #[allow(clippy::too_many_lines)]
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _renderer_style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let internal = tree.state.downcast_ref::<Internal<Renderer::Paragraph>>();
        let style = self.resolved_style_without_theme();
        let origin = layout.bounds().position();
        let selection = self.state.selection();
        let (from, to) = (selection.from(), selection.to());

        for (index, block) in internal.blocks.iter().enumerate() {
            let Some(bounds) = internal.bounds.get(index) else {
                continue;
            };
            let bounds = *bounds + Vector::new(origin.x, origin.y);
            if !bounds.intersects(viewport) {
                continue;
            }

            // A code block's ground, drawn behind everything else in it.
            if style::has_background(block) {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: Rectangle {
                            x: bounds.x - style.text_size * 0.5,
                            width: bounds.width + style.text_size,
                            y: bounds.y - style.text_size * 0.25,
                            height: bounds.height + style.text_size * 0.5,
                        },
                        border: Border {
                            radius: 4.0.into(),
                            ..Border::default()
                        },
                        ..renderer::Quad::default()
                    },
                    Background::Color(style.colors.code_background),
                );
            }

            // One bar per level of quotation, so a quote inside a quote reads
            // as two rather than as a thicker one.
            for depth in 0..block.quote_depth {
                let step = style.quote_bar_width + style.text_size * 0.75;
                #[allow(clippy::cast_precision_loss)]
                let offset = step * depth as f32;
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: Rectangle {
                            x: origin.x + style.padding + offset,
                            y: bounds.y,
                            width: style.quote_bar_width,
                            height: bounds.height,
                        },
                        border: Border {
                            radius: (style.quote_bar_width / 2.0).into(),
                            ..Border::default()
                        },
                        ..renderer::Quad::default()
                    },
                    Background::Color(style.colors.quote_bar),
                );
            }

            if let blocks::Kind::Rule = block.kind {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: Rectangle {
                            x: bounds.x,
                            y: bounds.y + bounds.height / 2.0,
                            width: bounds.width,
                            height: 1.0,
                        },
                        ..renderer::Quad::default()
                    },
                    Background::Color(style.colors.muted),
                );
            }

            if let Some(marker) = &block.marker {
                renderer.fill_text(
                    Text {
                        content: style::marker_text(marker),
                        bounds: Size::new(style.text_size * 2.0, bounds.height),
                        size: style.text_size.into(),
                        line_height: LineHeight::Absolute(style.line_height_of(block).into()),
                        font: style.body_font,
                        align_x: cosmic::iced::advanced::text::Alignment::Right,
                        align_y: cosmic::iced::alignment::Vertical::Top,
                        shaping: cosmic::iced::advanced::text::Shaping::Advanced,
                        wrapping: Wrapping::None,
                        ellipsize: cosmic::iced::advanced::text::Ellipsize::None,
                    },
                    Point::new(bounds.x - style.text_size * 0.6, bounds.y),
                    style.colors.muted,
                    *viewport,
                );
            }

            let Some(paragraph) = internal.paragraphs.get(index) else {
                continue;
            };

            // The selection goes behind the text, never over it.
            if to > from && block.from < to && block.to > from {
                for rect in selection_rects(block, paragraph, from.max(block.from), to.min(block.to))
                {
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: rect + Vector::new(bounds.x, bounds.y),
                            ..renderer::Quad::default()
                        },
                        Background::Color(style.colors.selection),
                    );
                }
            }

            renderer.fill_paragraph(
                paragraph,
                bounds.position(),
                style.colors.text,
                *viewport,
            );
        }

        // The caret last, over everything.
        if internal.focused
            && let Some(rect) = internal.caret.visible(internal.now, style.blink_period)
        {
            let color = if style.caret == Caret::Block {
                Color {
                    a: style.colors.caret.a * caret::BLOCK_ALPHA,
                    ..style.colors.caret
                }
            } else {
                style.colors.caret
            };
            renderer.fill_quad(
                renderer::Quad {
                    bounds: rect + Vector::new(origin.x, origin.y),
                    border: Border {
                        radius: (rect.width / 2.0).min(2.0).into(),
                        ..Border::default()
                    },
                    ..renderer::Quad::default()
                },
                Background::Color(color),
            );
        }
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        _renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        let internal = tree.state.downcast_mut::<Internal<Renderer::Paragraph>>();
        operation.focusable(self.id.as_ref(), layout.bounds(), internal);
    }

    fn mouse_interaction(
        &self,
        _tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        if cursor.is_over(layout.bounds()) {
            mouse::Interaction::Text
        } else {
            mouse::Interaction::None
        }
    }

    #[allow(clippy::too_many_lines)]
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let internal = tree.state.downcast_mut::<Internal<Renderer::Paragraph>>();
        self.reconcile(internal);
        let style = self.resolved_style_without_theme();
        let bounds = layout.bounds();
        match event {
            Event::Window(window::Event::RedrawRequested(now)) => {
                internal.now = *now;
                self.aim_caret(internal, &style);
                if internal.caret.tick(*now, style.caret_glide) {
                    // Mid-glide: the next frame as soon as there is one.
                    shell.request_redraw();
                } else if internal.focused
                    && let Some(at) = internal.caret.next_blink(*now, style.blink_period)
                {
                    // Otherwise wake only when the caret changes brightness —
                    // twice a second, not sixty times.
                    shell.request_redraw_at(at);
                }
            }

            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(point) = cursor.position_over(bounds) else {
                    if internal.focused {
                        internal.focused = false;
                        self.emit(shell, Action::Blurred);
                    }
                    return;
                };
                let local = point - Vector::new(bounds.x, bounds.y);
                if !internal.focused {
                    internal.focused = true;
                    self.emit(shell, Action::Focused);
                }
                if let Some(pos) = Self::position_at(internal, Point::new(local.x, local.y)) {
                    internal.drag_anchor = Some(pos);
                    internal.goal_x = None;
                    internal.caret.touch(internal.now);
                    self.select(internal, shell, Selection::cursor(pos));
                }
                shell.capture_event();
                shell.request_redraw();
            }

            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let (Some(anchor), Some(point)) =
                    (internal.drag_anchor, cursor.position_over(bounds))
                else {
                    return;
                };
                let local = point - Vector::new(bounds.x, bounds.y);
                if let Some(head) = Self::position_at(internal, Point::new(local.x, local.y))
                    && head != anchor
                {
                    let doc = self.live(internal).doc().clone();
                    self.select(internal, shell, Selection::between(&doc, anchor, head));
                    shell.request_redraw();
                }
            }

            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                internal.drag_anchor = None;
            }

            Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                modifiers,
                text,
                ..
            }) => {
                if !internal.focused {
                    return;
                }
                internal.caret.touch(internal.now);
                if self.handle_key(internal, key, *modifiers, text.as_deref(), clipboard, shell) {
                    shell.capture_event();
                    shell.request_redraw();
                }
            }

            _ => {}
        }
    }
}

impl<P> operation::Focusable for Internal<P> {
    fn is_focused(&self) -> bool {
        self.focused
    }

    fn focus(&mut self) {
        self.focused = true;
        // A caret that arrived dark is a caret nobody finds.
        self.caret.touch(self.now);
    }

    fn unfocus(&mut self) {
        self.focused = false;
        self.drag_anchor = None;
    }
}

impl<Message> Editor<'_, Message> {
    /// The style, without a theme to hand.
    ///
    /// `layout` and `update` do not receive one — only `draw` does — so the
    /// geometry has to be computable without it. That is why colours are the
    /// last thing [`Style`] resolves and the only thing that needs a theme.
    fn resolved_style_without_theme(&self) -> Style {
        self.style
            .clone()
            .unwrap_or_else(|| Style::from_theme(&cosmic::Theme::default()))
    }

    /// Drops the working state once the application has moved on.
    fn reconcile<P>(&self, internal: &mut Internal<P>) {
        let here = (self.state.doc().clone(), self.state.selection().clone());
        if internal.seen.as_ref() != Some(&here) {
            internal.seen = Some(here);
            internal.working = None;
        }
    }

    /// The state edits are built against: what this widget has already done
    /// this frame, or what the application gave it.
    fn live<'s, P>(&'s self, internal: &'s Internal<P>) -> &'s EditorState {
        internal.working.as_ref().unwrap_or(self.state)
    }

    fn emit(&self, shell: &mut Shell<'_, Message>, action: Action) {
        if let Some(f) = &self.on_action {
            shell.publish(f(action));
        }
    }

    fn apply<P>(
        &self,
        internal: &mut Internal<P>,
        shell: &mut Shell<'_, Message>,
        tr: Transaction,
    ) {
        internal.working = Some(self.live(internal).applied(tr.clone()));
        self.emit(shell, Action::Edit(Box::new(tr)));
    }

    fn select<P>(
        &self,
        internal: &mut Internal<P>,
        shell: &mut Shell<'_, Message>,
        selection: Selection,
    ) {
        let mut tr = self.live(internal).tr();
        tr.set_selection(selection);
        self.apply(internal, shell, tr.clone());
    }

    /// Rebuilds the block list and its paragraphs when the document or the
    /// width has changed.
    fn rebuild<P>(&self, internal: &mut Internal<P>, style: &Style, width: f32)
    where
        P: cosmic::iced::advanced::text::Paragraph<Font = cosmic::iced::Font> + 'static,
    {
        let doc = self.state.doc();
        let unchanged = internal.built_from.as_ref() == Some(doc)
            && (internal.built_width - width).abs() < f32::EPSILON;
        if unchanged {
            return;
        }
        internal.built_from = Some(doc.clone());
        internal.built_width = width;
        internal.blocks = blocks::flatten(doc);
        internal.paragraphs.clear();
        internal.bounds.clear();

        let empty = DecorationSet::empty();
        let decorations = self.decorations.unwrap_or(&empty);
        let mut y = style.padding;

        for block in &internal.blocks {
            let indent = style.indent_of(block);
            let x = style.padding + indent;
            let available = (width - x - style.padding).max(style.text_size);

            let (paragraph, height) = if block.kind == blocks::Kind::Rule {
                (P::default(), style.text_size)
            } else {
                {
                    let spans = style::spans(block, style, decorations);
                    let text = Text {
                        content: spans.as_slice(),
                        bounds: Size::new(available, f32::INFINITY),
                        size: style.size_of(block).into(),
                        line_height: LineHeight::Absolute(style.line_height_of(block).into()),
                        font: style.body_font,
                        align_x: cosmic::iced::advanced::text::Alignment::Default,
                        align_y: cosmic::iced::alignment::Vertical::Top,
                        shaping: cosmic::iced::advanced::text::Shaping::Advanced,
                        wrapping: style::wrapping(block),
                        ellipsize: cosmic::iced::advanced::text::Ellipsize::None,
                    };
                    let paragraph = P::with_spans(text);
                    // An empty block still occupies a line: a paragraph with
                    // nothing in it must be tall enough to put a caret in.
                    let height = paragraph
                        .min_bounds()
                        .height
                        .max(style.line_height_of(block));
                    (paragraph, height)
                }
            };

            internal.bounds.push(Rectangle {
                x,
                y,
                width: available,
                height,
            });
            internal.paragraphs.push(paragraph);
            y += height + style.text_size * style.block_spacing;
        }
    }

    /// The document position under a point in the widget's own coordinates.
    fn position_at<P>(internal: &Internal<P>, point: Point) -> Option<usize>
    where
        P: cosmic::iced::advanced::text::Paragraph<Font = cosmic::iced::Font> + 'static,
    {
        // The block whose band the point falls in, or the nearest one — a
        // click in the gap between blocks belongs to one of them.
        let mut best: Option<(f32, usize)> = None;
        for (index, bounds) in internal.bounds.iter().enumerate() {
            let distance = if point.y < bounds.y {
                bounds.y - point.y
            } else if point.y > bounds.y + bounds.height {
                point.y - (bounds.y + bounds.height)
            } else {
                0.0
            };
            if best.is_none_or(|(d, _)| distance < d) {
                best = Some((distance, index));
            }
            if distance == 0.0 {
                break;
            }
        }
        let (_, index) = best?;
        let block = internal.blocks.get(index)?;
        let bounds = internal.bounds.get(index)?;
        let paragraph = internal.paragraphs.get(index)?;

        if !matches!(block.kind, blocks::Kind::Text) {
            return Some(block.node_pos);
        }
        let local = Point::new(point.x - bounds.x, (point.y - bounds.y).max(0.0));
        let line = line_at(paragraph, block, local.y);
        let offset = match paragraph.hit_test(local) {
            Some(hit) => block.line_start(line) + hit.cursor(),
            None if local.x <= 0.0 => block.line_start(line),
            None => block.line_end(line),
        };
        Some(block.doc_position(offset.min(block.text.len())))
    }

    /// Puts the caret where the selection says it is.
    fn aim_caret<P>(&self, internal: &mut Internal<P>, style: &Style)
    where
        P: cosmic::iced::advanced::text::Paragraph<Font = cosmic::iced::Font> + 'static,
    {
        let target = self
            .state
            .selection()
            .cursor_pos()
            .and_then(|pos| Self::caret_rect(internal, style, pos));
        internal.caret.aim(target, internal.now, style.caret_glide);
    }

    fn caret_rect<P>(
        internal: &Internal<P>,
        style: &Style,
        pos: usize,
    ) -> Option<Rectangle>
    where
        P: cosmic::iced::advanced::text::Paragraph<Font = cosmic::iced::Font> + 'static,
    {
        let index = internal.blocks.iter().position(|b| b.contains(pos))?;
        let block = internal.blocks.get(index)?;
        let bounds = internal.bounds.get(index)?;
        let paragraph = internal.paragraphs.get(index)?;

        let size = style.size_of(block);
        let height = style.line_height_of(block);
        let at = if matches!(block.kind, blocks::Kind::Text) {
            let offset = block.text_offset(pos);
            let (line, within) = block.line_of(offset);
            let point = paragraph
                .cursor_position(line, within, Affinity::After)
                .unwrap_or(Point::ORIGIN);
            Rectangle {
                x: bounds.x + point.x,
                y: bounds.y + point.y,
                width: 0.0,
                height,
            }
        } else {
            // A leaf has no inside: the caret sits on whichever edge the
            // position names.
            let x = if pos <= block.from {
                bounds.x
            } else {
                bounds.x + bounds.width
            };
            Rectangle {
                x,
                y: bounds.y,
                width: 0.0,
                height: bounds.height,
            }
        };
        Some(caret::rectangle(style.caret, at, size, None))
    }

    /// Handles a key press. Returns whether it was ours.
    fn handle_key<P>(
        &self,
        internal: &mut Internal<P>,
        key: &keyboard::Key,
        modifiers: keyboard::Modifiers,
        text: Option<&str>,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) -> bool
    where
        P: cosmic::iced::advanced::text::Paragraph<Font = cosmic::iced::Font> + 'static,
    {
        // Clipboard first: it is not a document edit, and a keymap that bound
        // Ctrl+C would have to know about the clipboard to do it.
        if modifiers.command()
            && !modifiers.alt()
            && let keyboard::Key::Character(c) = key
        {
            match c.as_str() {
                "c" => return self.copy(internal, clipboard),
                "x" => return self.cut(internal, clipboard, shell),
                "v" => return self.paste(internal, clipboard, shell),
                _ => {}
            }
        }
        let state = self.live(internal).clone();

        // Motion is the widget's, because only the widget knows where the
        // lines are.
        if let Some(tr) = self.motion(internal, &state, key, modifiers) {
            self.apply(internal, shell, tr);
            return true;
        }

        let owned;
        let keymap = if let Some(keymap) = self.keymap {
            keymap
        } else {
            owned = Keymap::base(state.schema());
            &owned
        };
        if let Some(binding) = to_binding(key, modifiers)
            && let Some(tr) = keymap.handle(&state, &binding)
        {
            internal.goal_x = None;
            self.apply(internal, shell, tr);
            return true;
        }

        // Ordinary typing, last: a character that no binding claimed.
        let Some(text) = text.filter(|t| !t.is_empty() && !t.chars().any(char::is_control)) else {
            return false;
        };
        if modifiers.command() || modifiers.alt() {
            return false;
        }
        internal.goal_x = None;
        let (from, to) = (state.selection().from(), state.selection().to());
        let rules = self.input_rules.unwrap_or(&[]);
        if let Some(tr) = nib_model::input_rules::apply(&state, rules, from, to, text) {
            self.apply(internal, shell, tr);
            return true;
        }
        let mut tr = state.tr().now();
        if tr.insert_text(text).is_ok() {
            self.apply(internal, shell, tr.clone());
            return true;
        }
        false
    }

    /// Arrow keys, Home and End.
    ///
    /// These live here rather than in the keymap because "up a line" is a
    /// question about layout, and the model has no lines — only positions.
    fn motion<P>(
        &self,
        internal: &mut Internal<P>,
        state: &EditorState,
        key: &keyboard::Key,
        modifiers: keyboard::Modifiers,
    ) -> Option<Transaction>
    where
        P: cosmic::iced::advanced::text::Paragraph<Font = cosmic::iced::Font> + 'static,
    {
        use keyboard::key::Named;

        let keyboard::Key::Named(named) = key else {
            return None;
        };
        let doc = state.doc();
        let selection = state.selection();
        let head = selection.head();

        let target = match named {
            Named::ArrowLeft | Named::ArrowRight => {
                internal.goal_x = None;
                let forward = *named == Named::ArrowRight;
                if modifiers.command() {
                    Self::by_word(doc, head, forward)
                } else {
                    Self::by_grapheme(doc, head, forward)
                }
            }
            Named::ArrowUp | Named::ArrowDown => {
                self.by_line(internal, head, *named == Named::ArrowDown)
            }
            Named::Home => {
                internal.goal_x = None;
                self.line_edge(internal, head, false)
            }
            Named::End => {
                internal.goal_x = None;
                self.line_edge(internal, head, true)
            }
            _ => return None,
        }?;

        let selection = if modifiers.shift() {
            Selection::between(doc, selection.anchor(), target)
        } else {
            Selection::between(doc, target, target)
        };
        let mut tr = state.tr();
        tr.set_selection(selection);
        Some(tr.clone())
    }

    /// One grapheme cluster left or right — not one byte, and not one
    /// character: an emoji with a skin tone is one thing to the user however
    /// many code points it is.
    fn by_grapheme(doc: &Node, from: usize, forward: bool) -> Option<usize> {
        use unicode_segmentation::UnicodeSegmentation;

        let at = doc.resolve(from);
        let text = at.parent().text_content();
        let start = at.start(at.depth());
        let offset = from.saturating_sub(start);

        if forward {
            if offset >= text.len() {
                // Out of this block: the next place a caret can be.
                let next = (from + 1).min(doc.content_size());
                return Selection::find_from(doc, &doc.resolve(next), 1, true).map(|s| s.head());
            }
            let step = text[offset..].graphemes(true).next()?.len();
            Some(from + step)
        } else {
            if offset == 0 {
                let previous = from.saturating_sub(1);
                return Selection::find_from(doc, &doc.resolve(previous), -1, true)
                    .map(|s| s.head());
            }
            let step = text[..offset].graphemes(true).next_back()?.len();
            Some(from - step)
        }
    }

    /// To the next word boundary.
    fn by_word(doc: &Node, from: usize, forward: bool) -> Option<usize> {
        use unicode_segmentation::UnicodeSegmentation;

        let at = doc.resolve(from);
        let text = at.parent().text_content();
        let start = at.start(at.depth());
        let offset = from.saturating_sub(start);

        let boundary = if forward {
            text[offset..]
                .split_word_bound_indices()
                .find(|(i, word)| *i > 0 || !word.trim().is_empty())
                .map(|(i, word)| offset + i + if i == 0 { word.len() } else { 0 })
        } else {
            text[..offset]
                .split_word_bound_indices()
                .rfind(|(_, word)| !word.trim().is_empty())
                .map(|(i, _)| i)
        };
        match boundary {
            Some(offset) => Some(start + offset),
            None => Self::by_grapheme(doc, from, forward),
        }
    }

    /// One visual line up or down, keeping the column.
    fn by_line<P>(
        &self,
        internal: &mut Internal<P>,
        from: usize,
        down: bool,
    ) -> Option<usize>
    where
        P: cosmic::iced::advanced::text::Paragraph<Font = cosmic::iced::Font> + 'static,
    {
        let style = self.resolved_style_without_theme();
        let here = Self::caret_rect(internal, &style, from)?;
        // The column is remembered across a run of up/down presses, so passing
        // through a short line does not drag the caret left permanently.
        let goal = internal.goal_x.unwrap_or(here.x);
        internal.goal_x = Some(goal);

        let index = internal.blocks.iter().position(|b| b.contains(from))?;
        let block = internal.blocks.get(index)?;
        let step = style.line_height_of(block);
        let y = if down {
            here.y + step * 1.5
        } else {
            here.y - step * 0.5
        };
        Self::position_at(internal, Point::new(goal, y))
    }

    /// To the start or end of the visual line.
    fn line_edge<P>(
        &self,
        internal: &Internal<P>,
        from: usize,
        end: bool,
    ) -> Option<usize>
    where
        P: cosmic::iced::advanced::text::Paragraph<Font = cosmic::iced::Font> + 'static,
    {
        let style = self.resolved_style_without_theme();
        let here = Self::caret_rect(internal, &style, from)?;
        let x = if end { f32::MAX / 2.0 } else { 0.0 };
        Self::position_at(internal, Point::new(x, here.y + here.height / 2.0))
    }

    fn copy<P>(&self, internal: &Internal<P>, clipboard: &mut dyn Clipboard) -> bool {
        let state = self.live(internal);
        let selection = state.selection();
        if selection.is_empty() {
            return false;
        }
        let slice = selection.content(state.doc());
        clipboard.write(
            cosmic::iced::advanced::clipboard::Kind::Standard,
            plain_text(state, &slice),
        );
        // The structured slice is kept alongside: the system clipboard carries
        // text, and a paste back into this application should not lose the
        // headings on the way.
        true
    }

    fn cut<P>(
        &self,
        internal: &mut Internal<P>,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) -> bool {
        if !self.copy(internal, clipboard) {
            return false;
        }
        let mut tr = self.live(internal).tr().now();
        if tr.delete_selection().is_ok() {
            self.apply(internal, shell, tr.clone());
        }
        true
    }

    fn paste<P>(
        &self,
        internal: &mut Internal<P>,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) -> bool {
        let state = self.live(internal).clone();
        let text = clipboard.read(cosmic::iced::advanced::clipboard::Kind::Standard);
        let slice = match (&internal.clipboard, text.as_deref()) {
            // The remembered slice is used only when the system clipboard
            // still holds what it was copied from; otherwise something else
            // was copied since.
            (Some(slice), Some(text)) if plain_text(&state, slice) == text => slice.clone(),
            (_, Some(text)) => parse_plain(&state, text),
            (_, None) => return false,
        };
        if slice.is_empty() {
            return false;
        }
        let mut tr = state.tr().now();
        if tr.replace_selection(slice).is_ok() {
            self.apply(internal, shell, tr.clone());
        }
        true
    }
}

/// The plain text of a slice.
fn plain_text(state: &EditorState, slice: &Slice) -> String {
    let doc = state
        .schema()
        .create_and_fill(
            state.schema().top_node_type(),
            None,
            slice.content().clone(),
            nib_model::mark::Marks::none(),
        )
        .unwrap_or_else(|| state.schema().empty_doc());
    doc.text_between(0, doc.content_size(), Some("\n\n"), None)
}

/// Plain text as a slice: one paragraph per blank-line-separated run.
fn parse_plain(state: &EditorState, text: &str) -> Slice {
    let schema = state.schema();
    let Some(paragraph) = schema.node_id(nib_model::basic::nodes::PARAGRAPH) else {
        return Slice::empty();
    };
    let marks = state.marks();
    let blocks: Vec<Node> = text
        .split("\n\n")
        .filter(|run| !run.trim().is_empty())
        .filter_map(|run| {
            schema
                .create(
                    paragraph,
                    None,
                    nib_model::fragment::Fragment::from(schema.text(run.trim_end(), marks.clone())),
                    nib_model::mark::Marks::none(),
                )
                .ok()
        })
        .collect();

    match blocks.len() {
        0 => Slice::empty(),
        // A single run pastes as inline content, so it continues the paragraph
        // it lands in rather than starting one.
        1 => Slice::new(
            blocks[0].content().clone(),
            0,
            0,
        ),
        _ => Slice::new(nib_model::fragment::Fragment::from_vec(blocks), 1, 1),
    }
}

/// The selection rectangles for a range within one block.
fn selection_rects<P>(block: &Block, paragraph: &P, from: usize, to: usize) -> Vec<Rectangle>
where
    P: cosmic::iced::advanced::text::Paragraph,
{
    let (start, end) = (block.text_offset(from), block.text_offset(to));
    let (first_line, first_byte) = block.line_of(start);
    let (last_line, last_byte) = block.line_of(end);
    let mut rects = Vec::new();

    for line in first_line..=last_line {
        let line_start = if line == first_line { first_byte } else { 0 };
        let line_end = if line == last_line {
            last_byte
        } else {
            block.line_length(line)
        };
        rects.extend(paragraph.highlight(
            line,
            (line_start, Affinity::After),
            (line_end, Affinity::Before),
        ));
    }
    rects
}

/// Which buffer line a y falls on.
fn line_at<P>(paragraph: &P, block: &Block, y: f32) -> usize
where
    P: cosmic::iced::advanced::text::Paragraph,
{
    let mut found = 0;
    for line in 0..block.line_count() {
        let Some(point) = paragraph.cursor_position(line, 0, Affinity::After) else {
            break;
        };
        if point.y <= y {
            found = line;
        } else {
            break;
        }
    }
    found
}

/// An iced key press as a binding the keymap understands.
fn to_binding(key: &keyboard::Key, modifiers: keyboard::Modifiers) -> Option<Binding> {
    use keyboard::key::Named;

    let key = match key {
        keyboard::Key::Character(c) => Key::Char(c.chars().next()?.to_ascii_lowercase()),
        keyboard::Key::Named(named) => match named {
            Named::Enter => Key::Enter,
            Named::Tab => Key::Tab,
            Named::Backspace => Key::Backspace,
            Named::Delete => Key::Delete,
            Named::Escape => Key::Escape,
            Named::Home => Key::Home,
            Named::End => Key::End,
            Named::PageUp => Key::PageUp,
            Named::PageDown => Key::PageDown,
            Named::ArrowLeft => Key::ArrowLeft,
            Named::ArrowRight => Key::ArrowRight,
            Named::ArrowUp => Key::ArrowUp,
            Named::ArrowDown => Key::ArrowDown,
            _ => return None,
        },
        keyboard::Key::Unidentified => return None,
    };

    let mut mods = Mods::NONE;
    if modifiers.shift() {
        mods = mods.union(Mods::SHIFT);
    }
    if modifiers.alt() {
        mods = mods.union(Mods::ALT);
    }
    // `command` is Ctrl on Linux and Windows and Cmd on macOS, which is
    // exactly what PRIMARY means.
    if modifiers.command() {
        mods = mods.union(Mods::PRIMARY);
    }
    Some(Binding::new(key, mods))
}

impl<'a, Message, Theme, Renderer> From<Editor<'a, Message>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'static,
    Renderer: TextRenderer<Font = cosmic::iced::Font> + cosmic::iced::advanced::Renderer + 'a,
{
    fn from(editor: Editor<'a, Message>) -> Self {
        Self::new(editor)
    }
}

/// A command bound to a name, for a toolbar button.
///
/// The same commands the keymap runs, so a button and a shortcut cannot drift
/// apart.
#[must_use]
pub fn toolbar_command(state: &EditorState, name: &str) -> Option<Transaction> {
    let schema = state.schema();
    let command = match name {
        "bold" => commands::toggle_mark(schema.mark_id("strong")?, None),
        "italic" => commands::toggle_mark(schema.mark_id("em")?, None),
        "underline" => commands::toggle_mark(schema.mark_id("underline")?, None),
        "strikethrough" => commands::toggle_mark(schema.mark_id("strikethrough")?, None),
        "code" => commands::toggle_mark(schema.mark_id("code")?, None),
        "quote" => commands::wrap_in(schema.node_id("blockquote")?, None),
        "bullet_list" => commands::wrap_in_list(schema.node_id("bullet_list")?, None),
        "ordered_list" => commands::wrap_in_list(schema.node_id("ordered_list")?, None),
        "undo" => commands::undo(),
        "redo" => commands::redo(),
        _ => return None,
    };
    command(state)
}
