//! Session groups (map #70, spec #76): one unified group model.
//!
//! A [`Group`] is id/name/commander plus `units` and `children` members;
//! [`GroupKind`] fixes the level (Unsur < Satuan Tugas < Gugus < Operasi
//! Gabungan) as an explicit gapped rank, so middle insertion never
//! renumbers. Per-responsibility submodules below; the parent keeps the
//! [`Groups`] facade, the re-exports, and the tests.
//!
//! - [`model`]: [`Group`], [`GroupKind`], ranks.
//! - [`hierarchy`]: build/remove/query the forest, fail-loud validation.
//! - [`authority`]: rank-derived jurisdiction and authority.
//! - [`scopes`]: desktop [`Scope`]s with `kind:id` ids.

mod authority;
mod hierarchy;
mod model;
mod scopes;

pub use model::{Group, GroupKind};
pub use scopes::Scope;

/// The session's group forest.
#[derive(Debug, Clone, Default)]
pub struct Groups {
    groups: Vec<Group>,
}

#[cfg(test)]
mod scope_tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn scopes_cover_command_and_helm() {
        let mut g = Groups::default();
        g.add_group(
            "s1".into(),
            "Satgas A".into(),
            GroupKind::SatuanTugas,
            vec!["u1".into(), "u2".into()],
            vec![],
            Some("ani".into()),
        )
        .unwrap();
        g.add_group(
            "g1".into(),
            "Gugus X".into(),
            GroupKind::Gugus,
            vec![],
            vec!["s1".into()],
            Some("caca".into()),
        )
        .unwrap();
        let mut seats = HashMap::new();
        seats.insert("u2".into(), "budi".into());
        let mut helm = HashMap::new();
        helm.insert("u3".into(), "budi".into());
        let scopes = g.scopes_for("ani", &seats, &helm);
        assert_eq!(scopes.len(), 1);
        assert_eq!(scopes[0].id, "SatuanTugas:s1");
        assert_eq!(scopes[0].units, vec!["u1".to_string(), "u2".to_string()]);
        let scopes = g.scopes_for("caca", &seats, &helm);
        assert_eq!(scopes.len(), 1);
        assert_eq!(scopes[0].id, "Gugus:g1");
        // budi commands u2 and helms u3: two unit scopes.
        let scopes = g.scopes_for("budi", &seats, &helm);
        assert_eq!(scopes.len(), 2);
        assert!(scopes.iter().all(|s| s.id.starts_with("unit:")));
        assert!(g.scopes_for("nobody", &seats, &helm).is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Authority;
    use std::collections::HashMap;

    fn rig() -> (Groups, HashMap<String, String>) {
        let mut g = Groups::default();
        g.add_group(
            "s1".into(),
            "Satgas A".into(),
            GroupKind::SatuanTugas,
            vec!["u1".into(), "u2".into()],
            vec![],
            Some("ani".into()),
        )
        .unwrap();
        g.add_group(
            "s2".into(),
            "Satgas B".into(),
            GroupKind::SatuanTugas,
            vec!["u3".into()],
            vec![],
            Some("budi".into()),
        )
        .unwrap();
        g.add_group(
            "g1".into(),
            "Gugus X".into(),
            GroupKind::Gugus,
            vec![],
            vec!["s1".into(), "s2".into()],
            Some("caca".into()),
        )
        .unwrap();
        let mut seats = HashMap::new();
        seats.insert("u1".into(), "ani".into());
        (g, seats)
    }

    #[test]
    fn jurisdiction_descends_through_children() {
        let (g, seats) = rig();
        let caca = g.commanded_units("caca", &seats);
        assert_eq!(caca, ["u1", "u2", "u3"].into_iter().map(String::from).collect());
        let ani = g.commanded_units("ani", &seats);
        assert_eq!(ani, ["u1", "u2"].into_iter().map(String::from).collect());
        assert!(g.commanded_units("nobody", &seats).is_empty());
    }

    #[test]
    fn authority_takes_highest_rank() {
        let (g, seats) = rig();
        assert_eq!(g.authority("caca", "u1", &seats), Some(Authority::GUGUS));
        // ani commands u1 directly AND its group: the group rank wins.
        assert_eq!(g.authority("ani", "u1", &seats), Some(Authority::SATGAS));
        assert_eq!(g.authority("budi", "u3", &seats), Some(Authority::SATGAS));
        assert_eq!(g.authority("ani", "u3", &seats), None);
    }

    #[test]
    fn deeper_level_outranks_shallower() {
        let (mut g, seats) = (Groups::default(), HashMap::new());
        g.add_group("e1".into(), "Unsur 1".into(), GroupKind::Unsur, vec!["u1".into()], vec![], Some(
            "ani".into(),
        ))
        .unwrap();
        g.add_group(
            "o1".into(),
            "OpGab".into(),
            GroupKind::OperasiGabungan,
            vec![],
            vec!["e1".into()],
            Some("caca".into()),
        )
        .unwrap();
        assert_eq!(g.authority("caca", "u1", &seats), Some(Authority::OPERASI_GABUNGAN));
        assert_eq!(g.authority("ani", "u1", &seats), Some(Authority::UNSUR));
    }

    #[test]
    fn middle_insertion_needs_no_renumber() {
        assert!(GroupKind::Unsur.rank() < GroupKind::SatuanTugas.rank());
        assert!(GroupKind::SatuanTugas.rank() < GroupKind::Gugus.rank());
        assert!(GroupKind::Gugus.rank() < GroupKind::OperasiGabungan.rank());
        // A Koarmada between Gugus(30) and OperasiGabungan(40) fits at 35.
        assert!(GroupKind::Gugus.rank() < 35 && 35 < GroupKind::OperasiGabungan.rank());
    }

    #[test]
    fn unit_in_two_groups_is_rejected() {
        let (mut g, _) = rig();
        let err = g
            .add_group("s3".into(), "Satgas C".into(), GroupKind::SatuanTugas, vec!["u1".into()], vec![], None)
            .unwrap_err();
        assert!(err.contains("already in group"), "{err}");
    }

    #[test]
    fn group_over_unknown_child_is_rejected() {
        let (mut g, _) = rig();
        let err = g
            .add_group("g2".into(), "Gugus Y".into(), GroupKind::Gugus, vec![], vec!["nope".into()], None)
            .unwrap_err();
        assert!(err.contains("unknown group"), "{err}");
    }

    #[test]
    fn child_at_or_above_own_rank_is_rejected() {
        let (mut g, _) = rig();
        let err = g
            .add_group("x1".into(), "Sideways".into(), GroupKind::SatuanTugas, vec![], vec!["s1".into()], None)
            .unwrap_err();
        assert!(err.contains("rank violation"), "{err}");
    }

    #[test]
    fn remove_group_strips_parent_refs() {
        let (mut g, _) = rig();
        g.remove_group("s1");
        assert!(g.group_of_unit("u1").is_none());
        assert_eq!(g.group_units("g1"), vec!["u3".to_string()]);
    }
}
