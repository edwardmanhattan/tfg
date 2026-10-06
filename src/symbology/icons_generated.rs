// GENERATED. Do not hand-edit; regenerate with
// `cargo run --manifest-path proto/p5-epaint/Cargo.toml -- --symbology-generate`.
//
// Which icons exist and what they are called is assets/symbology/icons.tsv.
// The coordinates come from assets/symbology/milsymbol.tsv, extracted from
// spatialillusions/milsymbol (MIT, Copyright (c) 2017 Mans Beckman) at a pinned
// commit; see licenses/milsymbol-LICENSE.md and the header of that file.
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
    SurfaceCombatant = 0,
    #[allow(non_camel_case_types)]
    PatrolCraft = 1,
    #[allow(non_camel_case_types)]
    MineWarfare = 2,
    #[allow(non_camel_case_types)]
    AmphibiousWarfareShip = 3,
    #[allow(non_camel_case_types)]
    Carrier = 4,
    #[allow(non_camel_case_types)]
    OilerTanker = 5,
    #[allow(non_camel_case_types)]
    HospitalShip = 6,
    #[allow(non_camel_case_types)]
    Auxiliary = 7,
    #[allow(non_camel_case_types)]
    Convoy = 8,
    #[allow(non_camel_case_types)]
    UnmannedSurface = 9,
    #[allow(non_camel_case_types)]
    Submarine = 10,
    #[allow(non_camel_case_types)]
    Infantry = 11,
    #[allow(non_camel_case_types)]
    Armour = 12,
    #[allow(non_camel_case_types)]
    Artillery = 13,
    #[allow(non_camel_case_types)]
    Reconnaissance = 14,
    #[allow(non_camel_case_types)]
    AirDefence = 15,
    #[allow(non_camel_case_types)]
    Engineer = 16,
    #[allow(non_camel_case_types)]
    Medical = 17,
    #[allow(non_camel_case_types)]
    Signal = 18,
    #[allow(non_camel_case_types)]
    Supply = 19,
    #[allow(non_camel_case_types)]
    AmphibiousGround = 20,
    #[allow(non_camel_case_types)]
    NavalInfantry = 21,
    #[allow(non_camel_case_types)]
    RotaryWing = 22,
    #[allow(non_camel_case_types)]
    FixedWing = 23,
    #[allow(non_camel_case_types)]
    Unspecified = 24,
}

impl UnitIcon {
    pub const ALL: [UnitIcon; 25] = [
        UnitIcon::SurfaceCombatant,
        UnitIcon::PatrolCraft,
        UnitIcon::MineWarfare,
        UnitIcon::AmphibiousWarfareShip,
        UnitIcon::Carrier,
        UnitIcon::OilerTanker,
        UnitIcon::HospitalShip,
        UnitIcon::Auxiliary,
        UnitIcon::Convoy,
        UnitIcon::UnmannedSurface,
        UnitIcon::Submarine,
        UnitIcon::Infantry,
        UnitIcon::Armour,
        UnitIcon::Artillery,
        UnitIcon::Reconnaissance,
        UnitIcon::AirDefence,
        UnitIcon::Engineer,
        UnitIcon::Medical,
        UnitIcon::Signal,
        UnitIcon::Supply,
        UnitIcon::AmphibiousGround,
        UnitIcon::NavalInfantry,
        UnitIcon::RotaryWing,
        UnitIcon::FixedWing,
        UnitIcon::Unspecified,
    ];

