// SPDX-License-Identifier: MPL-2.0

//! CSS colour values.
//!
//! Hex, `rgb()`/`rgba()`, and the named colours. Not `hsl()`, not `color()`,
//! not `var()` — a value this does not understand returns `None` and the
//! author's colour is simply not applied, which is the right failure: a
//! mis-parsed colour is a colour, and a colour nobody chose is worse than the
//! reader's own.

use nib_model::decoration::Rgba;

/// Reads a CSS colour value.
///
/// ```
/// # use nib_css::parse_color;
/// # use nib_model::decoration::Rgba;
/// assert_eq!(parse_color("#c00"), Some(Rgba::new(0xcc, 0, 0, 0xff)));
/// assert_eq!(parse_color("rgb(255 0 0)"), Some(Rgba::new(255, 0, 0, 0xff)));
/// assert_eq!(parse_color("rebeccapurple"), Some(Rgba::new(0x66, 0x33, 0x99, 0xff)));
/// assert_eq!(parse_color("hsl(0 100% 50%)"), None, "not in the subset");
/// ```
#[must_use]
pub fn parse_color(value: &str) -> Option<Rgba> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        return from_hex(hex);
    }
    let lower = value.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("rgba(").or_else(|| lower.strip_prefix("rgb(")) {
        return from_rgb(rest.strip_suffix(')')?);
    }
    named(&lower)
}

fn from_hex(hex: &str) -> Option<Rgba> {
    let digit = |c: u8| char::from(c).to_digit(16).and_then(|d| u8::try_from(d).ok());
    let bytes = hex.as_bytes();
    match bytes.len() {
        // `#rgb` and `#rgba`: each digit doubled, so `c` is `cc`.
        3 | 4 => {
            let mut channels = [0xff_u8; 4];
            for (i, byte) in bytes.iter().enumerate() {
                let d = digit(*byte)?;
                channels[i] = d * 17;
            }
            Some(Rgba::new(channels[0], channels[1], channels[2], channels[3]))
        }
        6 | 8 => {
            let mut channels = [0xff_u8; 4];
            for (i, pair) in bytes.as_chunks::<2>().0.iter().enumerate() {
                channels[i] = digit(pair[0])? * 16 + digit(pair[1])?;
            }
            Some(Rgba::new(channels[0], channels[1], channels[2], channels[3]))
        }
        _ => None,
    }
}

/// The inside of `rgb(…)`, in either the comma or the space spelling.
fn from_rgb(inner: &str) -> Option<Rgba> {
    let parts: Vec<&str> = inner
        .split([',', '/', ' '])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() < 3 {
        return None;
    }
    let channel = |p: &str| -> Option<u8> {
        let value = if let Some(percent) = p.strip_suffix('%') {
            percent.parse::<f32>().ok()? / 100.0 * 255.0
        } else {
            p.parse::<f32>().ok()?
        };
        Some(to_byte(value / 255.0))
    };
    let alpha = match parts.get(3) {
        None => 0xff,
        Some(a) => {
            let v = if let Some(percent) = a.strip_suffix('%') {
                percent.parse::<f32>().ok()? / 100.0
            } else {
                a.parse::<f32>().ok()?
            };
            to_byte(v)
        }
    };
    Some(Rgba::new(
        channel(parts[0])?,
        channel(parts[1])?,
        channel(parts[2])?,
        alpha,
    ))
}

