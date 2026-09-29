// SPDX-License-Identifier: MPL-2.0

//! Parses and round trips over generated input: every document parsed from
//! arbitrary Markdown must satisfy its schema, and writing it out and reading
//! it back must give the same document.
//!
//! The source is stitched from fragments chosen for being syntax, or nearly
//! syntax, in as many contexts as possible. The generator is seeded, so a
//! failure names the exact input and reproduces on every run.

use nib_markdown::{Dialect, Markdown};
use nib_model::basic;

/// A small, fixed-seed generator: the point is coverage that repeats.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn pick<'a>(&mut self, from: &[&'a str]) -> &'a str {
        let index = usize::try_from(self.next() % from.len() as u64).expect("in range");
        from[index]
    }
}

const FRAGMENTS: &[&str] = &[
    "word",
    "two words",
    " ",
    "  ",
    "\n",
    "\n\n",
    "  \n",
    "*",
    "**",
    "_",
    "__",
    "`",
    "``",
    "```",
    "~",
    "~~",
    "\\",
    "[",
    "]",
    "(",
    ")",
    "](",
    "![",
    "<",
    ">",
    "<b>",
    "</b>",
    "&",
    "&amp;",
    "&#35;",
    "#",
    "# ",
    "## ",
    "- ",
    "+ ",
    "* ",
    "1. ",
    "2) ",
    "1986. ",
    "> ",
    ">> ",
    "---",
    "===",
    "|",
    "| a | b |\n| --- | --- |\n",
    ":",
    "http://x.test",
    "<http://x.test>",
    "[link](http://x.test \"t\")",
    "![img](x.png)",
    "    indented",
    "\t",
    "- [ ] ",
    "- [x] ",
    "```js\ncode\n```\n",
    "`code`",
    "**bold**",
    "*em*",
    "~~gone~~",
    "\"",
    "'",
    "=",
    "!",
];

fn source(rng: &mut Rng) -> String {
    let len = 1 + rng.next() % 12;
    (0..len).map(|_| rng.pick(FRAGMENTS)).collect()
}

#[test]
fn writing_a_parsed_document_and_reading_it_back_changes_nothing() {
    let md = Markdown::new(&basic::schema(), Dialect::Gfm);
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut failures = Vec::new();
    for _ in 0..5_000 {
        let input = source(&mut rng);
        let doc = md.parse(&input);
        // No parse may produce a document the schema would refuse.
        if let Err(error) = doc.check() {
            failures.push(format!("input   {input:?}\ninvalid {error}\nparsed  {doc}"));
            continue;
        }
        let written = md.to_markdown(&doc);
        let again = md.parse(&written);
        if again != doc {
            failures.push(format!(
                "input   {input:?}\nparsed  {doc}\nwritten {written:?}\nreread  {again}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of 5000 changed; the first few:\n\n{}",
        failures.len(),
        failures
            .iter()
            .take(8)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n\n")
    );
}