    /// The manifest's variant name: the stable key a persisted symbol refers
/// to. Distinct from [`UnitIcon::name`], which is the operator-facing string.
    pub fn variant_name(self) -> &'static str {
        match self {
            UnitIcon::SurfaceCombatant => "surface_combatant",
            UnitIcon::PatrolCraft => "patrol_craft",
            UnitIcon::MineWarfare => "mine_warfare",
            UnitIcon::AmphibiousWarfareShip => "amphibious_warfare_ship",
            UnitIcon::Carrier => "carrier",
            UnitIcon::OilerTanker => "oiler_tanker",
            UnitIcon::HospitalShip => "hospital_ship",
            UnitIcon::Auxiliary => "auxiliary",
            UnitIcon::Convoy => "convoy",
            UnitIcon::UnmannedSurface => "unmanned_surface",
            UnitIcon::Submarine => "submarine",
            UnitIcon::Infantry => "infantry",
            UnitIcon::Armour => "armour",
            UnitIcon::Artillery => "artillery",
            UnitIcon::Reconnaissance => "reconnaissance",
            UnitIcon::AirDefence => "air_defence",
            UnitIcon::Engineer => "engineer",
            UnitIcon::Medical => "medical",
            UnitIcon::Signal => "signal",
            UnitIcon::Supply => "supply",
            UnitIcon::AmphibiousGround => "amphibious_ground",
            UnitIcon::NavalInfantry => "naval_infantry",
            UnitIcon::RotaryWing => "rotary_wing",
            UnitIcon::FixedWing => "fixed_wing",
            UnitIcon::Unspecified => "unspecified",
        }
    }

    /// Operator-facing display name.
    pub fn name(self) -> &'static str {
        match self {
            UnitIcon::SurfaceCombatant => "Surface combatant",
            UnitIcon::PatrolCraft => "Patrol craft",
            UnitIcon::MineWarfare => "Mine warfare vessel",
            UnitIcon::AmphibiousWarfareShip => "Amphibious warfare ship",
            UnitIcon::Carrier => "Carrier",
            UnitIcon::OilerTanker => "Tanker / oiler",
            UnitIcon::HospitalShip => "Hospital ship",
            UnitIcon::Auxiliary => "Auxiliary / cargo hull",
            UnitIcon::Convoy => "Convoy",
            UnitIcon::UnmannedSurface => "Unmanned surface vehicle",
            UnitIcon::Submarine => "Submarine",
            UnitIcon::Infantry => "Infantry",
            UnitIcon::Armour => "Armour",
            UnitIcon::Artillery => "Artillery",
            UnitIcon::Reconnaissance => "Reconnaissance",
            UnitIcon::AirDefence => "Air defence",
            UnitIcon::Engineer => "Engineer",
            UnitIcon::Medical => "Medical",
            UnitIcon::Signal => "Signal",
            UnitIcon::Supply => "Supply",
            UnitIcon::AmphibiousGround => "Amphibious (ground)",
            UnitIcon::NavalInfantry => "Naval (anchor)",
            UnitIcon::RotaryWing => "Rotary wing aviation",
            UnitIcon::FixedWing => "Fixed wing aviation",
            UnitIcon::Unspecified => "Unspecified",
        }
    }

    /// The dimension an icon of this role draws in, from the manifest's SIDC
/// letter collapsed the way this symbology collapses it.
    pub fn default_dimension(self) -> BattleDimension {
        match self {
            UnitIcon::SurfaceCombatant => BattleDimension::LandAndSeaSurface,
            UnitIcon::PatrolCraft => BattleDimension::LandAndSeaSurface,
            UnitIcon::MineWarfare => BattleDimension::LandAndSeaSurface,
            UnitIcon::AmphibiousWarfareShip => BattleDimension::LandAndSeaSurface,
            UnitIcon::Carrier => BattleDimension::LandAndSeaSurface,
            UnitIcon::OilerTanker => BattleDimension::LandAndSeaSurface,
            UnitIcon::HospitalShip => BattleDimension::LandAndSeaSurface,
            UnitIcon::Auxiliary => BattleDimension::LandAndSeaSurface,
            UnitIcon::Convoy => BattleDimension::LandAndSeaSurface,
            UnitIcon::UnmannedSurface => BattleDimension::LandAndSeaSurface,
            UnitIcon::Submarine => BattleDimension::Subsurface,
            UnitIcon::Infantry => BattleDimension::LandAndSeaSurface,
            UnitIcon::Armour => BattleDimension::LandAndSeaSurface,
            UnitIcon::Artillery => BattleDimension::LandAndSeaSurface,
            UnitIcon::Reconnaissance => BattleDimension::LandAndSeaSurface,
            UnitIcon::AirDefence => BattleDimension::LandAndSeaSurface,
            UnitIcon::Engineer => BattleDimension::LandAndSeaSurface,
            UnitIcon::Medical => BattleDimension::LandAndSeaSurface,
            UnitIcon::Signal => BattleDimension::LandAndSeaSurface,
            UnitIcon::Supply => BattleDimension::LandAndSeaSurface,
            UnitIcon::AmphibiousGround => BattleDimension::LandAndSeaSurface,
            UnitIcon::NavalInfantry => BattleDimension::LandAndSeaSurface,
            UnitIcon::RotaryWing => BattleDimension::AirAndSpace,
            UnitIcon::FixedWing => BattleDimension::AirAndSpace,
            UnitIcon::Unspecified => BattleDimension::LandAndSeaSurface,
        }
    }
}

