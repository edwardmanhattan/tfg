// GENERATED. Source of truth is assets/symbology/icons.tsv; regenerate with
// `cargo run --manifest-path proto/p5-epaint/Cargo.toml -- --symbology-generate`.
//
// The invariants encoded here are the reason this file is generated rather
// than written: a hand edit is exactly how they stop holding.

use super::icons::{Fit, IconMark};
use super::BattleDimension;

/// One icon of the standard's vocabulary.
///
/// Discriminants are PINNED and follow the manifest's row order. Reordering
/// the manifest renumbers them, which silently repaints every symbol already
/// persisted against the old numbering, so the generator refuses to emit a
/// reordered table rather than letting it pass as a refactor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum UnitIcon {
    #[allow(non_camel_case_types)]
    Naval = 0,
    #[allow(non_camel_case_types)]
    Infantry = 1,
    #[allow(non_camel_case_types)]
    Armour = 2,
    #[allow(non_camel_case_types)]
    Artillery = 3,
    #[allow(non_camel_case_types)]
    RotaryWing = 4,
    #[allow(non_camel_case_types)]
    FixedWing = 5,
    #[allow(non_camel_case_types)]
    Missile = 6,
    #[allow(non_camel_case_types)]
    Unspecified = 7,
}

impl UnitIcon {
    pub const ALL: [UnitIcon; 8] = [
        UnitIcon::Naval,
        UnitIcon::Infantry,
        UnitIcon::Armour,
        UnitIcon::Artillery,
        UnitIcon::RotaryWing,
        UnitIcon::FixedWing,
        UnitIcon::Missile,
        UnitIcon::Unspecified,
    ];

    /// The manifest's variant name: the stable key a persisted symbol refers
/// to. Distinct from [`UnitIcon::name`], which is the operator-facing string.
    pub fn variant_name(self) -> &'static str {
        match self {
            UnitIcon::Naval => "naval",
            UnitIcon::Infantry => "infantry",
            UnitIcon::Armour => "armour",
            UnitIcon::Artillery => "artillery",
            UnitIcon::RotaryWing => "rotary_wing",
            UnitIcon::FixedWing => "fixed_wing",
            UnitIcon::Missile => "missile",
            UnitIcon::Unspecified => "unspecified",
        }
    }

    /// Operator-facing display name.
    pub fn name(self) -> &'static str {
        match self {
            UnitIcon::Naval => "Naval",
            UnitIcon::Infantry => "Infantry",
            UnitIcon::Armour => "Armour",
            UnitIcon::Artillery => "Artillery",
            UnitIcon::RotaryWing => "Rotary wing aviation",
            UnitIcon::FixedWing => "Fixed wing aviation",
            UnitIcon::Missile => "Missile",
            UnitIcon::Unspecified => "Unspecified",
        }
    }

    /// The dimension an icon of this role draws in, from the manifest's SIDC
/// letter collapsed the way this symbology collapses it.
    pub fn default_dimension(self) -> BattleDimension {
        match self {
            UnitIcon::Naval => BattleDimension::LandAndSeaSurface,
            UnitIcon::Infantry => BattleDimension::LandAndSeaSurface,
            UnitIcon::Armour => BattleDimension::LandAndSeaSurface,
            UnitIcon::Artillery => BattleDimension::LandAndSeaSurface,
            UnitIcon::RotaryWing => BattleDimension::AirAndSpace,
            UnitIcon::FixedWing => BattleDimension::AirAndSpace,
            UnitIcon::Missile => BattleDimension::LandAndSeaSurface,
            UnitIcon::Unspecified => BattleDimension::LandAndSeaSurface,
        }
    }
}

