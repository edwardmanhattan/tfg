//! The embark rule: what may go aboard what, decided against the spec the
//! server published.
//!
//! THE CHECK IS LOCAL; THE SPEC IS NOT. A drop is judged on this machine —
//! the complement the host declares, the hulls already in it, and the
//! register's read of what kind of hull the child is — and the server stays
//! the final arbiter of anything this cannot see. That split is the console's
//! own: a drop that cannot be decided locally is never guessed, and a hull
//! the register does not resolve is passed through to the server rather than
//! refused on a local shrug.
//!
//! NOTHING HERE TALKS TO THE NETWORK. The spec arrives as plain data — the
//! complements a host declares and the hulls it holds — so the rule is a
//! function over values, testable without a session, and the shell's part is
//! only to fetch the spec and to say what the verdict was.
//!
//! THE VERDICT NAMES THE ACT, NOT THE WRITE. An allowed embark is a `host`
//! link in the operator's draft; which endpoint that becomes, and in what
//! order it is posted, is [`crate::force`]'s business at the stage advance.
//! Keeping the two apart is why a drop can be refused with a sentence the
//! operator reads without the server ever hearing about it.

use std::collections::BTreeMap;

use crate::backend::{EmbarkedUnit, UnitCapacity};

/// A hull's place in the taxonomy: the ids a complement's level names.
///
/// The class is always there — every hull has one. The type and the category
/// are the links above it, and a broken chain leaves them absent rather than
/// wrong, because "no type" and "the wrong type" are different truths and
/// only one of them is provable.
///
/// The register's read of this does not move under us, so it is read ONCE —
/// the console read it on the spot from its own store, and the GPUI binary
/// reads it at startup and hands the map over. A hull the register does not
/// know is ABSENT from the map, and that absence is a fact: the rule leaves
/// such a hull to the server rather than refusing it on a local shrug.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HullTaxonomy {
    pub class_id: i64,
    pub type_id: Option<i64>,
    pub category_id: Option<i64>,
}

/// Whether a hull belongs to a complement's kind: the kind is a level AND an
/// id — "four helicopters" against "four aircraft of any kind" against "this
/// exact hull".
///
/// An unknown level matches nothing. A complement whose level this client
/// does not understand is a rule it cannot read, and pretending it read it
/// would be a guess dressed as a check.
pub fn tax_matches(unit_id: i64, tax: &HullTaxonomy, level: &str, id_level: i64) -> bool {
    match level {
        "unit" => unit_id == id_level,
        "class" => tax.class_id == id_level,
        "type" => tax.type_id == Some(id_level),
        "category" => tax.category_id == Some(id_level),
        _ => false,
    }
}

/// One host's real spec, as the server published it.
///
/// This is the whole of what the rule reads about the SERVER's side: what
/// the host declares it carries, and what is in it right now. It crosses the
/// binary's boundary as one value so the shell never has to know which
/// endpoint answered what.
///
/// The register's read of what KIND each hull is NOT here: that is the
/// store's, it does not move under us, and it is read once at startup — see
/// [`HullTaxonomy`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EmbarkSpec {
    /// The host this spec is about.
    pub host: i64,
    /// The complements the host declares — `GET /units/{host}/capacities`.
    /// A hull that declares none carries nothing, and nothing may go aboard
    /// it.
    pub caps: Vec<UnitCapacity>,
    /// What is directly inside the host — `GET
    /// /games/{id}/units/{host}/embarkations`.
    pub aboard: Vec<EmbarkedUnit>,
}

/// Why a drop is refused.
///
/// The reason is what keeps the sentence truthful: a full complement is not
/// a wrong kind of hull, and telling the operator it is would send them to
/// look for the wrong thing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbarkRefusal {
    /// The hull is the host.
    SelfEmbark,
    /// The hull is already inside this host.
    AlreadyAboard,
    /// The host declares no complements at all: it carries nothing, and
    /// nothing may go aboard it.
    ///
    /// This is the server's own reading of an empty complement list — "it
    /// cannot carry these" is said by having no row at all — and the
    /// register's candidates read answers the same way.
    CarriesNothing,
    /// The host carries something else: the child's kind is not one of the
    /// host's complements.
    NotACandidate,
    /// A complement the child would fall under is full.
    Full {
        level: String,
        id_level: i64,
        would_hold: i64,
        max_aboard: i64,
    },
}

impl EmbarkRefusal {
    /// The reason, in the console's own words. The caller wraps it in the
    /// sentence the operator reads: "{child} cannot embark aboard {host} —
    /// {reason}".
    pub fn reason(&self) -> String {
        match self {
            EmbarkRefusal::SelfEmbark => "a hull does not embark on itself".to_string(),
            EmbarkRefusal::AlreadyAboard => "it is already aboard".to_string(),
            EmbarkRefusal::CarriesNothing => "it carries nothing".to_string(),
            EmbarkRefusal::NotACandidate => "not an allowed candidate".to_string(),
            EmbarkRefusal::Full {
                level,
                id_level,
                would_hold,
                max_aboard,
            } => format!("{level} {id_level} holds {would_hold} (max {max_aboard}) full"),
        }
    }
}

