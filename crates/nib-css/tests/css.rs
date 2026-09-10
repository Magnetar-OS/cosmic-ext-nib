// SPDX-License-Identifier: MPL-2.0

//! What the subset lets through, and what it refuses.

use nib_css::{Align, Declarations, Hiding, Refusal, parse, parse_color};
use nib_model::decoration::Rgba;

// ---------------------------------------------------------------------------
// Colours
// ---------------------------------------------------------------------------

#[test]
fn hex_colours_read_in_every_length() {
    assert_eq!(parse_color("#c00"), Some(Rgba::new(0xcc, 0, 0, 0xff)));
    assert_eq!(parse_color("#cc0000"), Some(Rgba::new(0xcc, 0, 0, 0xff)));
    assert_eq!(parse_color("#cc000080"), Some(Rgba::new(0xcc, 0, 0, 0x80)));
    // `#rgba`: the alpha digit doubles like the others.
    assert_eq!(parse_color("#c00f"), Some(Rgba::new(0xcc, 0, 0, 0xff)));
}

#[test]
fn a_malformed_colour_is_no_colour_rather_than_a_guess() {
    // A mis-parsed colour is still a colour, and one nobody chose is worse
    // than the reader's own.
    assert_eq!(parse_color("#gg0000"), None);
    assert_eq!(parse_color("#12345"), None);
    assert_eq!(parse_color("not-a-colour"), None);
    assert_eq!(parse_color("hsl(0 100% 50%)"), None);
}

#[test]
fn rgb_reads_both_spellings_and_percentages() {
    let red = Some(Rgba::new(255, 0, 0, 0xff));
    assert_eq!(parse_color("rgb(255, 0, 0)"), red);
    assert_eq!(parse_color("rgb(255 0 0)"), red);
    assert_eq!(parse_color("rgb(100%, 0%, 0%)"), red);
    assert_eq!(parse_color("rgba(255, 0, 0, 1)"), red);
    assert_eq!(parse_color("rgba(255 0 0 / 50%)"), Some(Rgba::new(255, 0, 0, 128)));
}

#[test]
fn a_channel_out_of_range_saturates_rather_than_wrapping() {
    // `rgb(300, -20, 0)` is invalid CSS; clamping is what browsers do, and
    // wrapping would turn an over-bright red into a dark one.
    assert_eq!(parse_color("rgb(300, -20, 0)"), Some(Rgba::new(255, 0, 0, 0xff)));
}

// ---------------------------------------------------------------------------
// What is honoured
// ---------------------------------------------------------------------------

#[test]
fn the_text_properties_survive() {
    let d = parse("color: #c00; font-weight: bold; font-style: italic; text-align: center");
    assert_eq!(d.color, Some(Rgba::new(0xcc, 0, 0, 0xff)));
    assert_eq!(d.bold, Some(true));
    assert_eq!(d.italic, Some(true));
    assert_eq!(d.align, Some(Align::Center));
}

#[test]
fn a_numeric_font_weight_is_bold_from_600() {
    assert_eq!(parse("font-weight: 700").bold, Some(true));
    assert_eq!(parse("font-weight: 600").bold, Some(true));
    assert_eq!(parse("font-weight: 400").bold, Some(false));
}

#[test]
fn text_decoration_reads_both_and_neither() {
    let both = parse("text-decoration: underline line-through");
    assert_eq!((both.underline, both.strikethrough), (Some(true), Some(true)));
    // Naming neither turns both off — which is how `text-decoration: none`
    // undoes a link's underline.
    let none = parse("text-decoration: none");
    assert_eq!((none.underline, none.strikethrough), (Some(false), Some(false)));
}

#[test]
fn font_size_becomes_a_ratio_not_a_measurement() {
    // The sender does not know the reader's base size or their display, so
    // what is kept is the ratio they chose.
    assert_eq!(parse("font-size: 200%").scale, Some(2.0));
    assert_eq!(parse("font-size: 2em").scale, Some(2.0));
    assert_eq!(parse("font-size: 32px").scale, Some(2.0));
    assert_eq!(parse("font-size: large").scale, Some(1.125));
}

#[test]
fn a_font_size_is_clamped_at_both_ends() {
    // Small enough to skip over, and large enough to fill the screen, are both
    // ways of controlling what the reader sees rather than how it looks.
    assert_eq!(parse("font-size: 4px").scale, Some(0.6));
    assert_eq!(parse("font-size: 900%").scale, Some(3.0));
}

// ---------------------------------------------------------------------------
// What is refused
// ---------------------------------------------------------------------------

#[test]
fn anything_that_would_fetch_is_refused_by_name() {
    let d = parse("background: url(https://tracker.example/p.gif)");
    assert_eq!(d.background, None, "the URL did not become a colour");
    assert!(matches!(d.refused.as_slice(), [Refusal::Fetches { .. }]));
}

