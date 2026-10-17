//! Group model (map #70, spec #76): one struct plus a level enum.
//!
//! A [`Group`] is an id, a name, a [`GroupKind`], an optional commander,
//! and two member lists: directly mustered `units` and `children` groups.
//! Nesting is generic — any group may hold units, child groups, or both —
//! and every child must sit at a strictly lower rank (see `hierarchy`).

/// A level of the task organisation, lowest first.
///
/// Ranks are explicit and gapped (never positional): inserting a level
/// between two others takes a midpoint instead of renumbering anything,
/// mirroring the Minos echelon vocabulary (`Unsur / Satuan Tugas /
/// Gugus / Operasi Gabungan`, with a future Koarmada between Gugus and
/// Operasi Gabungan).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupKind {
    Unsur,
    SatuanTugas,
    Gugus,
    OperasiGabungan,
}

impl GroupKind {
    /// Sort key for this level. Compared, never indexed into.
    pub fn rank(self) -> u8 {
        match self {
            GroupKind::Unsur => 10,
            GroupKind::SatuanTugas => 20,
            GroupKind::Gugus => 30,
            GroupKind::OperasiGabungan => 40,
        }
    }

    /// Stable code-facing label (scope ids, logs). Human glossary terms
    /// live in `CONTEXT.md`; this never changes under them.
    pub fn label(self) -> &'static str {
        match self {
            GroupKind::Unsur => "Unsur",
            GroupKind::SatuanTugas => "SatuanTugas",
            GroupKind::Gugus => "Gugus",
            GroupKind::OperasiGabungan => "OperasiGabungan",
        }
    }

    /// Resolve a vocabulary level to a local kind, by Indonesian name.
    /// `None` for a level this build has no equivalent for (a future
    /// Koarmada between Gugus and Operasi Gabungan, or a renamed
    /// custom level): the caller refuses loudly rather than mustering
    /// units under a rank it cannot reason about. The server-side tree
    /// (a later slice) will carry those levels natively.
    pub fn from_echelon(name: &str) -> Option<Self> {
        let squashed: String = name
            .to_lowercase()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        match squashed.as_str() {
            "unsur" => Some(GroupKind::Unsur),
            "satuantugas" | "satgas" => Some(GroupKind::SatuanTugas),
            "gugus" => Some(GroupKind::Gugus),
            "operasigabungan" | "opgab" => Some(GroupKind::OperasiGabungan),
            _ => None,
        }
    }
}

/// One group: shared identity plus unit and child-group members.
///
/// `Satgas` survives only as a documented alias for a `SatuanTugas`-kind
/// group (glossary canonicalizes Satuan Tugas); it is not a type.
#[derive(Debug, Clone)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub kind: GroupKind,
    pub units: Vec<String>,
    pub children: Vec<String>,
    pub commander: Option<String>,
}

/// The rank rule, in the one place it is worded: a child sits strictly
/// below its parent. Both the in-memory forest (`hierarchy`) and the local
/// draft (`plan`) refuse through here, so a rank violation reads the same
/// wherever it is raised.
pub(super) fn rank_refusal(child_name: &str, child: GroupKind, parent: GroupKind) -> String {
    format!(
        "rank violation: child `{child_name}` ({}:{}) not below {}:{}",
        child.label(),
        child.rank(),
        parent.label(),
        parent.rank(),
    )
}

/// A group may not become its own ancestor. The in-memory forest makes
/// that impossible by construction (a child is always strictly lower), so
/// only the draft, where a re-parent is an edit, can meet it.
pub(super) fn cycle_refusal(group_name: &str) -> String {
    format!("group `{group_name}` cannot be its own ancestor")
}

/// A unit belongs to at most one group: two owners make jurisdiction
/// ambiguous. One wording for the in-memory forest and the draft.
pub(super) fn duplicate_unit_refusal(unit: &str, owner_name: &str) -> String {
    format!("unit `{unit}` already in group `{owner_name}`")
}

/// The same unit named twice in one group's own list.
pub(super) fn repeated_unit_refusal(unit: &str) -> String {
    format!("unit `{unit}` listed twice")
}
