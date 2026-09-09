// SPDX-License-Identifier: MPL-2.0

//! Moving positions through changes.
//!
//! # The one rule
//!
//! A position taken before a change is meaningless after it unless it is
//! *mapped*. The cursor, the ends of a selection, a comment's anchor, a
//! collaborator's caret, a decoration's range, the target of a queued command —
//! every one of them is a position someone is holding across an edit, and every
//! one goes through here.
//!
//! Because it is one rule implemented once, it is also the whole of
//! collaborative editing: a step from another user is a change your local
//! positions have not seen, and rebasing your unsent steps onto it is mapping
//! them through it.
//!
//! # Association
//!
//! A position sitting exactly where text was inserted has two answers, and
//! neither is wrong. `assoc` picks: negative to stay before the insertion,
//! positive to move after it. A cursor uses positive — you type, the caret
//! follows what you typed. The left end of a selection being replaced uses
//! negative, so the selection does not swallow the replacement.
//!
//! # Recovery
//!
//! When a position falls *inside* a range that a step deleted, mapping it
//! forward loses where it was. [`MapResult::recover`] keeps that information,
//! so if a later step in the same [`Mapping`] is the recorded inverse of the
//! deleting one — undo, or a rebase that reapplies what was removed — the
//! position can be put back exactly where it started rather than at the edge of
//! the hole. That is why [`Mapping`] tracks mirrors at all.

/// A range that a step replaced: where it started, how long it was, and how
/// long it became.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Range {
    start: usize,
    old_size: usize,
    new_size: usize,
}

/// Where a position was, inside a range that no longer exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recover {
    index: usize,
    offset: usize,
}

/// What became of a mapped position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapResult {
    pos: usize,
    /// Bit flags; see the `deleted_*` accessors.
    del: u8,
    recover: Option<Recover>,
}

const DEL_BEFORE: u8 = 1;
const DEL_AFTER: u8 = 2;
const DEL_ACROSS: u8 = 4;
const DEL_SIDE: u8 = 8;

impl MapResult {
    #[must_use]
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// True when the content on the side the position was associated with was
    /// deleted — the position lost the thing it was anchored to.
    #[must_use]
    pub fn deleted(&self) -> bool {
        self.del & DEL_SIDE > 0
    }

    /// True when the content immediately before the position was deleted.
    #[must_use]
    pub fn deleted_before(&self) -> bool {
        self.del & (DEL_ACROSS | DEL_BEFORE) > 0
    }

    /// True when the content immediately after the position was deleted.
    #[must_use]
    pub fn deleted_after(&self) -> bool {
        self.del & (DEL_ACROSS | DEL_AFTER) > 0
    }

    /// True when the position was strictly inside deleted content — a
    /// selection endpoint that has nothing left to point at.
    #[must_use]
    pub fn deleted_across(&self) -> bool {
        self.del & DEL_ACROSS > 0
    }

    #[must_use]
    pub fn recover(&self) -> Option<Recover> {
        self.recover
    }
}

/// How one step moved every position in the document.
///
/// A step's whole effect on positions, independent of what it did to content —
/// which is why a step can be inverted and mapped without re-examining the
/// document it applied to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StepMap {
    ranges: Vec<Range>,
    inverted: bool,
}

impl StepMap {
    /// The map of a step that moved nothing.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// A map for one replaced range.
    #[must_use]
    pub fn single(start: usize, old_size: usize, new_size: usize) -> Self {
        Self {
            ranges: vec![Range {
                start,
                old_size,
                new_size,
            }],
            inverted: false,
        }
    }

