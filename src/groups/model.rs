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
