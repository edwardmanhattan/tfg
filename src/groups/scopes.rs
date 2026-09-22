//! Desktop scopes (map #70, spec #76): one scope per command held.
//!
//! Everything views all; actions gate on the scope's unit set. Scope ids
//! generalize the old `unit:`/`satgas:`/`gugus:` prefixes to `kind:id`
//! (`Unsur:..`, `SatuanTugas:..`, `Gugus:..`, `OperasiGabungan:..`).

use std::collections::{HashMap, HashSet};

use super::Groups;

/// One desktop scope: everything views all, actions gate on the scope's
/// unit set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub id: String,
    pub label: String,
    pub units: Vec<String>,
}

impl Groups {
    /// A user's desktop scopes: directly commanded units (helm counts as
    /// a unit scope) plus commanded groups. Empty for the seat-less
    /// (observers get the merged view-only desktop instead).
    pub fn scopes_for(
        &self,
        user: &str,
        unit_commander: &HashMap<String, String>,
        helm: &HashMap<String, String>,
    ) -> Vec<Scope> {
        let mut scopes: Vec<Scope> = Vec::new();
        let mut unit_scoped: HashSet<String> = HashSet::new();
        for (u, c) in unit_commander.iter().chain(helm.iter()) {
            if c.as_str() == user && unit_scoped.insert(u.clone()) {
                scopes.push(Scope { id: format!("unit:{u}"), label: u.clone(), units: vec![u.clone()] });
            }
        }
        for g in &self.groups {
            if g.commander.as_deref() == Some(user) {
                scopes.push(Scope {
                    id: format!("{}:{}", g.kind.label(), g.id),
                    label: g.name.clone(),
                    units: self.group_units(&g.id),
                });
            }
        }
        scopes
    }
}
