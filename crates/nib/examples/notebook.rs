// SPDX-License-Identifier: MPL-2.0

//! The editor, in a real window.
//!
//! `cargo run --example notebook`
//!
//! Type. Press Enter in the middle of a paragraph, and at the end of the
//! heading. Type `# ` at the start of a line, or `- `, or `> `, or `**bold**`.
//! Select across two blocks and press Ctrl+B. Press Ctrl+Z, then Ctrl+Shift+Z.
//! Press Tab inside a list item.
//!
//! Note what the application holds: an `EditorState` and nothing else. It has
//! no cursor, no selection, no undo stack and no idea what a paragraph is. The
//! widget hands it a transaction and it applies it.

use cosmic::app::{Core, Settings, Task};
use cosmic::iced::advanced::widget::Id;
use cosmic::iced::{Length, Size};
use cosmic::prelude::*;
use cosmic::widget;

use nib::{Action, Style, editor, toolbar_command};
use nib_model::basic::nodes;
use nib_model::input_rules::{self, InputRule};
use nib_model::keymap::Keymap;
use nib_model::state::EditorState;
use nib_model::{Transaction, basic, history};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let settings = Settings::default().size(Size::new(820.0, 720.0));
    cosmic::app::run::<App>(settings, ())?;
    Ok(())
}

#[derive(Clone, Debug)]
enum Message {
    /// Everything the editor does arrives here.
    Edit(Action),
    /// A toolbar button, by the name the widget knows it as.
    Toolbar(&'static str),
}

struct App {
    core: Core,
    state: EditorState,
    keymap: Keymap,
    rules: Vec<InputRule>,
}

impl App {
    fn apply(&mut self, tr: Transaction) {
        self.state = self.state.applied(tr);
    }

    fn toolbar(&self) -> Element<'_, Message> {
        let button = |label: &'static str, name: &'static str| {
            widget::button::text(label)
                .on_press_maybe(toolbar_command(&self.state, name).map(|_| Message::Toolbar(name)))
                .into()
        };
        widget::row::with_children(vec![
            button("Bold", "bold"),
            button("Italic", "italic"),
            button("Code", "code"),
            button("Quote", "quote"),
            button("List", "bullet_list"),
            widget::space::horizontal().into(),
            button("Undo", "undo"),
            button("Redo", "redo"),
        ])
        .spacing(4)
        .padding(8)
        .into()
    }
}

impl cosmic::Application for App {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;

    const APP_ID: &'static str = "org.magnetar.NibNotebook";

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _flags: ()) -> (Self, Task<Message>) {
        let schema = basic::schema();
        let doc = starting_document();
        let state = EditorState::with_selection(
            schema.clone(),
            doc,
            nib_model::Selection::cursor(1),
            vec![
                history::history(history::Options::default()),
                input_rules::input_rules_plugin(),
            ],
        );
        let app = Self {
            core,
            keymap: Keymap::base(&schema),
            rules: input_rules::base(&schema),
            state,
        };
        (app, Task::none())
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Edit(Action::Edit(tr)) => self.apply(*tr),
            Message::Edit(_) => {}
            Message::Toolbar(name) => {
                if let Some(tr) = toolbar_command(&self.state, name) {
                    self.apply(tr);
                }
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let editor = editor(&self.state)
            .id(Id::new("nib-editor"))
            .autofocus()
            .keymap(&self.keymap)
            .input_rules(&self.rules)
            .style(Style::from_theme(&cosmic::theme::active()))
            .on_action(Message::Edit);

        widget::column::with_children(vec![
            self.toolbar(),
            widget::divider::horizontal::default().into(),
            widget::scrollable(editor)
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
        ])
        .into()
    }
}

