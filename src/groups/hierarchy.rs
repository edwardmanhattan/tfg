//! Group hierarchy (map #70, spec #76): build and query the forest.
//!
//! Groups are created bottom-up over existing children; every child must
//! sit at a strictly lower rank than its parent, which makes cycles
//! impossible by construction. Everything fails loud: blank/dup names
//! and ids, double-mustered units, unknown or mis-ranked children.

use std::collections::HashSet;

use super::model::{Group, GroupKind};
use super::Groups;

impl Groups {
    /// Add a group. Fails loud: blank/dup name or id, a unit already
    /// mustered in another group (jurisdiction stays unambiguous), an
    /// unknown child, or a child at or above this group's own rank.
    pub fn add_group(
        &mut self,
        id: String,
        name: String,
        kind: GroupKind,
        units: Vec<String>,
        children: Vec<String>,
        commander: Option<String>,
    ) -> Result<(), String> {
        if name.trim().is_empty() {
            return Err("group needs a name".into());
        }
        if self.groups.iter().any(|g| g.id == id) {
            return Err(format!("duplicate group id `{id}`"));
        }
        if self.groups.iter().any(|g| g.name == name) {
            return Err(format!("duplicate group name `{name}`"));
        }
        let mut seen: HashSet<&str> = HashSet::new();
        for u in &units {
            if !seen.insert(u.as_str()) {
                return Err(format!("unit `{u}` listed twice"));
            }
            if let Some(owner) = self.groups.iter().find(|g| g.units.iter().any(|m| m == u)) {
                return Err(format!("unit `{u}` already in group `{}`", owner.name));
            }
        }
        for c in &children {
            if c == &id {
                return Err(format!("group `{id}` cannot contain itself"));
            }
            let Some(child) = self.groups.iter().find(|g| &g.id == c) else {
                return Err(format!("unknown group `{c}`"));
            };
            if child.kind.rank() >= kind.rank() {
                return Err(format!(
                    "rank violation: child `{}` ({}:{}) not below {}:{}",
                    child.name,
                    child.kind.label(),
                    child.kind.rank(),
                    kind.label(),
                    kind.rank()
                ));
            }
        }
        self.groups.push(Group { id, name, kind, units, children, commander });
        Ok(())
    }

    /// Remove a group; references to it are stripped from parents with it.
    pub fn remove_group(&mut self, id: &str) {
        self.groups.retain(|g| g.id != id);
        for g in &mut self.groups {
            g.children.retain(|c| c != id);
        }
    }

    /// Every group, in creation order.
    pub fn group_list(&self) -> &[Group] {
        &self.groups
    }

    /// The group with this id, if any.
    pub fn group(&self, id: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }

    /// The group directly mustering a unit, if any.
    pub fn group_of_unit(&self, unit: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.units.iter().any(|u| u == unit))
    }

    /// Member units of one group, descended through its children.
    pub fn group_units(&self, id: &str) -> Vec<String> {
        let mut out = Vec::new();
        self.collect_units(id, &mut out);
        out
    }

    fn collect_units(&self, id: &str, out: &mut Vec<String>) {
        let Some(g) = self.group(id) else { return };
        out.extend(g.units.iter().cloned());
        for c in &g.children {
            self.collect_units(c, out);
        }
    }
}