#[test]
fn the_historic_execution_vectors_are_refused_with_it() {
    for css in [
        "width: expression(alert(1))",
        "background: javascript:alert(1)",
    ] {
        let d = parse(css);
        assert!(
            d.refused.iter().any(|r| matches!(r, Refusal::Fetches { .. })),
            "{css} was not refused"
        );
    }
}

#[test]
fn an_at_rule_refuses_the_whole_list() {
    // The parse is already not what the author expected, and guessing which
    // half was meant is how a bypass gets in.
    let d = parse("@import url(evil.css); color: red");
    assert_eq!(d.color, None, "a declaration beside an @rule was still applied");
    assert!(matches!(d.refused.as_slice(), [Refusal::Fetches { .. }]));
}

#[test]
fn positioning_is_refused_because_text_can_be_hidden_under_text() {
    for property in ["position", "float", "z-index", "transform", "clip"] {
        let d = parse(&format!("{property}: whatever"));
        assert!(
            d.refused.iter().any(|r| matches!(r, Refusal::Positions { .. })),
            "{property} was not refused"
        );
    }
}

// ---------------------------------------------------------------------------
// What is reported rather than obeyed
// ---------------------------------------------------------------------------

#[test]
fn hiding_is_counted_and_never_honoured() {
    // Honouring these would reintroduce the exact attack that text extraction
    // exists to expose, so they are reported instead.
    assert_eq!(parse("display: none").hiding, vec![Hiding::Removed]);
    assert_eq!(parse("visibility: hidden").hiding, vec![Hiding::Removed]);
    assert_eq!(parse("opacity: 0").hiding, vec![Hiding::Transparent]);
    assert_eq!(parse("opacity: 0.01").hiding, vec![Hiding::Transparent]);
    assert_eq!(parse("font-size: 0px").hiding, vec![Hiding::Shrunk]);

    assert!(!parse("opacity: 0.9").hides());
    assert!(!parse("display: block").hides());
}

#[test]
fn a_hiding_declaration_does_not_take_its_neighbours_with_it() {
    // The text stays visible *and* the attempt is on the record.
    let d = parse("color: #c00; display: none");
    assert_eq!(d.color, Some(Rgba::new(0xcc, 0, 0, 0xff)));
    assert!(d.hides());
}

// ---------------------------------------------------------------------------
// Inheritance
// ---------------------------------------------------------------------------

#[test]
fn the_inheriting_properties_inherit_and_background_does_not() {
    let parent = parse("color: #c00; text-align: right; background-color: #eee");
    let child = parse("font-weight: bold").inherited_from(&parent);

    assert_eq!(child.color, parent.color, "colour inherits");
    assert_eq!(child.align, parent.align, "alignment inherits");
    assert_eq!(child.bold, Some(true));
    assert_eq!(
        child.background, None,
        "a child with no background shows what is behind it, not its parent's paint"
    );
}

#[test]
fn a_child_overrides_rather_than_merges() {
    let parent = parse("color: red");
    let child = parse("color: blue").inherited_from(&parent);
    assert_eq!(child.color, Some(Rgba::new(0, 0, 255, 0xff)));
}

#[test]
fn nested_scales_compound_the_way_nested_percentages_do() {
    let parent = parse("font-size: 80%");
    let child = parse("font-size: 90%").inherited_from(&parent);
    assert_eq!(child.scale, Some(0.8 * 0.9));
}

#[test]
fn compounding_cannot_shrink_past_the_floor() {
    // The leak this guards: each element is individually within the limit, and
    // the text at the bottom of the nest is invisible anyway.
    let mut current = parse("font-size: 60%");
    for _ in 0..4 {
        current = parse("font-size: 60%").inherited_from(&current);
    }
    assert_eq!(current.scale, Some(0.6), "the floor leaked through nesting");
}

// ---------------------------------------------------------------------------
// The ordinary case
// ---------------------------------------------------------------------------

#[test]
fn a_declaration_list_of_nothing_useful_is_empty_rather_than_noisy() {
    let d = parse("margin: 0; padding: 0; border-collapse: collapse");
    assert!(d.is_empty());
    assert!(!d.hides());
    assert_eq!(d.refused.len(), 3, "recorded, but only so the count is honest");
}

#[test]
fn malformed_input_is_skipped_rather_than_fatal() {
    let d = parse(";;; color ; : ; color: #c00 ;;");
    assert_eq!(d.color, Some(Rgba::new(0xcc, 0, 0, 0xff)));
}

#[test]
fn an_empty_style_attribute_says_nothing() {
    assert_eq!(parse(""), Declarations::default());
}
