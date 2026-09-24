//! Command model (precedence grill, #19): multi-unit directives above orders.
//!
//! A [`MoveCommand`] is a commander's intent over several units; it fans
//! out into one [`Leg`] per ship, and the sim executes legs as orders.
//! Authority is a placeholder rank until seats land (setup grill, #23):
//! higher jurisdictions override lower ones, loudly, never silently.

use crate::geo::coordinates::GeoPosition;

/// Command authority rank, derived from the commanding scope's level:
/// unit-direct command is 0, group levels carry their [`GroupKind`] rank
/// (Unsur 10 < Satuan Tugas 20 < Gugus 30 < Operasi Gabungan 40, with
/// gaps for middle insertion), the organizer tops all.
/// Higher wins; equal re-applies; lower is refused loudly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Authority(u8);

impl Authority {
    pub const UNIT: Self = Self(0);
    pub const UNSUR: Self = Self(10);
    /// `Satgas` survives only as an alias: the glossary canonicalizes
    /// Satuan Tugas.
    pub const SATGAS: Self = Self(20);
    pub const GUGUS: Self = Self(30);
    pub const OPERASI_GABUNGAN: Self = Self(40);
    pub const ORGANIZER: Self = Self(u8::MAX);

    /// Authority from a group rank (see [`GroupKind::rank`]).
    ///
    /// [`GroupKind::rank`]: crate::groups::GroupKind::rank
    pub const fn from_rank(rank: u8) -> Self {
        Self(rank)
    }

    pub fn rank(self) -> u8 {
        self.0
    }
}

/// Verbs a grant may allow. Starts at the grill-#13 verbs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Move,
    /// Set a persistent heading/speed HelmOrder.
    SetHelm,
    Cancel,
    Hold,
}

/// What bounds a command: an explicit unit list, a game-time expiry, and
/// a verb whitelist. Expiry bars new commands; in-flight orders run out.
#[derive(Debug, Clone)]
pub struct Grant {
    pub units: Vec<String>,
    pub expires_game_secs: u64,
    pub verbs: Vec<Verb>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantDenial {
    Expired,
    OutsideScope,
    VerbDenied,
}

impl Grant {
    pub fn covers(
        &self,
        ship_id: &str,
        verb: Verb,
        now_game_secs: u64,
    ) -> Result<(), GrantDenial> {
        if now_game_secs >= self.expires_game_secs {
            return Err(GrantDenial::Expired);
        }
        if !self.units.iter().any(|u| u == ship_id) {
            return Err(GrantDenial::OutsideScope);
        }
        if !self.verbs.contains(&verb) {
            return Err(GrantDenial::VerbDenied);
        }
        Ok(())
    }
}

/// One fanned-out leg: the per-ship directive the sim executes.
#[derive(Debug, Clone)]
pub struct Leg {
    pub ship_id: String,
    pub waypoint: GeoPosition,
    pub speed_kn: f32,
}

/// A multi-unit move: explicit per-unit legs plus an optional group-wide
/// default speed. Fan-out is a pure mapping; validation (land, class max)
/// stays in the sim, applied per leg exactly like single-ship orders.
#[derive(Debug, Clone)]
pub struct MoveCommand {
    pub legs: Vec<Leg>,
    pub default_speed_kn: Option<f32>,
    pub authority: Authority,
    pub grant: Grant,
}

impl MoveCommand {
    /// Explicit per-unit legs with the default speed filled in where a
    /// leg carries a non-positive speed.
    pub fn fan_out(&self) -> Vec<Leg> {
        self.legs
            .iter()
            .map(|l| Leg {
                ship_id: l.ship_id.clone(),
                waypoint: l.waypoint,
                speed_kn: if l.speed_kn > 0.0 {
                    l.speed_kn
                } else {
                    self.default_speed_kn.unwrap_or(0.0)
                },
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wp() -> GeoPosition {
        GeoPosition { latitude: -5.92, longitude: 106.95 }
    }

    fn grant() -> Grant {
        Grant {
            units: vec!["a".into(), "b".into()],
            expires_game_secs: 1000,
            verbs: vec![Verb::Move],
        }
    }

    #[test]
    fn fan_out_fills_default_speed() {
        let cmd = MoveCommand {
            legs: vec![
                Leg { ship_id: "a".into(), waypoint: wp(), speed_kn: 12.0 },
                Leg { ship_id: "b".into(), waypoint: wp(), speed_kn: 0.0 },
            ],
            default_speed_kn: Some(15.0),
            authority: Authority::SATGAS,
            grant: grant(),
        };
        let legs = cmd.fan_out();
        assert_eq!(legs[0].speed_kn, 12.0, "explicit speed wins");
        assert_eq!(legs[1].speed_kn, 15.0, "default fills in");
    }

    #[test]
    fn authority_orders_by_jurisdiction() {
        assert!(Authority::UNIT < Authority::UNSUR);
        assert!(Authority::UNSUR < Authority::SATGAS);
        assert!(Authority::SATGAS < Authority::GUGUS);
        assert!(Authority::GUGUS < Authority::OPERASI_GABUNGAN);
        assert!(Authority::OPERASI_GABUNGAN < Authority::ORGANIZER);
        assert_eq!(Authority::from_rank(crate::groups::GroupKind::Gugus.rank()), Authority::GUGUS);
    }

    #[test]
    fn grant_checks_time_then_scope_then_verb() {
        let g = grant();
        assert_eq!(g.covers("a", Verb::Move, 1001), Err(GrantDenial::Expired));
        assert_eq!(g.covers("zzz", Verb::Move, 10), Err(GrantDenial::OutsideScope));
        assert_eq!(g.covers("a", Verb::Cancel, 10), Err(GrantDenial::VerbDenied));
        assert!(g.covers("a", Verb::Move, 10).is_ok());
    }
}
