// SPDX-License-Identifier: MPL-2.0

//! What gets a squiggle, and — more importantly — what does not.

use nib_model::basic::{self, nodes};
use nib_model::build::Builder;
use nib_model::decoration::Kind;
use nib_model::node::Node;
use nib_model::{attrs, nodes};
use nib_spell::{Speller, worth_checking};

/// A dictionary of six words, which is all these tests need.
///
/// Built from strings rather than the system's files: a test that only runs on
/// a machine with `hunspell-en_us` installed is a test that does not run.
fn speller() -> Speller {
    let aff = "SET UTF-8\n";
    let dic = "6\nhello\nworld\ndocument\nthe\nis\na\n";
    Speller::from_strings(aff, dic, "test").expect("a dictionary this small parses")
}

fn b() -> Builder {
    Builder::new(basic::schema())
}

fn misspellings(speller: &Speller, doc: &Node) -> Vec<(usize, usize)> {
    speller
        .decorate(doc)
        .all()
        .iter()
        .filter(|d| matches!(d.kind(), Kind::Inline(_)))
        .map(|d| (d.from(), d.to()))
        .collect()
}

fn words(speller: &Speller, doc: &Node) -> Vec<String> {
    let text = doc.text_content();
    misspellings(speller, doc)
        .iter()
        .map(|(from, to)| text[from - 1..to - 1].to_owned())
        .collect()
}

#[test]
fn a_known_word_gets_no_squiggle() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("hello world")])]);
    assert!(misspellings(&speller(), &doc).is_empty());
}

#[test]
fn an_unknown_word_gets_one() {
    let b = b();
    let doc = b.doc(nodes![b.node(
        nodes::PARAGRAPH,
        nodes![b.text("hello wrold")]
    )]);
    assert_eq!(words(&speller(), &doc), ["wrold"]);
}

#[test]
fn the_squiggle_covers_the_word_and_nothing_else() {
    let b = b();
    let doc = b.doc(nodes![b.node(
        nodes::PARAGRAPH,
        nodes![b.text("hello wrold there")]
    )]);
    let found = misspellings(&speller(), &doc);
    assert_eq!(found.len(), 2, "wrold and there: {found:?}");
    // "hello" starts at position 1; "wrold" is six bytes later.
    assert_eq!(found[0], (7, 12));
}

#[test]
fn a_code_block_is_not_prose() {
    let b = b();
    let doc = b.doc(nodes![b.attr_node(
        nodes::CODE_BLOCK,
        &attrs! { "language" => "rust" },
        nodes![b.text("let wrold = zzz;")]
    )]);
    assert!(
        misspellings(&speller(), &doc).is_empty(),
        "a variable name is not a misspelling"
    );
}

#[test]
fn inline_code_is_not_prose_either() {
    let b = b();
    let doc = b.doc(nodes![b.node(
        nodes::PARAGRAPH,
        nodes![
            b.text("the "),
            b.mark("code", None, nodes![b.text("zzzz")]),
            b.text(" is a document"),
        ]
    )]);
    assert!(misspellings(&speller(), &doc).is_empty());
}

#[test]
fn things_that_are_not_words_are_left_alone() {
    for not_a_word in ["a", "x", "v2", "3rd", "some_name", "a/b", "C:\\x"] {
        assert!(
            !worth_checking(not_a_word),
            "{not_a_word:?} should not be checked"
        );
    }
    for word in ["hello", "misspelt", "Grüße"] {
        assert!(worth_checking(word), "{word:?} should be checked");
    }
}

#[test]
fn a_learnt_word_stops_being_a_misspelling() {
    let b = b();
    let doc = b.doc(nodes![b.node(nodes::PARAGRAPH, nodes![b.text("hello zzzz")])]);
    let mut speller = speller();
    assert_eq!(words(&speller, &doc), ["zzzz"]);

    speller.learn("zzzz");
    assert!(misspellings(&speller, &doc).is_empty());
    assert_eq!(speller.learnt().collect::<Vec<_>>(), ["zzzz"]);

    speller.unlearn("zzzz");
    assert_eq!(words(&speller, &doc), ["zzzz"]);
}

#[test]
fn the_word_under_a_position_is_found_with_its_range() {
    let b = b();
    let doc = b.doc(nodes![b.node(
        nodes::PARAGRAPH,
        nodes![b.text("hello wrold")]
    )]);
    let speller = speller();
    // Inside "wrold".
    let (word, from, to) = speller.word_at(&doc, 9).expect("a misspelling there");
    assert_eq!(word, "wrold");
    assert_eq!((from, to), (7, 12));

    // Inside "hello", which is spelled correctly.
    assert!(speller.word_at(&doc, 3).is_none());
}

#[test]
fn several_blocks_are_each_checked_in_their_own_coordinates() {
    let b = b();
    let doc = b.doc(nodes![
        b.node(nodes::PARAGRAPH, nodes![b.text("hello")]),
        b.node(nodes::PARAGRAPH, nodes![b.text("wrold")]),
    ]);
    let found = misspellings(&speller(), &doc);
    assert_eq!(found.len(), 1);
    // The second paragraph's content starts after the first's seven positions.
    assert_eq!(found[0], (8, 13));
}

#[test]
fn a_machine_with_no_dictionaries_says_so_rather_than_failing() {
    // Whatever this machine has, asking for a language nobody packages is the
    // normal "off" state and not a fault.
    let missing = Speller::system("zz_ZZ");
    assert!(missing.is_err());
    // And the list of what is installed is always answerable.
    let _ = Speller::installed();
}
