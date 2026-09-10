// SPDX-License-Identifier: MPL-2.0

//! Binding keys to commands.
//!
//! The key type here is abstract — a named key and a set of modifiers — and
//! deliberately not any toolkit's. The model has no business knowing what an
//! `iced::keyboard::Key` is, and a keymap that is plain data can be tested,
//! serialised, shown in a cheat sheet, and rebound by a user without any of
//! that going through a widget.
//!
//! # The platform difference is one line
//!
//! [`Mods::PRIMARY`] is Ctrl everywhere and Cmd on macOS. Bindings are written
//! against it, so a keymap is written once. `Alt` on macOS produces characters
//! rather than acting as a modifier, which is why the default bindings below
//! avoid it for anything a user might type.

use std::collections::BTreeMap;

use crate::commands::{Command, chain};
use crate::state::{EditorState, Transaction};

bitflags_like! {
    /// Modifier keys held with a binding.
    pub struct Mods: u8 {
        const NONE = 0;
        const SHIFT = 1;
        const ALT = 2;
        /// Ctrl on Linux and Windows, Cmd on macOS.
        const PRIMARY = 4;
        /// The other one: Cmd on Linux and Windows (rare), Ctrl on macOS.
        const SECONDARY = 8;
    }
}

/// A key, named the way a keymap names it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Key {
    /// A character-producing key, by the character it produces unshifted.
    Char(char),
    Enter,
    Tab,
    Backspace,
    Delete,
    Escape,
    Home,
    End,
    PageUp,
    PageDown,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    /// Anything else, by name.
    Named(&'static str),
}

/// A key and its modifiers.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Binding {
    pub key: Key,
    pub mods: Mods,
}

impl Binding {
    #[must_use]
    pub fn new(key: Key, mods: Mods) -> Self {
        Self { key, mods }
    }

    #[must_use]
    pub fn plain(key: Key) -> Self {
        Self::new(key, Mods::NONE)
    }

    #[must_use]
    pub fn primary(key: Key) -> Self {
        Self::new(key, Mods::PRIMARY)
    }

    #[must_use]
    pub fn shift(key: Key) -> Self {
        Self::new(key, Mods::SHIFT)
    }

    #[must_use]
    pub fn primary_shift(key: Key) -> Self {
        Self::new(key, Mods::PRIMARY.union(Mods::SHIFT))
    }
}

/// Bindings from keys to commands.
///
/// Ordered, so a cheat sheet built from one lists bindings the same way twice.
#[derive(Default, Clone)]
pub struct Keymap {
    bindings: BTreeMap<Binding, Command>,
}