/// Something with one of everything in it.
fn starting_document() -> nib_model::Node {
    let schema = basic::schema();
    let md = "\
# Nib

A rich text editor for COSMIC, in Rust, with no web engine anywhere in the
pipeline. This paragraph has **bold**, *emphasis*, `code`, and a
[link](https://github.com/Magnetar-OS/cosmic-ext-nib).

## Try it

- Press Enter in the middle of a line
- Type `# ` at the start of one
- Select across two blocks and press Ctrl+B
- Press Tab inside this item

> The selection runs from a position in one block to a position in another.
> That is why this is one widget and not a tree of them.

```rust
fn main() {
    println!(\"the schema is enforced, not advisory\");
}
```

| what | where |
| --- | --- |
| model | nib-model |
| view | nib |
";
    nib_markdown_parse(&schema, md)
}

/// The example carries its starting text as Markdown, which means depending on
/// the Markdown crate. Rather than do that for one string, the parse is done
/// here by hand — badly, and only well enough for the text above.
#[allow(clippy::too_many_lines)]
fn nib_markdown_parse(schema: &nib_model::Schema, source: &str) -> nib_model::Node {
    use nib_model::attrs;
    use nib_model::build::Builder;

    let b = Builder::new(schema.clone());
    let mut blocks = Vec::new();
    let mut lines = source.lines().peekable();

    while let Some(line) = lines.next() {
        if line.trim().is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("## ") {
            blocks.push(b.attr_node(
                nodes::HEADING,
                &attrs! { "level" => 2_i64 },
                nib_model::nodes![b.text(rest)],
            ));
        } else if let Some(rest) = line.strip_prefix("# ") {
            blocks.push(b.attr_node(
                nodes::HEADING,
                &attrs! { "level" => 1_i64 },
                nib_model::nodes![b.text(rest)],
            ));
        } else if line.starts_with("```") {
            let mut code = String::new();
            for inner in lines.by_ref() {
                if inner.starts_with("```") {
                    break;
                }
                code.push_str(inner);
                code.push('\n');
            }
            blocks.push(b.attr_node(
                nodes::CODE_BLOCK,
                &attrs! { "language" => "rust" },
                nib_model::nodes![b.text(code.trim_end())],
            ));
        } else if line.starts_with("- ") {
            let mut items = Vec::new();
            let mut current = Some(line);
            while let Some(item) = current {
                let Some(rest) = item.strip_prefix("- ") else {
                    break;
                };
                items.push(b.node(
                    nodes::LIST_ITEM,
                    nib_model::nodes![b.node(nodes::PARAGRAPH, nib_model::nodes![b.text(rest)])],
                ));
                current = lines.peek().copied().filter(|l| l.starts_with("- "));
                if current.is_some() {
                    lines.next();
                }
            }
            blocks.push(b.node(nodes::BULLET_LIST, items));
        } else if line.starts_with("> ") {
            let mut quoted = Vec::new();
            let mut current = Some(line);
            while let Some(item) = current {
                let Some(rest) = item.strip_prefix("> ") else {
                    break;
                };
                quoted.push(b.node(nodes::PARAGRAPH, nib_model::nodes![b.text(rest)]));
                current = lines.peek().copied().filter(|l| l.starts_with("> "));
                if current.is_some() {
                    lines.next();
                }
            }
            blocks.push(b.node(nodes::BLOCKQUOTE, quoted));
        } else if line.starts_with("| ") {
            let mut rows = Vec::new();
            let mut current = Some(line);
            while let Some(item) = current {
                if !item.starts_with("| ") {
                    break;
                }
                let cells: Vec<&str> = item.trim_matches('|').split('|').map(str::trim).collect();
                if !cells.iter().all(|c| c.chars().all(|ch| ch == '-')) {
                    rows.push(
                        b.node(
                            nodes::TABLE_ROW,
                            cells
                                .iter()
                                .map(|cell| {
                                    b.node(
                                        nodes::TABLE_CELL,
                                        nib_model::nodes![b.node(
                                            nodes::PARAGRAPH,
                                            nib_model::nodes![b.text(cell)]
                                        )],
                                    )
                                })
                                .collect::<Vec<_>>(),
                        ),
                    );
                }
                current = lines.peek().copied().filter(|l| l.starts_with("| "));
                if current.is_some() {
                    lines.next();
                }
            }
            blocks.push(b.node(nodes::TABLE, rows));
        } else {
            // Paragraphs run until a blank line, with inline markup by the
            // crudest possible reading.
            let mut text = line.to_owned();
            while let Some(next) = lines.peek() {
                if next.trim().is_empty() {
                    break;
                }
                text.push(' ');
                text.push_str(lines.next().unwrap_or_default());
            }
            blocks.push(b.node(nodes::PARAGRAPH, inline(&b, &text)));
        }
    }
    b.doc(blocks)
}

/// `**bold**`, `*em*`, `` `code` `` and `[text](href)`, and nothing else.
fn inline(b: &nib_model::build::Builder, text: &str) -> Vec<nib_model::Node> {
    use nib_model::attrs;

    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let next = ["**", "*", "`", "["]
            .iter()
            .filter_map(|d| rest.find(*d).map(|i| (i, *d)))
            // At the same offset the longer delimiter wins, or `**bold**`
            // reads as emphasis around `*bold*`.
            .min_by_key(|(i, d)| (*i, std::cmp::Reverse(d.len())));
        let Some((at, delimiter)) = next else {
            out.push(b.text(rest));
            break;
        };
        if at > 0 {
            out.push(b.text(&rest[..at]));
        }
        let after = &rest[at + delimiter.len()..];
        if delimiter == "[" {
            let Some(close) = after.find("](") else {
                out.push(b.text(&rest[at..]));
                break;
            };
            let label = &after[..close];
            let tail = &after[close + 2..];
            let Some(end) = tail.find(')') else {
                out.push(b.text(&rest[at..]));
                break;
            };
            out.extend(b.mark(
                "link",
                Some(&attrs! { "href" => &tail[..end] }),
                nib_model::nodes![b.text(label)],
            ));
            rest = &tail[end + 1..];
            continue;
        }
        let Some(close) = after.find(delimiter) else {
            out.push(b.text(&rest[at..]));
            break;
        };
        let mark = match delimiter {
            "**" => "strong",
            "*" => "em",
            _ => "code",
        };
        out.extend(b.mark(mark, None, nib_model::nodes![b.text(&after[..close])]));
        rest = &after[close + delimiter.len()..];
    }
    out
}