/// Each icon's marks, flattened and already in the unit square. Indexed by
/// [`UnitIcon`] as a `u8`.
pub const GEOMETRY: [&[IconMark]; 25] = [
    &[
        IconMark::Fill(&[(0.500000, 0.899902), (-0.000000, 0.560059), (0.300049, 0.600098), (0.300049, 0.399902), (0.399902, 0.399902), (0.399902, 0.300049), (0.100098, 0.300049), (0.100098, 0.199951), (0.399902, 0.199951), (0.399902, 0.100098), (0.600098, 0.100098), (0.600098, 0.199951), (0.899902, 0.199951), (0.899902, 0.300049), (0.600098, 0.300049), (0.600098, 0.399902), (0.699951, 0.399902), (0.699951, 0.600098), (1.000000, 0.560059), (0.500000, 0.899902)]),
    ],
    &[
        IconMark::Fill(&[(0.000000, 0.500000), (0.500000, 1.000000), (1.000000, 0.500000), (0.750000, 0.500000), (0.750000, 0.000000), (0.250000, 0.000000), (0.250000, 0.500000), (0.000000, 0.500000)]),
    ],
    &[
        IconMark::Fill(&[(0.463135, 0.076172), (0.463135, 0.165283), (0.323975, 0.228271), (0.247803, 0.152100), (0.195557, 0.204346), (0.273926, 0.282715), (0.234863, 0.423828), (0.000000, 0.423828), (0.000000, 0.489014), (0.065186, 0.489014), (0.500000, 0.923828), (0.934814, 0.489014), (1.000000, 0.489014), (1.000000, 0.423828), (0.782715, 0.423828), (0.717285, 0.278320), (0.804443, 0.197754), (0.760986, 0.145752), (0.673828, 0.223877), (0.543457, 0.165283), (0.543457, 0.076172), (0.463135, 0.076172)]),
    ],
    &[
        IconMark::Stroke(&[(0.500000, 1.000000), (1.000000, 1.000000)]),
        IconMark::Fill(&[(0.500000, 1.000000), (0.000000, 0.500000), (0.250000, 0.500000), (0.250000, 0.000000), (0.750000, 0.000000), (0.750000, 0.500000), (1.000000, 0.500000), (0.500000, 1.000000)]),
    ],
    &[
        IconMark::Fill(&[(0.000000, 0.500000), (0.500000, 1.000000), (1.000000, 0.500000), (0.500000, 0.500000), (0.500000, 0.000000), (0.000000, 0.000000), (0.000000, 0.500000)]),
    ],
    &[
        IconMark::Stroke(&[(0.222168, 0.500000), (0.222168, 0.111084), (0.777832, 0.111084), (0.777832, 0.500000), (1.000000, 0.500000), (0.833252, 0.888916), (0.166748, 0.888916), (-0.000000, 0.500000), (0.222168, 0.500000)]),
    ],
    &[
        IconMark::Stroke(&[(0.222168, 0.500000), (0.222168, 0.111084), (0.777832, 0.111084), (0.777832, 0.500000), (1.000000, 0.500000), (0.833252, 0.888916), (0.166748, 0.888916), (-0.000000, 0.500000), (0.222168, 0.500000)]),
        IconMark::Fill(&[(0.444336, 0.444336), (0.444336, 0.277832), (0.555664, 0.277832), (0.555664, 0.444336), (0.722168, 0.444336), (0.722168, 0.555664), (0.555664, 0.555664), (0.555664, 0.722168), (0.444336, 0.722168), (0.444336, 0.555664), (0.277832, 0.555664), (0.277832, 0.444336), (0.444336, 0.444336)]),
    ],
    &[
        IconMark::Stroke(&[(0.222168, 0.500000), (0.222168, 0.111084), (0.777832, 0.111084), (0.777832, 0.500000), (1.000000, 0.500000), (0.833252, 0.888916), (0.166748, 0.888916), (-0.000000, 0.500000), (0.222168, 0.500000)]),
    ],
    &[
        IconMark::Fill(&[(0.250000, 0.718750), (0.000000, 0.718750), (0.000000, 0.281250), (1.000000, 0.281250), (1.000000, 0.718750), (0.750000, 0.718750), (0.750000, 0.468750), (0.250000, 0.468750), (0.250000, 0.718750)]),
    ],
    &[
        IconMark::Fill(&[(0.000000, 0.293701), (0.500000, 0.543701), (1.000000, 0.293701), (1.000000, 0.393799), (0.500000, 0.706299), (0.000000, 0.393799), (0.000000, 0.293701)]),
    ],
    &[
        IconMark::Fill(&[(0.187500, 0.312500), (0.812500, 0.312500), (1.000000, 0.500000), (0.812500, 0.687500), (0.187500, 0.687500), (0.000000, 0.500000), (0.187500, 0.312500)]),
    ],
    &[
        IconMark::Stroke(&[(0.000000, 0.166748), (1.000000, 0.833252)]),
        IconMark::Stroke(&[(0.000000, 0.833252), (1.000000, 0.166748)]),
    ],
    &[
        IconMark::Stroke(&[(0.800049, 0.260010), (1.000000, 0.384521), (1.000000, 0.615479), (0.800049, 0.739990), (0.199951, 0.739990), (-0.000000, 0.615479), (-0.000000, 0.384521), (0.199951, 0.260010), (0.800049, 0.260010)]),
    ],
    &[
        IconMark::Stroke(&[(0.500000, 1.000000), (0.500000, -0.000000)]),
        IconMark::Stroke(&[(0.345459, 0.793701), (0.345459, 0.278320)]),
        IconMark::Stroke(&[(0.654541, 0.793701), (0.654541, 0.278320)]),
        IconMark::Stroke(&[(0.345459, 0.206299), (0.500000, -0.000000), (0.654541, 0.206299)]),
    ],
    &[
        IconMark::Stroke(&[(0.000000, 0.833252), (1.000000, 0.166748)]),
    ],
    &[
        IconMark::Stroke(&[(0.000000, 0.600098), (0.156250, 0.449951), (0.500000, 0.399902), (0.843750, 0.449951), (1.000000, 0.600098)]),
    ],
    &[
        IconMark::Stroke(&[(0.000000, 0.718750), (0.000000, 0.281250), (1.000000, 0.281250), (1.000000, 0.718750)]),
        IconMark::Stroke(&[(0.500000, 0.281250), (0.500000, 0.618652)]),
    ],
    &[
        IconMark::Stroke(&[(0.500000, 0.166748), (0.500000, 0.833252)]),
        IconMark::Stroke(&[(0.000000, 0.500000), (1.000000, 0.500000)]),
    ],
    &[
        IconMark::Stroke(&[(0.000000, 0.166748), (0.500000, 0.566650), (0.500000, 0.433350), (1.000000, 0.833252)]),
    ],
    &[
        IconMark::Stroke(&[(0.000000, 0.500000), (1.000000, 0.500000)]),
    ],
    &[
        IconMark::Stroke(&[(-0.000000, 0.434082), (0.059814, 0.468262), (0.064209, 0.531738), (0.124023, 0.565918), (0.183838, 0.531738), (0.188232, 0.468262), (0.248047, 0.434082), (0.307617, 0.468262), (0.312256, 0.531738), (0.372070, 0.565918), (0.431641, 0.531738), (0.436279, 0.468262), (0.496094, 0.434082), (0.555664, 0.468262), (0.560303, 0.531738), (0.620117, 0.565918), (0.679688, 0.531738), (0.684326, 0.468262), (0.744141, 0.434082), (0.803711, 0.468262), (0.808350, 0.531738), (0.868164, 0.565918), (0.927979, 0.531738), (0.934814, 0.468262), (1.000000, 0.434082)]),
    ],
    &[
        IconMark::Stroke(&[(0.499512, 0.973389), (0.499512, 0.026611)]),
        IconMark::Stroke(&[(0.144287, 0.085693), (0.854492, 0.085693)]),
        IconMark::Fill(&[(-0.000000, 0.505859), (0.000244, 0.630371), (0.033203, 0.767334), (0.087646, 0.736572), (0.362549, 0.942871), (0.498291, 0.968750), (0.501709, 0.968750), (0.637451, 0.942383), (0.912354, 0.736572), (0.966797, 0.767334), (0.999023, 0.630859), (1.000000, 0.505859), (0.920654, 0.581543), (0.829590, 0.635010), (0.875732, 0.692871), (0.739746, 0.819580), (0.505371, 0.894043), (0.501709, 0.945068), (0.498291, 0.894043), (0.263428, 0.819580), (0.127930, 0.692871), (0.174072, 0.635010), (0.082764, 0.581543), (0.003662, 0.505859), (-0.000000, 0.505859)]),
    ],
    &[
        IconMark::Fill(&[(0.000000, 0.312500), (0.500000, 0.500000), (1.000000, 0.312500), (1.000000, 0.687500), (0.500000, 0.500000), (0.000000, 0.687500), (0.000000, 0.312500)]),
    ],
    &[
        IconMark::Fill(&[(0.500000, 0.500000), (0.863525, 0.354492), (1.000000, 0.500000), (0.863525, 0.645508), (0.500000, 0.500000), (0.136475, 0.645508), (-0.000000, 0.500000), (0.136475, 0.354492), (0.500000, 0.500000)]),
    ],
    &[],
];