    /// A map for two replaced ranges — what a gap-replace produces.
    #[must_use]
    pub fn pair(a: (usize, usize, usize), b: (usize, usize, usize)) -> Self {
        Self {
            ranges: vec![
                Range {
                    start: a.0,
                    old_size: a.1,
                    new_size: a.2,
                },
                Range {
                    start: b.0,
                    old_size: b.1,
                    new_size: b.2,
                },
            ],
            inverted: false,
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// The same change, read backwards.
    #[must_use]
    pub fn invert(&self) -> Self {
        Self {
            ranges: self.ranges.clone(),
            inverted: !self.inverted,
        }
    }

    /// Puts a position back where it was, given what
    /// [`MapResult::recover`] remembered.
    ///
    /// # Panics
    ///
    /// If the recovery data is not from this map.
    #[must_use]
    pub fn recover(&self, recover: Recover) -> usize {
        let mut diff: isize = 0;
        if !self.inverted {
            for range in &self.ranges[..recover.index] {
                diff += range.new_size as isize - range.old_size as isize;
            }
        }
        let start = self.ranges[recover.index].start;
        usize::try_from(start as isize + diff).unwrap_or(0) + recover.offset
    }

    /// Maps a position, discarding the detail.
    #[must_use]
    pub fn map(&self, pos: usize, assoc: i32) -> usize {
        self.map_result(pos, assoc).pos
    }

    /// Maps a position, reporting what happened to it.
    #[must_use]
    pub fn map_result(&self, pos: usize, assoc: i32) -> MapResult {
        let mut diff: isize = 0;
        for (i, range) in self.ranges.iter().enumerate() {
            let (old_size, new_size) = if self.inverted {
                (range.new_size, range.old_size)
            } else {
                (range.old_size, range.new_size)
            };
            let start = if self.inverted {
                usize::try_from(range.start as isize - diff).unwrap_or(0)
            } else {
                range.start
            };
            if start > pos {
                break;
            }
            let end = start + old_size;
            if pos <= end {
                // A pure insertion has no side to be on; the caller's
                // association decides. Otherwise the ends of the replaced
                // range keep their side and the interior follows `assoc`.
                let side = if old_size == 0 {
                    assoc
                } else if pos == start {
                    -1
                } else if pos == end {
                    1
                } else {
                    assoc
                };
                let mapped = usize::try_from(start as isize + diff).unwrap_or(0)
                    + if side < 0 { 0 } else { new_size };
                let anchor = if assoc < 0 { start } else { end };
                let recover = if pos == anchor {
                    None
                } else {
                    Some(Recover {
                        index: i,
                        offset: pos - start,
                    })
                };
                let mut del = if pos == start {
                    DEL_AFTER
                } else if pos == end {
                    DEL_BEFORE
                } else {
                    DEL_ACROSS
                };
                let lost = if assoc < 0 { pos != start } else { pos != end };
                if lost {
                    del |= DEL_SIDE;
                }
                return MapResult {
                    pos: mapped,
                    del,
                    recover,
                };
            }
            diff += new_size as isize - old_size as isize;
        }
        MapResult {
            pos: usize::try_from(pos as isize + diff).unwrap_or(0),
            del: 0,
            recover: None,
        }
    }
}

/// A sequence of [`StepMap`]s, mapped through in order.
///
/// `mirror` records which maps are each other's inverses. That is what lets a
/// position deleted by one step and restored by its inverse come back to where
/// it started instead of collapsing to the edge of the deletion — the case
/// that makes undo put the cursor back, and makes a rebase preserve a
/// collaborator's selection.
#[derive(Debug, Clone, Default)]
pub struct Mapping {
    maps: Vec<StepMap>,
    /// Pairs of indices into `maps` that invert one another.
    mirror: Vec<(usize, usize)>,
    from: usize,
    to: usize,
}

impl Mapping {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn from_maps(maps: Vec<StepMap>) -> Self {
        let to = maps.len();
        Self {
            maps,
            mirror: Vec::new(),
            from: 0,
            to,
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.maps.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.from == self.to
    }

    #[must_use]
    pub fn maps(&self) -> &[StepMap] {
        &self.maps
    }

    /// A view of part of this mapping. Shares nothing — the maps are cloned —
    /// but they are small.
    #[must_use]
    pub fn slice(&self, from: usize, to: usize) -> Self {
        Self {
            maps: self.maps.clone(),
            mirror: self.mirror.clone(),
            from,
            to,
        }
    }

    /// Adds a map, optionally recording which existing map it inverts.
    pub fn append_map(&mut self, map: StepMap, mirror: Option<usize>) {
        self.maps.push(map);
        self.to = self.maps.len();
        if let Some(mirror) = mirror {
            self.mirror.push((self.maps.len() - 1, mirror));
        }
    }

    /// Adds every map of `other`, keeping its mirror pairs.
    pub fn append_mapping(&mut self, other: &Self) {
        let base = self.maps.len();
        for (i, map) in other.maps.iter().enumerate() {
            let mirror = other.get_mirror(i).filter(|m| *m < i).map(|m| m + base);
            self.append_map(map.clone(), mirror);
        }
    }

    /// Adds `other`'s maps inverted and in reverse — what undo does with the
    /// mapping of the change it is about to take back.
    pub fn append_mapping_inverted(&mut self, other: &Self) {
        let mut i = other.maps.len();
        let base = self.maps.len() + other.maps.len() - 1;
        while i > 0 {
            i -= 1;
            let mirror = other
                .get_mirror(i)
                .filter(|m| *m > i)
                .map(|m| base - m);
            self.append_map(other.maps[i].invert(), mirror);
        }
    }

    /// The whole mapping, inverted.
    #[must_use]
    pub fn invert(&self) -> Self {
        let mut inverse = Self::new();
        inverse.append_mapping_inverted(self);
        inverse
    }

    /// The map that inverts the one at `n`, if this mapping knows of one.
    #[must_use]
    pub fn get_mirror(&self, n: usize) -> Option<usize> {
        self.mirror.iter().find_map(|(a, b)| {
            if *a == n {
                Some(*b)
            } else if *b == n {
                Some(*a)
            } else {
                None
            }
        })
    }

    /// Records that the maps at `n` and `m` invert one another.
    pub fn set_mirror(&mut self, n: usize, m: usize) {
        self.mirror.push((n, m));
    }

    /// Maps a position through every map in turn.
    #[must_use]
    pub fn map(&self, pos: usize, assoc: i32) -> usize {
        let mut pos = pos;
        let mut i = self.from;
        while i < self.to {
            if let Some(recovered) = self.step_through(i, &mut pos, assoc) {
                i = recovered + 1;
                continue;
            }
            i += 1;
        }
        pos
    }

    /// Maps a position, accumulating what happened to it along the way.
    #[must_use]
    pub fn map_result(&self, pos: usize, assoc: i32) -> MapResult {
        let mut pos = pos;
        let mut del = 0u8;
        let mut i = self.from;
        while i < self.to {
            let result = self.maps[i].map_result(pos, assoc);
            if let Some(recover) = result.recover
                && let Some(corr) = self.get_mirror(i)
                && corr > i
                && corr < self.to
            {
                // The deletion is undone later in this same mapping. Put the
                // position back rather than leaving it at the hole's edge.
                pos = self.maps[corr].recover(recover);
                i = corr + 1;
                continue;
            }
            del |= result.del;
            pos = result.pos;
            i += 1;
        }
        MapResult {
            pos,
            del,
            recover: None,
        }
    }

    /// One map of [`Mapping::map`], returning the index jumped to when a
    /// mirrored map recovered the position.
    fn step_through(&self, i: usize, pos: &mut usize, assoc: i32) -> Option<usize> {
        let result = self.maps[i].map_result(*pos, assoc);
        if let Some(recover) = result.recover
            && let Some(corr) = self.get_mirror(i)
            && corr > i
            && corr < self.to
        {
            *pos = self.maps[corr].recover(recover);
            return Some(corr);
        }
        *pos = result.pos;
        None
    }
}