impl std::fmt::Debug for Keymap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Keymap")
            .field("bindings", &self.bindings.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl Keymap {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How far one Tab indents inside a code block.
    ///
    /// Four spaces, and spaces rather than a tab: the block's text is what gets
    /// saved, and a document that renders differently depending on the reader's
    /// tab width is one the author did not write.
    pub const CODE_INDENT: usize = 4;

    /// The bindings an editor has unless it says otherwise. See
    /// [`base_keymap`].
    #[must_use]
    pub fn base(schema: &crate::schema::Schema) -> Self {
        base_keymap(schema)
    }

    /// Binds a key. A second binding of the same key runs after the first, so
    /// an application can add to a default keymap without replacing it.
    #[must_use]
    pub fn bind(mut self, binding: Binding, command: Command) -> Self {
        match self.bindings.remove(&binding) {
            Some(existing) => {
                self.bindings
                    .insert(binding, chain(vec![existing, command]));
            }
            None => {
                self.bindings.insert(binding, command);
            }
        }
        self
    }

    /// Replaces whatever was bound to a key.
    #[must_use]
    pub fn rebind(mut self, binding: Binding, command: Command) -> Self {
        self.bindings.insert(binding, command);
        self
    }

    /// Removes a binding.
    #[must_use]
    pub fn unbind(mut self, binding: &Binding) -> Self {
        self.bindings.remove(binding);
        self
    }

    /// Every binding, for a cheat sheet or a settings page.
    pub fn bindings(&self) -> impl Iterator<Item = &Binding> {
        self.bindings.keys()
    }

    #[must_use]
    pub fn command(&self, binding: &Binding) -> Option<&Command> {
        self.bindings.get(binding)
    }

    /// The transaction a key press produces, or `None` when nothing is bound
    /// or the bound command does not apply here.
    #[must_use]
    pub fn handle(&self, state: &EditorState, binding: &Binding) -> Option<Transaction> {
        self.bindings.get(binding)?(state)
    }

    /// Adds every binding of `other`, chaining where both bind a key.
    #[must_use]
    pub fn merge(mut self, other: Self) -> Self {
        for (binding, command) in other.bindings {
            self = self.bind(binding, command);
        }
        self
    }
}

/// A tiny stand-in for `bitflags`, so the crate keeps its dependency list to
/// what it genuinely needs. Only the operations a modifier set uses.
#[macro_export]
#[doc(hidden)]
macro_rules! bitflags_like {
    (
        $(#[$outer:meta])*
        pub struct $name:ident: $repr:ty {
            $($(#[$inner:meta])* const $flag:ident = $value:expr;)*
        }
    ) => {
        $(#[$outer])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
        pub struct $name($repr);

        impl $name {
            $($(#[$inner])* pub const $flag: Self = Self($value);)*

            #[must_use]
            pub const fn bits(self) -> $repr { self.0 }

            #[must_use]
            pub const fn union(self, other: Self) -> Self { Self(self.0 | other.0) }

            #[must_use]
            pub const fn contains(self, other: Self) -> bool {
                self.0 & other.0 == other.0
            }

            #[must_use]
            pub const fn is_empty(self) -> bool { self.0 == 0 }
        }

        impl ::core::ops::BitOr for $name {
            type Output = Self;
            fn bitor(self, other: Self) -> Self { self.union(other) }
        }
    };
}
use crate::bitflags_like;

// ---------------------------------------------------------------------------
// The default bindings
// ---------------------------------------------------------------------------

/// How far one Tab indents inside a code block.
///
/// Four spaces, and spaces rather than a tab: the block's text is what gets
/// saved, and a document that renders differently depending on the reader's
/// tab width is one the author did not write.
pub const CODE_INDENT: usize = 4;

/// The bindings an editor has unless it says otherwise.
///
/// Built against a schema by *name*, so a schema without lists simply gets no
/// list bindings rather than an error. An application that renames its node
/// types builds its own keymap; one that uses [`basic`](crate::basic) gets all
/// of this.
#[must_use]
#[allow(clippy::too_many_lines)]
fn base_keymap(schema: &crate::schema::Schema) -> Keymap {
    use crate::attrs::Attrs;
    use crate::basic::{marks, nodes};
    use crate::commands as cmd;

    let node = |name: &str| schema.node_id(name);
    let mark = |name: &str| schema.mark_id(name);

    let mut map = Keymap::new();

    // -- editing -----------------------------------------------------------
    let mut enter = vec![cmd::new_line_in_code(), cmd::create_paragraph_near()];
    if let Some(item) = node(nodes::LIST_ITEM) {
        enter.push(cmd::split_list_item(item));
    }
    enter.push(cmd::lift_empty_block());
    enter.push(cmd::split_block());
    map = map.bind(Binding::plain(Key::Enter), chain(enter));

    map = map
        .bind(Binding::shift(Key::Enter), cmd::exit_code())
        .bind(Binding::plain(Key::Backspace), cmd::delete_backward())
        .bind(Binding::plain(Key::Delete), cmd::delete_forward())
        .bind(Binding::primary(Key::Char('a')), cmd::select_all())
        .bind(Binding::plain(Key::Escape), cmd::select_parent_node())
        .bind(Binding::primary(Key::Char('z')), cmd::undo())
        .bind(Binding::primary_shift(Key::Char('z')), cmd::redo())
        .bind(Binding::primary(Key::Char('y')), cmd::redo());

    // -- marks -------------------------------------------------------------
    for (name, key) in [
        (marks::STRONG, 'b'),
        (marks::EM, 'i'),
        (marks::UNDERLINE, 'u'),
        (marks::CODE, 'e'),
    ] {
        if let Some(id) = mark(name) {
            map = map.bind(Binding::primary(Key::Char(key)), cmd::toggle_mark(id, None));
        }
    }
    if let Some(id) = mark(marks::STRIKETHROUGH) {
        map = map.bind(
            Binding::primary_shift(Key::Char('x')),
            cmd::toggle_mark(id, None),
        );
    }

    // -- blocks ------------------------------------------------------------
    if let Some(id) = node(nodes::PARAGRAPH) {
        map = map.bind(
            Binding::new(Key::Char('0'), Mods::PRIMARY.union(Mods::ALT)),
            cmd::set_block_type(id, None),
        );
    }
    if let Some(id) = node(nodes::HEADING) {
        for level in 1..=6_i64 {
            let key = char::from_digit(u32::try_from(level).unwrap_or(1), 10).unwrap_or('1');
            map = map.bind(
                Binding::new(Key::Char(key), Mods::PRIMARY.union(Mods::ALT)),
                cmd::set_block_type(id, Some(crate::attrs! { "level" => level })),
            );
        }
    }
    if let Some(id) = node(nodes::CODE_BLOCK) {
        map = map.bind(
            Binding::new(Key::Char('c'), Mods::PRIMARY.union(Mods::ALT)),
            cmd::set_block_type(id, None),
        );
    }
    if let Some(id) = node(nodes::BLOCKQUOTE) {
        map = map.bind(
            Binding::primary_shift(Key::Char('b')),
            cmd::wrap_in(id, None),
        );
    }
    if let Some(id) = node(nodes::BULLET_LIST) {
        map = map.bind(
            Binding::primary_shift(Key::Char('8')),
            cmd::wrap_in_list(id, None),
        );
    }
    if let Some(id) = node(nodes::ORDERED_LIST) {
        map = map.bind(
            Binding::primary_shift(Key::Char('7')),
            cmd::wrap_in_list(id, Some(Attrs::none())),
        );
    }

    // -- indentation -------------------------------------------------------
    //
    // Tab means two things and the schema decides which: inside a list it is
    // an outline level, inside code it is whitespace. Chained in that order,
    // because a list item inside a code block is not a thing that exists.
    let mut tab: Vec<cmd::Command> = Vec::new();
    let mut shift_tab: Vec<cmd::Command> = Vec::new();
    if let Some(item) = node(nodes::LIST_ITEM) {
        tab.push(cmd::sink_list_item(item));
        shift_tab.push(cmd::lift_list_item(item));
    }
    if node(nodes::CODE_BLOCK).is_some() {
        tab.push(cmd::indent_code(CODE_INDENT));
        shift_tab.push(cmd::outdent_code(CODE_INDENT));
    }
    if !tab.is_empty() {
        map = map
            .bind(Binding::plain(Key::Tab), chain(tab))
            .bind(Binding::shift(Key::Tab), chain(shift_tab));
    }
    map.bind(Binding::primary_shift(Key::Char('l')), cmd::lift())
}
