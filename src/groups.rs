//! Session groups (slice iii, task #33): Satgas of units, Gugus of Satgas.
//!
//! Organizer-built in Setup with roster commanders; one player may command
//! several scopes. Jurisdiction: a commander's units are their directly
//! commanded units plus every unit under their commanded Satgas/Gugus.
//! The local operator is the organizer and commands all (authority sent
//! on commands stays [`Authority::ORGANIZER`]); this model answers who
//! else may command what (scoping in slice iv, zones here).

use std::collections::{HashMap, HashSet};

use crate::command::Authority;

/// A group of units under one commander.
#[derive(Debug, Clone)]
pub struct Satgas {
    pub id: String,
    pub name: String,
    pub units: Vec<String>,
    pub commander: Option<String>,
}

/// A group of Satgas under one commander.
#[derive(Debug, Clone)]
pub struct Gugus {
    pub id: String,
    pub name: String,
    pub satgas: Vec<String>,
    pub commander: Option<String>,
}

/// The session's group hierarchy.
#[derive(Debug, Clone, Default)]
pub struct Groups {
    satgas: Vec<Satgas>,
    gugus: Vec<Gugus>,
}

impl Groups {
    /// Add a Satgas. Fails loud: blank/dup name or id, or a unit already
    /// mustered in another Satgas (jurisdiction stays unambiguous).
    pub fn add_satgas(
        &mut self,
        id: String,
        name: String,
        units: Vec<String>,
        commander: Option<String>,
    ) -> Result<(), String> {
        if name.trim().is_empty() {
            return Err("satgas needs a name".into());
        }
        if self.satgas.iter().any(|s| s.id == id) {
            return Err(format!("duplicate satgas id `{id}`"));
        }
        if self.satgas.iter().any(|s| s.name == name) {
            return Err(format!("duplicate satgas name `{name}`"));
        }
        for u in &units {
            if let Some(owner) = self.satgas.iter().find(|s| s.units.iter().any(|m| m == u)) {
                return Err(format!("unit `{u}` already in satgas `{}`", owner.name));
            }
        }
        self.satgas.push(Satgas { id, name, units, commander });
        Ok(())
    }

    /// Remove a Satgas; Gugus references to it are stripped with it.
    pub fn remove_satgas(&mut self, id: &str) {
        self.satgas.retain(|s| s.id != id);
        for g in &mut self.gugus {
            g.satgas.retain(|s| s != id);
        }
    }

    /// Add a Gugus over existing Satgas. Fails loud: blank/dup name or
    /// id, or a referenced Satgas that does not exist.
    pub fn add_gugus(
        &mut self,
        id: String,
        name: String,
        satgas: Vec<String>,
        commander: Option<String>,
    ) -> Result<(), String> {
        if name.trim().is_empty() {
            return Err("gugus needs a name".into());
        }
        if self.gugus.iter().any(|g| g.id == id) {
            return Err(format!("duplicate gugus id `{id}`"));
        }
        if self.gugus.iter().any(|g| g.name == name) {
            return Err(format!("duplicate gugus name `{name}`"));
        }
        for s in &satgas {
            if !self.satgas.iter().any(|x| &x.id == s) {
                return Err(format!("unknown satgas `{s}`"));
            }
        }
        self.gugus.push(Gugus { id, name, satgas, commander });
        Ok(())
    }

    /// Remove a Gugus.
    pub fn remove_gugus(&mut self, id: &str) {
        self.gugus.retain(|g| g.id != id);
    }

    pub fn satgas_list(&self) -> &[Satgas] {
        &self.satgas
    }

    pub fn gugus_list(&self) -> &[Gugus] {
        &self.gugus
    }

    /// The Satgas mustering a unit, if any.
    pub fn satgas_of_unit(&self, unit: &str) -> Option<&Satgas> {
        self.satgas.iter().find(|s| s.units.iter().any(|u| u == unit))
    }

