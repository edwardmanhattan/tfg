//! Group authority (map #70, spec #76): who may command what.
//!
//! Authority is rank-derived: a commander's authority over a ship is the
//! highest rank covering it — unit-direct command is rank 0, a covering
//! group contributes its kind rank, deeper outranks shallower. The local
//! operator is the organizer and commands all (authority sent on commands
//! stays [`Authority::ORGANIZER`]); this model answers who else may
//! command what. None means outside jurisdiction.

use std::collections::{HashMap, HashSet};

use super::Groups;
use crate::command::Authority;

impl Groups {
    /// Every unit under a user's command: directly commanded units plus
    /// all units under their commanded groups, descended through children.
    pub fn commanded_units(
        &self,
        user: &str,
        unit_commander: &HashMap<String, String>,
    ) -> HashSet<String> {
        let mut out: HashSet<String> = unit_commander
            .iter()
            .filter(|(_, c)| c.as_str() == user)
            .map(|(u, _)| u.clone())
            .collect();
        for g in &self.groups {
            if g.commander.as_deref() == Some(user) {
                out.extend(self.group_units(&g.id));
            }
        }
        out
    }

    /// A user's authority over a ship: the highest applicable rank.
    /// None when outside jurisdiction.
    pub fn authority(
        &self,
        user: &str,
        ship: &str,
        unit_commander: &HashMap<String, String>,
    ) -> Option<Authority> {
        let mut level: Option<Authority> = None;
        if unit_commander.get(ship).map(|c| c.as_str()) == Some(user) {
            level = Some(Authority::UNIT);
        }
        for g in &self.groups {
            if g.commander.as_deref() == Some(user)
                && self.group_units(&g.id).iter().any(|u| u == ship)
            {
                let a = Authority::from_rank(g.kind.rank());
                level = Some(level.map_or(a, |b: Authority| b.max(a)));
            }
        }
        level
    }
}