/// Decide whether a hull may go aboard another.
///
/// The order of the checks is the order an operator is likely to meet them,
/// and each one answers with its own reason:
///
/// 1. **Itself** — a hull does not embark on itself.
/// 2. **Already aboard** — the attach endpoint refuses a unit that is
///    "already inside something", so a hull already in this host is refused
///    here rather than at the server.
/// 3. **Carries nothing** — a host that declares no complements carries
///    nothing, and the server's own candidates read answers the same way:
///    "it cannot carry these" is said by having no row at all.
/// 4. **Not a candidate** — a hull of a kind the host does not carry is
///    refused. A child the register cannot resolve is NOT refused: the
///    server stays the final arbiter of a hull this client cannot classify.
/// 5. **Full** — a complement the child falls under is already at its
///    maximum. The count is the hulls already aboard whose kind this client
///    can resolve, plus the child: a hull whose kind cannot be resolved is
///    skipped rather than guessed.
///
/// An `Ok` is not a promise the server will keep. It is the local answer,
/// and the server's verdict still lands on the operator's line.
pub fn check(
    caps: &[UnitCapacity],
    aboard: &[EmbarkedUnit],
    taxonomy: &BTreeMap<i64, HullTaxonomy>,
    host: i64,
    child: i64,
) -> Result<(), EmbarkRefusal> {
    if host == child {
        return Err(EmbarkRefusal::SelfEmbark);
    }
    if aboard.iter().any(|u| u.id == child) {
        return Err(EmbarkRefusal::AlreadyAboard);
    }
    if caps.is_empty() {
        // No complements, no candidates: a hull that carries nothing offers
        // nothing, and the drop is refused rather than posted for the server
        // to refuse in its own words.
        return Err(EmbarkRefusal::CarriesNothing);
    }
    let Some(child_tax) = taxonomy.get(&child) else {
        // The register does not know this hull. It may be perfectly legal
        // aboard, so the server decides — the client cannot prove otherwise.
        return Ok(());
    };
    if !caps
        .iter()
        .any(|cap| tax_matches(child, child_tax, &cap.level, cap.id_level))
    {
        return Err(EmbarkRefusal::NotACandidate);
    }
    for cap in caps {
        if !tax_matches(child, child_tax, &cap.level, cap.id_level) {
            continue;
        }
        let held = aboard
            .iter()
            .filter(|u| {
                taxonomy
                    .get(&u.id)
                    .is_some_and(|tax| tax_matches(u.id, tax, &cap.level, cap.id_level))
            })
            .count() as i64;
        let would_hold = held + 1;
        if would_hold > cap.max_aboard {
            return Err(EmbarkRefusal::Full {
                level: cap.level.clone(),
                id_level: cap.id_level,
                would_hold,
                max_aboard: cap.max_aboard,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tax(class_id: i64, type_id: Option<i64>, category_id: Option<i64>) -> HullTaxonomy {
        HullTaxonomy {
            class_id,
            type_id,
            category_id,
        }
    }

    fn cap(level: &str, id_level: i64, max_aboard: i64) -> UnitCapacity {
        UnitCapacity {
            level: level.to_string(),
            id_level,
            max_aboard,
        }
    }

    fn aboard(id: i64, name: &str) -> EmbarkedUnit {
        EmbarkedUnit {
            id,
            name: name.to_string(),
            hull_number: String::new(),
            class_name: String::new(),
            is_cleared: false,
            cleared_at: None,
        }
    }

    /// A host that carries two of class 5, and a child of class 5.
    fn facts() -> (Vec<UnitCapacity>, BTreeMap<i64, HullTaxonomy>) {
        let caps = vec![cap("class", 5, 2)];
        let mut taxonomy = BTreeMap::new();
        taxonomy.insert(7, tax(5, Some(3), Some(2)));
        taxonomy.insert(9, tax(5, Some(3), Some(2)));
        taxonomy.insert(11, tax(6, Some(4), Some(2)));
        taxonomy.insert(12, tax(5, Some(3), Some(2)));
        (caps, taxonomy)
    }

    /// The plain case: a class-5 hull going into a host that carries two of
    /// class 5, with room to spare.
    #[test]
    fn a_matching_kind_is_allowed() {
        let (caps, taxonomy) = facts();
        assert_eq!(check(&caps, &[], &taxonomy, 5, 7), Ok(()));
    }

    /// A hull does not embark on itself — the one gesture that needs no
    /// spec at all.
    #[test]
    fn a_hull_on_itself_is_refused() {
        let (caps, taxonomy) = facts();
        assert_eq!(
            check(&caps, &[], &taxonomy, 7, 7),
            Err(EmbarkRefusal::SelfEmbark)
        );
    }

    /// The attach endpoint refuses a unit that is "already inside
    /// something", so a hull already in this host is refused locally.
    #[test]
    fn a_hull_already_aboard_is_refused() {
        let (caps, taxonomy) = facts();
        assert_eq!(
            check(&caps, &[aboard(7, "KRI Nanggala")], &taxonomy, 5, 7),
            Err(EmbarkRefusal::AlreadyAboard)
        );
    }

    /// A host that declares no complements carries nothing: there is no
    /// candidate list, so there is no embark. The reason says THAT, rather
    /// than the vaguer "not an allowed candidate" — a hull that carries
    /// nothing and a hull that carries something else are two different
    /// things to go and fix.
    #[test]
    fn a_host_that_carries_nothing_takes_nothing() {
        let (_, taxonomy) = facts();
        assert_eq!(
            check(&[], &[], &taxonomy, 5, 7),
            Err(EmbarkRefusal::CarriesNothing)
        );
    }

    /// A hull of a kind the host does not carry is not a candidate.
    #[test]
    fn a_kind_the_host_does_not_carry_is_refused() {
        let (caps, taxonomy) = facts();
        assert_eq!(
            check(&caps, &[], &taxonomy, 5, 11),
            Err(EmbarkRefusal::NotACandidate)
        );
    }

    /// A hull the register cannot resolve is passed through: the server
    /// stays the final arbiter of a hull this client cannot classify.
    #[test]
    fn an_unresolvable_hull_is_the_servers_to_decide() {
        let (caps, taxonomy) = facts();
        assert_eq!(check(&caps, &[], &taxonomy, 5, 999), Ok(()));
    }

    /// A complement with room left takes one more.
    #[test]
    fn a_complement_with_room_takes_one_more() {
        let (caps, taxonomy) = facts();
        assert_eq!(
            check(&caps, &[aboard(9, "KRI Rangsat")], &taxonomy, 5, 7),
            Ok(())
        );
    }

    /// A complement at its maximum refuses the drop, and the reason names
    /// the complement and the count — the operator needs to know WHICH
    /// complement is full, not that something is.
    #[test]
    fn a_full_complement_refuses_and_names_itself() {
        let (caps, taxonomy) = facts();
        assert_eq!(
            check(
                &caps,
                &[aboard(9, "KRI Rangsat"), aboard(12, "KRI Sultan")],
                &taxonomy,
                5,
                7
            ),
            Err(EmbarkRefusal::Full {
                level: "class".to_string(),
                id_level: 5,
                would_hold: 3,
                max_aboard: 2,
            })
        );
    }

    /// A hull already aboard whose kind this client cannot resolve is
    /// skipped rather than guessed, so it does not count against the
    /// complement — and the drop is allowed.
    #[test]
    fn an_unresolvable_passenger_does_not_count_against_the_complement() {
        let (caps, taxonomy) = facts();
        // 999 is aboard and unknown to the register, so the class-5
        // complement holds one known hull and has room for the child.
        assert_eq!(
            check(&caps, &[aboard(999, "unknown")], &taxonomy, 5, 7),
            Ok(())
        );
    }

    /// The kind is a level AND an id: a complement on the type lets a hull of
    /// that type aboard whatever its class is.
    #[test]
    fn a_type_level_complement_matches_the_type() {
        let caps = vec![cap("type", 3, 1)];
        let mut taxonomy = BTreeMap::new();
        taxonomy.insert(7, tax(5, Some(3), Some(2)));
        taxonomy.insert(11, tax(6, Some(3), Some(2)));
        assert_eq!(check(&caps, &[], &taxonomy, 5, 11), Ok(()));
    }

    /// "This exact hull" is the narrowest complement there is, and it names
    /// one hull.
    #[test]
    fn a_unit_level_complement_names_one_hull() {
        let caps = vec![cap("unit", 7, 1)];
        let mut taxonomy = BTreeMap::new();
        taxonomy.insert(7, tax(5, Some(3), Some(2)));
        taxonomy.insert(9, tax(5, Some(3), Some(2)));
        assert_eq!(check(&caps, &[], &taxonomy, 5, 7), Ok(()));
        assert_eq!(
            check(&caps, &[], &taxonomy, 5, 9),
            Err(EmbarkRefusal::NotACandidate)
        );
    }

    /// A complement whose level this client does not understand matches
    /// nothing — the rule cannot be read, so it is not satisfied.
    #[test]
    fn an_unknown_level_matches_nothing() {
        let caps = vec![cap("squadron", 5, 4)];
        let (_, taxonomy) = facts();
        assert_eq!(
            check(&caps, &[], &taxonomy, 5, 7),
            Err(EmbarkRefusal::NotACandidate)
        );
    }

    /// Every refusal says why, in words, and says something different from
    /// the others — a sentence that could be any of them is a sentence that
    /// tells the operator nothing.
    #[test]
    fn every_refusal_has_its_own_reason() {
        let reasons = [
            EmbarkRefusal::SelfEmbark.reason(),
            EmbarkRefusal::AlreadyAboard.reason(),
            EmbarkRefusal::CarriesNothing.reason(),
            EmbarkRefusal::NotACandidate.reason(),
            EmbarkRefusal::Full {
                level: "class".into(),
                id_level: 5,
                would_hold: 3,
                max_aboard: 2,
            }
            .reason(),
        ];
        for (i, a) in reasons.iter().enumerate() {
            assert!(!a.is_empty());
            for b in reasons.iter().skip(i + 1) {
                assert_ne!(a, b, "two refusals share the sentence {a:?}");
            }
        }
    }
}