/// A 0..=1 fraction as a channel byte, saturating rather than wrapping.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamped to 0..=255 on the line above the cast"
)]
fn to_byte(fraction: f32) -> u8 {
    (fraction.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// The CSS named colours.
///
/// The full list, because a partial one is a colour that silently does not
/// apply for reasons nobody can see, and the table is cheap.
#[allow(
    clippy::unreadable_literal,
    reason = "`0xf0f8ff` is how a colour is written; `0x00f0_f8ff` is not"
)]
#[allow(clippy::match_same_arms, reason = "one arm per colour name, in CSS order")]
fn named(name: &str) -> Option<Rgba> {
    let hex = match name {
        "transparent" => return Some(Rgba::new(0, 0, 0, 0)),
        "aliceblue" => 0xf0f8ff, "antiquewhite" => 0xfaebd7, "aqua" => 0x00ffff,
        "aquamarine" => 0x7fffd4, "azure" => 0xf0ffff, "beige" => 0xf5f5dc,
        "bisque" => 0xffe4c4, "black" => 0x000000, "blanchedalmond" => 0xffebcd,
        "blue" => 0x0000ff, "blueviolet" => 0x8a2be2, "brown" => 0xa52a2a,
        "burlywood" => 0xdeb887, "cadetblue" => 0x5f9ea0, "chartreuse" => 0x7fff00,
        "chocolate" => 0xd2691e, "coral" => 0xff7f50, "cornflowerblue" => 0x6495ed,
        "cornsilk" => 0xfff8dc, "crimson" => 0xdc143c, "cyan" => 0x00ffff,
        "darkblue" => 0x00008b, "darkcyan" => 0x008b8b, "darkgoldenrod" => 0xb8860b,
        "darkgray" | "darkgrey" => 0xa9a9a9, "darkgreen" => 0x006400,
        "darkkhaki" => 0xbdb76b, "darkmagenta" => 0x8b008b, "darkolivegreen" => 0x556b2f,
        "darkorange" => 0xff8c00, "darkorchid" => 0x9932cc, "darkred" => 0x8b0000,
        "darksalmon" => 0xe9967a, "darkseagreen" => 0x8fbc8f, "darkslateblue" => 0x483d8b,
        "darkslategray" | "darkslategrey" => 0x2f4f4f, "darkturquoise" => 0x00ced1,
        "darkviolet" => 0x9400d3, "deeppink" => 0xff1493, "deepskyblue" => 0x00bfff,
        "dimgray" | "dimgrey" => 0x696969, "dodgerblue" => 0x1e90ff,
        "firebrick" => 0xb22222, "floralwhite" => 0xfffaf0, "forestgreen" => 0x228b22,
        "fuchsia" => 0xff00ff, "gainsboro" => 0xdcdcdc, "ghostwhite" => 0xf8f8ff,
        "gold" => 0xffd700, "goldenrod" => 0xdaa520, "gray" | "grey" => 0x808080,
        "green" => 0x008000, "greenyellow" => 0xadff2f, "honeydew" => 0xf0fff0,
        "hotpink" => 0xff69b4, "indianred" => 0xcd5c5c, "indigo" => 0x4b0082,
        "ivory" => 0xfffff0, "khaki" => 0xf0e68c, "lavender" => 0xe6e6fa,
        "lavenderblush" => 0xfff0f5, "lawngreen" => 0x7cfc00, "lemonchiffon" => 0xfffacd,
        "lightblue" => 0xadd8e6, "lightcoral" => 0xf08080, "lightcyan" => 0xe0ffff,
        "lightgoldenrodyellow" => 0xfafad2, "lightgray" | "lightgrey" => 0xd3d3d3,
        "lightgreen" => 0x90ee90, "lightpink" => 0xffb6c1, "lightsalmon" => 0xffa07a,
        "lightseagreen" => 0x20b2aa, "lightskyblue" => 0x87cefa,
        "lightslategray" | "lightslategrey" => 0x778899, "lightsteelblue" => 0xb0c4de,
        "lightyellow" => 0xffffe0, "lime" => 0x00ff00, "limegreen" => 0x32cd32,
        "linen" => 0xfaf0e6, "magenta" => 0xff00ff, "maroon" => 0x800000,
        "mediumaquamarine" => 0x66cdaa, "mediumblue" => 0x0000cd,
        "mediumorchid" => 0xba55d3, "mediumpurple" => 0x9370db,
        "mediumseagreen" => 0x3cb371, "mediumslateblue" => 0x7b68ee,
        "mediumspringgreen" => 0x00fa9a, "mediumturquoise" => 0x48d1cc,
        "mediumvioletred" => 0xc71585, "midnightblue" => 0x191970,
        "mintcream" => 0xf5fffa, "mistyrose" => 0xffe4e1, "moccasin" => 0xffe4b5,
        "navajowhite" => 0xffdead, "navy" => 0x000080, "oldlace" => 0xfdf5e6,
        "olive" => 0x808000, "olivedrab" => 0x6b8e23, "orange" => 0xffa500,
        "orangered" => 0xff4500, "orchid" => 0xda70d6, "palegoldenrod" => 0xeee8aa,
        "palegreen" => 0x98fb98, "paleturquoise" => 0xafeeee,
        "palevioletred" => 0xdb7093, "papayawhip" => 0xffefd5, "peachpuff" => 0xffdab9,
        "peru" => 0xcd853f, "pink" => 0xffc0cb, "plum" => 0xdda0dd,
        "powderblue" => 0xb0e0e6, "purple" => 0x800080, "rebeccapurple" => 0x663399,
        "red" => 0xff0000, "rosybrown" => 0xbc8f8f, "royalblue" => 0x4169e1,
        "saddlebrown" => 0x8b4513, "salmon" => 0xfa8072, "sandybrown" => 0xf4a460,
        "seagreen" => 0x2e8b57, "seashell" => 0xfff5ee, "sienna" => 0xa0522d,
        "silver" => 0xc0c0c0, "skyblue" => 0x87ceeb, "slateblue" => 0x6a5acd,
        "slategray" | "slategrey" => 0x708090, "snow" => 0xfffafa,
        "springgreen" => 0x00ff7f, "steelblue" => 0x4682b4, "tan" => 0xd2b48c,
        "teal" => 0x008080, "thistle" => 0xd8bfd8, "tomato" => 0xff6347,
        "turquoise" => 0x40e0d0, "violet" => 0xee82ee, "wheat" => 0xf5deb3,
        "white" => 0xffffff, "whitesmoke" => 0xf5f5f5, "yellow" => 0xffff00,
        "yellowgreen" => 0x9acd32,
        _ => return None,
    };
    let [_, r, g, b] = u32::to_be_bytes(hex);
    Some(Rgba::new(r, g, b, 0xff))
}