    /// Member units of one Satgas.
    pub fn satgas_units(&self, id: &str) -> Vec<String> {
        self.satgas
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.units.clone())
            .unwrap_or_default()
    }

    /// All units under a Gugus, descended through its Satgas.
    pub fn gugus_units(&self, id: &str) -> Vec<String> {
        self.gugus
            .iter()
            .find(|g| g.id == id)
            .map(|g| g.satgas.iter().flat_map(|s| self.satgas_units(s)).collect())
            .unwrap_or_default()
    }

    /// Every unit under a user's command: directly commanded units plus
    /// all units under their commanded Satgas/Gugus (grill #19 fan-out
    /// scope; `unit_commander` is the slice-ii seat draft map).
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
        for s in &self.satgas {
            if s.commander.as_deref() == Some(user) {
                out.extend(s.units.iter().cloned());
            }
        }
        for g in &self.gugus {
            if g.commander.as_deref() == Some(user) {
                out.extend(self.gugus_units(&g.id));
            }
        }
        out
    }

    /// A user's authority over a ship: the highest applicable level
    /// (Gugus > Satgas > unit, grill #19). None when outside jurisdiction.
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
        if self.satgas_of_unit(ship).and_then(|s| s.commander.as_deref()) == Some(user) {
            level = Some(Authority::SATGAS);
        }
        for g in &self.gugus {
            if g.commander.as_deref() == Some(user) && self.gugus_units(&g.id).iter().any(|u| u == ship) {
                level = Some(Authority::GUGUS);
            }
        }
        level
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rig() -> (Groups, HashMap<String, String>) {
        let mut g = Groups::default();
        g.add_satgas("s1".into(), "Satgas A".into(), vec!["u1".into(), "u2".into()], Some("ani".into()))
            .unwrap();
        g.add_satgas("s2".into(), "Satgas B".into(), vec!["u3".into()], Some("budi".into())).unwrap();
        g.add_gugus("g1".into(), "Gugus X".into(), vec!["s1".into(), "s2".into()], Some("caca".into()))
            .unwrap();
        let mut seats = HashMap::new();
        seats.insert("u1".into(), "ani".into());
        (g, seats)
    }

    #[test]
    fn jurisdiction_descends_through_gugus() {
        let (g, seats) = rig();
        let caca = g.commanded_units("caca", &seats);
        assert_eq!(caca, ["u1", "u2", "u3"].into_iter().map(String::from).collect());
        let ani = g.commanded_units("ani", &seats);
        assert_eq!(ani, ["u1", "u2"].into_iter().map(String::from).collect());
        assert!(g.commanded_units("nobody", &seats).is_empty());
    }

    #[test]
    fn authority_takes_highest_level() {
        let (g, seats) = rig();
        assert_eq!(g.authority("caca", "u1", &seats), Some(Authority::GUGUS));
        // ani commands u1 directly AND its satgas: satgas wins.
        assert_eq!(g.authority("ani", "u1", &seats), Some(Authority::SATGAS));
        assert_eq!(g.authority("budi", "u3", &seats), Some(Authority::SATGAS));
        assert_eq!(g.authority("ani", "u3", &seats), None);
    }

    #[test]
    fn unit_in_two_satgas_is_rejected() {
        let (mut g, _) = rig();
        let err = g
            .add_satgas("s3".into(), "Satgas C".into(), vec!["u1".into()], None)
            .unwrap_err();
        assert!(err.contains("already in satgas"), "{err}");
    }

    #[test]
    fn gugus_over_unknown_satgas_is_rejected() {
        let (mut g, _) = rig();
        let err = g.add_gugus("g2".into(), "Gugus Y".into(), vec!["nope".into()], None).unwrap_err();
        assert!(err.contains("unknown satgas"), "{err}");
    }

    #[test]
    fn remove_satgas_strips_gugus_refs() {
        let (mut g, _) = rig();
        g.remove_satgas("s1");
        assert!(g.satgas_of_unit("u1").is_none());
        assert_eq!(g.gugus_units("g1"), vec!["u3".to_string()]);
    }
}