/// Each icon's marks, flattened and already in the unit square. Indexed by
/// [`UnitIcon`] as a `u8`.
pub const GEOMETRY: [&[IconMark]; 8] = [
    &[
        IconMark::Stroke(&[(0.500000, 0.000000), (0.574951, 0.031006), (0.605957, 0.105957), (0.574951, 0.180664), (0.500000, 0.211670), (0.425049, 0.180664), (0.394043, 0.105957), (0.425049, 0.031006), (0.500000, 0.000000)]),
        IconMark::Stroke(&[(0.500000, 0.211670), (0.500000, 0.882324)]),
        IconMark::Stroke(&[(0.264648, 0.353027), (0.735352, 0.353027)]),
        IconMark::Stroke(&[(0.370605, 0.882324), (0.629395, 0.882324)]),
        IconMark::Stroke(&[(0.370605, 0.882324), (0.205811, 1.000000), (0.205811, 0.835205)]),
        IconMark::Stroke(&[(0.629395, 0.882324), (0.794189, 1.000000), (0.794189, 0.835205)]),
    ],
    &[
        IconMark::Stroke(&[(-0.000000, -0.000000), (1.000000, 1.000000)]),
        IconMark::Stroke(&[(-0.000000, 1.000000), (1.000000, -0.000000)]),
    ],
    &[
        IconMark::Fill(&[(-0.000000, 0.166748), (1.000000, 0.166748), (1.000000, 0.300049), (-0.000000, 0.300049)]),
        IconMark::Fill(&[(-0.000000, 0.699951), (1.000000, 0.699951), (1.000000, 0.833252), (-0.000000, 0.833252)]),
    ],
    &[
        IconMark::Fill(&[(0.500000, -0.000000), (0.750000, 0.066895), (0.933105, 0.250000), (1.000000, 0.500000), (0.933105, 0.750000), (0.750000, 0.933105), (0.500000, 1.000000), (0.250000, 0.933105), (0.066895, 0.750000), (-0.000000, 0.500000), (0.066895, 0.250000), (0.250000, 0.066895), (0.500000, -0.000000)]),
    ],
    &[
        IconMark::Fill(&[(0.500000, 0.252686), (0.674805, 0.325195), (0.747314, 0.500000), (0.674805, 0.674805), (0.500000, 0.747314), (0.325195, 0.674805), (0.252686, 0.500000), (0.325195, 0.325195), (0.500000, 0.252686)]),
        IconMark::Stroke(&[(0.650146, 0.650146), (1.000000, 1.000000)]),
        IconMark::Stroke(&[(0.349854, 0.650146), (-0.000000, 1.000000)]),
        IconMark::Stroke(&[(0.349854, 0.349854), (-0.000000, -0.000000)]),
        IconMark::Stroke(&[(0.650146, 0.349854), (1.000000, -0.000000)]),
    ],
    &[
        IconMark::Fill(&[(0.500000, 0.362549), (0.597168, 0.402832), (0.637451, 0.500000), (0.597168, 0.597168), (0.500000, 0.637451), (0.402832, 0.597168), (0.362549, 0.500000), (0.402832, 0.402832), (0.500000, 0.362549)]),
        IconMark::Stroke(&[(0.500000, 0.375000), (0.500000, 0.000000)]),
        IconMark::Stroke(&[(0.500000, 0.625000), (0.500000, 1.000000)]),
    ],
    &[
        IconMark::Fill(&[(0.500000, -0.000000), (0.617676, 0.220703), (0.617676, 1.000000), (0.382324, 1.000000), (0.382324, 0.220703)]),
        IconMark::Fill(&[(0.382324, 0.720703), (0.058838, 1.000000), (0.382324, 1.000000)]),
        IconMark::Fill(&[(0.617676, 0.720703), (0.941162, 1.000000), (0.617676, 1.000000)]),
    ],
    &[],
];

/// Each icon's em-to-unit-square fit, for callers that need to reason about an
/// icon's margins rather than draw it. `scale` is UNIFORM: a per-axis scale
/// would turn the infantry saltire into the hostile frame's rhombus. The
/// geometry above is already normalised, so drawing never needs this.
pub const FIT: [Fit; 8] = [
    Fit { scale: 0.001221, dx: 0.500000, dy: 0.529297 },
    Fit { scale: 0.001221, dx: 0.500000, dy: 0.500000 },
    Fit { scale: 0.001221, dx: 0.500000, dy: 0.500000 },
    Fit { scale: 0.001953, dx: 0.500000, dy: 0.500000 },
    Fit { scale: 0.001709, dx: 0.500000, dy: 0.500000 },
    Fit { scale: 0.001221, dx: 0.500000, dy: 0.500000 },
    Fit { scale: 0.001465, dx: 0.500000, dy: 0.632324 },
    Fit { scale: 1.000000, dx: 0.000000, dy: 0.000000 },
];
