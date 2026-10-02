//! A symbol is COMPOSED, not chosen — affiliation, battle dimension,
//! status, one icon, a set of qualifying modifiers, and an echelon.
//! This module is that model, and it depends on nothing.
//!
//! Nothing means nothing: no egui, no rusqlite, no maplibre. That is the
//! whole reason [`Affiliation`] moved here from `store` and
//! [`BattleDimension`] from `map_render`, and it is what lets
//! `proto/p5-epaint` compile this module and exercise the icon generator
//! in about two seconds, without a graphics context or a database. A
//! symbology core that reached through `store` for a domain enum would
//! drag a multi-gigabyte C++ build behind every vocabulary review.
//!
//! [`icons`]: the geometry primitive, the em box, the em-to-pixel affine.
//! [`generate`]: the manifest parser, the readability invariants, the
//! code emitter. Reads `assets/symbology/icons.tsv`.

pub mod generate;
pub mod icons;
mod icons_generated;
pub mod svgpath;

#[allow(unused_imports)]
pub use icons::{BOX_PX, EM_BOX};
#[allow(unused_imports)]
pub use icons_generated::{FIT, GEOMETRY, UnitIcon};

/// What a thing is in relation to the operator watching the map.
///
/// MIL-STD-2525 calls this standard identity (APP-6 calls it
/// affiliation); the four states are exactly 2525's four basic
/// categories. It is stated RELATIVE TO THE OPERATOR: blue is *your*
/// side, which is what makes "our branch" a stable fact across
/// exercises while "that branch is hostile" is a fact about one.
///
/// 2525D's further states — *assumed friend*, *exercise/pending* — are
/// deliberately NOT adopted. The client has no affiliation source at
/// all, so an assumed-friend confidence judgement would be invented,
/// and a fifth state to maintain per unit buys nothing observable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Affiliation {
    /// The default, and the answer for most units on a first run. A
    /// yellow quatrefoil says "nobody has said", which is true.
    Unknown = 0,
    Friendly = 1,
    Neutral = 2,
    Hostile = 3,
}

impl Affiliation {
    pub const ALL: [Affiliation; 4] = [
        Affiliation::Unknown,
        Affiliation::Friendly,
        Affiliation::Neutral,
        Affiliation::Hostile,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Affiliation::Unknown => "unknown",
            Affiliation::Friendly => "friendly",
            Affiliation::Neutral => "neutral",
            Affiliation::Hostile => "hostile",
        }
    }

    /// Operator-facing wording. The UI says "Side"; the code and the
    /// glossary keep Affiliation, per the vocabulary audit.
    pub fn label(self) -> &'static str {
        match self {
            Affiliation::Unknown => "Unknown",
            Affiliation::Friendly => "Friendly",
            Affiliation::Neutral => "Neutral",
            Affiliation::Hostile => "Hostile",
        }
    }

    /// A stale ordinal from a future or corrupt row resolves to
    /// Unknown — the honest default, and never a panic.
    pub fn from_ordinal(v: i32) -> Affiliation {
        Affiliation::ALL
            .iter()
            .copied()
            .find(|a| *a as i32 == v)
            .unwrap_or(Affiliation::Unknown)
    }
}

/// Where an object operates, which this symbology carries in the
/// frame's OPENNESS rather than in a fourth shape.
///
/// "Closed frames are used to denote the land and sea surface
/// dimensions, frames open at the bottom denote the air/space
/// dimension, and frames open at the top denote the subsurface
/// dimension." Getting this wrong is not cosmetic: a closed frame on a
/// helicopter says it is a land unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BattleDimension {
    /// Land and sea surface: a closed frame. One variant for both,
    /// because the standard draws them identically — which also means an
    /// icon we cannot classify finely still gets the right frame.
    LandAndSeaSurface,
    /// Air and space: the bottom edge is not drawn.
    AirAndSpace,
    /// Subsurface: the top edge is not drawn.
    Subsurface,
}

/// Whether the thing is there or planned to be, which the frame
/// carries as SOLID against DASHED rather than as a fifth shape.
///
/// APP-6C's planned/anticipated symbols draw the same frame and the
/// same icon in a lighter weight, so this is a paint parameter rather
/// than an identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Present,
    Planned,
}

/// A glyph that only ever qualifies another icon. Never an icon alone:
/// Airborne Infantry is one symbol, so the modifier is a property of the
/// composition and not a member of the vocabulary.
///
/// The discriminants are BITS and are pinned. `Modifiers` stores them
/// as a `u8` set, so inserting a variant in the middle would silently
/// re-decode every symbol already persisted against it — the same
/// hazard as renumbering an unpinned `#[repr]` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Modifier {
    /// Open-bottom chevron.
    Airborne = 1,
    /// Flying wing.
    Airmobile = 2,
    /// Squiggle.
    Amphibious = 4,
    /// Flat line.
    Motorized = 8,
    /// Triangle.
    Mountain = 16,
    /// Short vertical stroke: a cannon or gun system is fitted.
    GunSystem = 32,
    /// Three circles: wheeled and cross-country capable.
    WheeledCrossCountry = 64,
}

/// The qualifying glyphs a symbol carries, as a SET and not a sequence.
///
/// The standard composes Infantry + Mechanized + Wheeled into ONE
/// symbol, and the order the glyphs are drawn in is a constant of the
/// layout rather than a property of the value. Two symbols with the
/// same members are the same symbol whichever order they were assembled
/// in, which is why this cannot be a `Vec`.
///
/// `Modifiers` is opaque: a bit is only ever read through
/// [`Modifier`] and only ever set through [`Modifiers::with`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Modifiers(u8);

impl Modifiers {
    /// No qualifying glyph. The overwhelmingly common case: most units
    /// carry no modifier at all.
    pub const NONE: Modifiers = Modifiers(0);

    pub fn contains(self, m: Modifier) -> bool {
        self.0 & (m as u8) != 0
    }

    /// Idempotent, and deliberately so: adding a modifier twice is not
    /// an error the caller has to pre-check for.
    pub fn with(self, m: Modifier) -> Modifiers {
        Modifiers(self.0 | (m as u8))
    }
}

/// The formation-size indicator drawn ABOVE the frame.
///
/// Ordered ascending by size, because the ladder is the point: a caller
/// that needs "bigger than" is comparing, and `None` sorting first is
/// what makes a plain `>` mean it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Echelon {
    /// No indicator. The honest value for a `Unit`: one named hull with
    /// a hull number is a platform, not a formation, and three ticks
    /// over KRI Cakra would be a falsehood rather than a default.
    None,
    /// One dot.
    Team,
    /// Two dots.
    Section,
    /// Three dots.
    Platoon,
    /// One vertical tick.
    Company,
    /// Two vertical ticks.
    Battalion,
    /// Three vertical ticks.
    Regiment,
    /// One X.
    Brigade,
    /// Two Xs.
    Division,
    /// Three Xs.
    Corps,
    /// Four Xs.
    ArmyGroup,
}

/// The icon and everything that qualifies it: the part of a symbol the
/// frame's shape does not already carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Body {
    pub icon: UnitIcon,
    pub modifiers: Modifiers,
    pub echelon: Echelon,
}

/// A complete, paintable identity.
///
/// All four parts, because they answer four different questions and none
/// of them implies another: who it is to us, where it operates, whether
/// it is there, and what it is. The frame carries the first three by
/// shape, opening and weight; the icon carries the fourth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Symbol {
    pub affiliation: Affiliation,
    pub dimension: BattleDimension,
    pub status: Status,
    pub body: Body,
}