/// Each icon's em-to-unit-square fit, for callers that need to reason about an
/// icon's margins rather than draw it. `scale` is UNIFORM: a per-axis scale
/// would turn the infantry saltire into the hostile frame's rhombus. The
/// geometry above is already normalised, so drawing never needs this.
pub const FIT: [Fit; 25] = [
    Fit { scale: 0.003906, dx: 0.500000, dy: 0.500000 },
    Fit { scale: 0.004883, dx: 0.500000, dy: 0.500000 },
    Fit { scale: 0.004395, dx: 0.500000, dy: 0.489014 },
    Fit { scale: 0.004883, dx: 0.500000, dy: 0.500000 },
    Fit { scale: 0.004883, dx: 0.500000, dy: 0.500000 },
    Fit { scale: 0.002197, dx: 0.500000, dy: 0.500000 },
    Fit { scale: 0.002197, dx: 0.500000, dy: 0.500000 },
    Fit { scale: 0.002197, dx: 0.500000, dy: 0.500000 },
    Fit { scale: 0.002441, dx: 0.500000, dy: 0.531250 },
    Fit { scale: 0.002441, dx: 0.500000, dy: 0.493652 },
    Fit { scale: 0.002441, dx: 0.500000, dy: 0.500000 },
    Fit { scale: 0.001221, dx: 0.500000, dy: 0.500000 },
    Fit { scale: 0.002441, dx: 0.500000, dy: 0.500000 },
    Fit { scale: 0.001953, dx: 0.500000, dy: 0.484619 },
    Fit { scale: 0.001221, dx: 0.500000, dy: 0.500000 },
    Fit { scale: 0.001221, dx: 0.500000, dy: 0.266602 },
    Fit { scale: 0.002441, dx: 0.500000, dy: 0.493652 },
    Fit { scale: 0.001221, dx: 0.500000, dy: 0.500000 },
    Fit { scale: 0.001221, dx: 0.500000, dy: 0.500000 },
    Fit { scale: 0.001221, dx: 0.500000, dy: 0.366699 },
    Fit { scale: 0.001221, dx: 0.494629, dy: 0.500000 },
    Fit { scale: 0.002441, dx: 0.499512, dy: 0.440918 },
    Fit { scale: 0.002441, dx: 0.500000, dy: 0.500000 },
    Fit { scale: 0.002441, dx: 0.500000, dy: 0.500000 },
    Fit { scale: 1.000000, dx: 0.000000, dy: 0.000000 },
];
