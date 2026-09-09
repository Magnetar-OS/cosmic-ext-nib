// SPDX-License-Identifier: MPL-2.0

//! The caret: where it is, what shape it is, and how it gets there.
//!
//! # Why it animates at all
//!
//! A caret that teleports is correct and feels broken. The eye tracks a moving
//! object and loses a jumping one, so a caret that slides the width of a
//! character when you press an arrow key is a caret you do not lose — and one
//! that slides *across a line break* is one you can follow to the next line.
//! The distance is small and the duration is short; the point is continuity,
//! not spectacle.
//!
//! The glide is capped rather than proportional: a caret that took the same
//! time to cross a paragraph as to cross a character would lag behind held
//! keys, and one that moved at a constant speed would take a visible age to
//! cross a long document after Ctrl+End. Beyond a threshold it simply appears.
//!
//! # Why blinking stops while you type
//!
//! Because a caret that blinks out on the frame you press a key reads as a
//! dropped keystroke. Every edit restarts the cycle at full brightness.

use std::time::{Duration, Instant};

use cosmic::iced::Rectangle;

use crate::style::Caret;

/// The caret's position over time.
#[derive(Debug, Clone)]
pub struct Animation {
    /// Where it is being drawn.
    current: Option<Rectangle>,
    /// Where it is heading.
    target: Option<Rectangle>,
    /// Where the current glide started, and when.
    from: Option<Rectangle>,
    started: Instant,
    /// The last time the caret moved or the document changed.
    touched: Instant,
}

/// Beyond this distance the caret does not glide; it appears.
const GLIDE_LIMIT: f32 = 240.0;

/// How long after a change the caret stays solid before resuming its blink.
const SOLID_AFTER_CHANGE: Duration = Duration::from_millis(500);

impl Animation {
    #[must_use]
    pub fn new(now: Instant) -> Self {
        Self {
            current: None,
            target: None,
            from: None,
            started: now,
            touched: now,
        }
    }

    /// Tells the caret where it should be.
    pub fn aim(&mut self, target: Option<Rectangle>, now: Instant, glide: Duration) {
        if self.target == target {
            return;
        }
        self.touched = now;
        self.started = now;
        self.from = self.current;
        self.target = target;

        let jump = match (self.current, target) {
            (Some(from), Some(to)) => {
                glide.is_zero() || (to.x - from.x).abs() + (to.y - from.y).abs() > GLIDE_LIMIT
            }
            // Appearing or disappearing is not a move.
            _ => true,
        };
        if jump {
            self.current = target;
            self.from = target;
        }
    }

    /// Restarts the blink at full brightness — after a keystroke, so the caret
    /// is never dark on the frame the user acted.
    pub fn touch(&mut self, now: Instant) {
        self.touched = now;
    }

    /// Advances the animation. Returns true while it still has somewhere to
    /// go, so the caller knows to ask for another frame.
    pub fn tick(&mut self, now: Instant, glide: Duration) -> bool {
        let (Some(from), Some(target)) = (self.from, self.target) else {
            self.current = self.target;
            return false;
        };
        if glide.is_zero() || from == target {
            self.current = self.target;
            return false;
        }
        let elapsed = now.saturating_duration_since(self.started);
        if elapsed >= glide {
            self.current = self.target;
            self.from = self.target;
            return false;
        }
        let t = elapsed.as_secs_f32() / glide.as_secs_f32();
        // Ease out: fast off the mark, settling into place. A linear glide
        // reads as a drag rather than a movement.
        let t = 1.0 - (1.0 - t) * (1.0 - t);
        self.current = Some(Rectangle {
            x: from.x + (target.x - from.x) * t,
            y: from.y + (target.y - from.y) * t,
            width: from.width + (target.width - from.width) * t,
            height: from.height + (target.height - from.height) * t,
        });
        true
    }

    /// Where to draw the caret now, or `None` when it is blinked out or there
    /// is nowhere to draw it.
    #[must_use]
    pub fn visible(&self, now: Instant, period: Duration) -> Option<Rectangle> {
        let rect = self.current?;
        if !self.is_lit(now, period) {
            return None;
        }
        Some(rect)
    }

    fn is_lit(&self, now: Instant, period: Duration) -> bool {
        if period.is_zero() {
            return true;
        }
        let since = now.saturating_duration_since(self.touched);
        if since < SOLID_AFTER_CHANGE {
            return true;
        }
        let half = period.as_secs_f64() / 2.0;
        // Even half-periods are lit, odd ones dark. Counting in whole
        // half-periods rather than converting a float keeps the arithmetic
        // exact and the lint quiet about a cast that could never go wrong.
        let half = Duration::from_secs_f64(half);
        let phase = since.as_nanos() / half.as_nanos().max(1);
        phase.is_multiple_of(2)
    }

    /// When the caret next changes brightness, so the frame can be asked for
    /// then rather than continuously.
    ///
    /// This is what keeps a blinking caret from costing a redraw every frame:
    /// two wake-ups a second, not sixty.
    #[must_use]
    pub fn next_blink(&self, now: Instant, period: Duration) -> Option<Instant> {
        if period.is_zero() || self.current.is_none() {
            return None;
        }
        let since = now.saturating_duration_since(self.touched);
        if since < SOLID_AFTER_CHANGE {
            return Some(self.touched + SOLID_AFTER_CHANGE);
        }
        let half = period.as_secs_f64() / 2.0;
        let phase = (since.as_secs_f64() / half).floor() + 1.0;
        Some(self.touched + Duration::from_secs_f64(phase * half))
    }
}

/// The rectangle a caret occupies, given where the text says it is.
///
/// `at` is the caret's own line: the point the shaper gives for the position,
/// and the height of the line it sits on. The shape decides what is drawn
/// there, and `advance` — the width of the character after the caret — is what
/// a block or underline caret covers.
#[must_use]
pub fn rectangle(shape: Caret, at: Rectangle, text_size: f32, advance: Option<f32>) -> Rectangle {
    let width = advance
        .filter(|_| matches!(shape, Caret::Block | Caret::Underline))
        .unwrap_or_else(|| shape.width(text_size));
    match shape {
        Caret::Line | Caret::Bar => Rectangle {
            width: shape.width(text_size),
            ..at
        },
        Caret::Block => Rectangle { width, ..at },
        Caret::Underline => Rectangle {
            y: at.y + at.height - (text_size * 0.1).max(1.5),
            height: (text_size * 0.1).max(1.5),
            width,
            ..at
        },
    }
}

/// How opaque a block caret's fill is.
///
/// A solid block hides the character under it, which is fine in a terminal
/// where the character is redrawn in the background colour and wrong here,
/// where the text underneath is styled and the caret is not the only thing on
/// the line.
pub const BLOCK_ALPHA: f32 = 0.35;
