// SPDX-License-Identifier: MPL-2.0

//! Parses and round trips over generated input: every document parsed from
//! arbitrary HTML must satisfy its schema, and writing it out and reading it
//! back must give the same document.
//!
//! The source is stitched from tags, attributes and text chosen to cover the
//! rule table and the styling reader, including malformed nesting that the
//! HTML parser has to recover from. The generator is seeded, so a failure
//! names the exact input and reproduces on every run.

use nib_html::Html;
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
    " two words ",
    " ",
    "\n",
    "&amp;",
    "&lt;b&gt;",
    "<",
    "<p>",
    "</p>",
    "<b>",
    "</b>",
    "<i>",
    "</i>",
    "<u>",
    "</u>",
    "<s>",
    "</s>",
    "<code>",
    "</code>",
    "<pre>",
    "</pre>",
    "<br>",
    "<hr>",
    "<h1>",
    "</h1>",
    "<h3 style=\"text-align:center\">",
    "</h3>",
    "<blockquote>",
    "</blockquote>",
    "<ul>",
    "</ul>",
    "<ol start=\"3\">",
    "</ol>",
    "<li>",
    "</li>",
    "<a href=\"https://x.test\" title=\"t\">",
    "<a href=\"javascript:x\">",
    "</a>",
    "<img src=\"x.png\" alt=\"alt\">",
    "<table>",
    "</table>",
    "<tr>",
    "</tr>",
    "<td>",
    "</td>",
    "<th>",
    "</th>",
    "<div>",
    "</div>",
    "<span style=\"color:#c00\">",
    "<span>",
    "</span>",
    "<font size=\"5\" color=\"blue\">",
    "</font>",
    "<p align=\"right\">",
    "<span style=\"background:yellow;font-weight:bold\">",
    "<script>x</script>",
    "<style>p{}</style>",
    "<!-- note -->",
    "\t",
    "  ",
];

fn source(rng: &mut Rng) -> String {
    let len = 1 + rng.next() % 14;
    (0..len).map(|_| rng.pick(FRAGMENTS)).collect()
}

#[test]
fn writing_a_parsed_document_and_reading_it_back_changes_nothing() {
    let html = Html::new(&basic::schema());
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    let mut failures = Vec::new();
    for _ in 0..5_000 {
        let input = source(&mut rng);
        let doc = html.parse(&input);
        // No parse may produce a document the schema would refuse.
        if let Err(error) = doc.check() {
            failures.push(format!("input   {input:?}\ninvalid {error}\nparsed  {doc}"));
            continue;
        }
        let written = html.to_html(&doc);
        let again = html.parse(&written);
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
