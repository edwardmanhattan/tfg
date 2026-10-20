//! Entity-to-entity drag and drop: what the pointer is carrying, what it
//! landed on, and what the drop MEANS.
//!
//! THE PLANNING PHASE IS LOCAL FIRST, so a drop is a decision about the draft
//! rather than a write to Minos. That is what makes the interesting question a
//! pure one — "a hull was released on a group" — and this module is its whole
//! answer. Nothing here touches a draft, opens a socket, or knows GPUI: the
//! shell computes the target under the pointer, asks [`resolve_drop`], and
//! stages whatever verdict comes back. The same function decides the hover
//! indicator (would this land?) and the sound (ding or buzz), so a drop and
//! the affordance that predicted it can never disagree.
//!
//! THE VERDICT NAMES THE ACT, NOT THE WRITE. `Embark { host, child }` is
//! "put that hull aboard this one"; which endpoint that becomes, and in what
//! order it is posted, is [`crate::force`]'s business at the stage advance.
//! Keeping the two apart is the reason a drop can be refused with a sentence
//! the operator reads without the server ever hearing about it.
//!
//! A REFUSAL IS A VERDICT TOO. Dropping a hull on itself, a player on bare
//! water, a group on a hull — each is a real thing a pointer can do, and each
//! answers with its own sentence rather than a shrug. That is what lets the
//! buzz be honest: it fires for a reason the operator could have predicted
//! from the hover indicator, not for a drag that silently did nothing.

/// What the pointer is carrying.
///
/// The shell's ghost carries display data as well — a thumbnail, a glyph —
/// but the ghost is how the drag LOOKS. This is what it MEANS, and it is the
/// only half [`resolve_drop`] reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DragEntity {
    /// A hull: a register row from the Fleet picker, or a placed piece's
    /// marker on the map.
    Unit { unit_id: i64, name: String },
    /// A person: a directory row, or a seated participant's row.
    Player { user_id: i64, name: String },
    /// A node of the task organisation.
    Group { group_id: i64, name: String },
}

/// What the pointer landed on.
///
/// Each variant is a surface the shell can already hit-test: the map by view
/// fractions (the drop it has always supported), a hull by its marker or its
/// row, a group by its row, a person by theirs.
#[derive(Debug, Clone, PartialEq)]
pub enum DropTarget {
    /// Bare map, at fractions of the map view — the unprojected point is the
    /// binary's to compute, exactly as the existing fleet drop does it.
    Map { fx: f64, fy: f64 },
    /// A hull — a placed piece's marker, or a register row.
    Unit { unit_id: i64 },
    /// A node of the task organisation.
    Group { group_id: i64 },
    /// A person — a directory row, or a seated participant's row.
    Player { user_id: i64 },
    /// The players list ITSELF: the roster of seats, and the place a person
    /// is SEATED.
    ///
    /// Not a person. A person is a ROW in this list, and dropping a person on
    /// a person is the gesture that means nothing — so the list is its own
    /// target, and a person carried onto it takes a seat.
    Roster,
}

/// What a drop means, decided without touching anything.
///
/// The accepted variants each name one act the shell stages into the local
/// planning drafts. The refusal carries the operator's sentence, because a
/// drop that did nothing and a drop that was refused are different events and
/// only one of them deserves a buzz.
#[derive(Debug, Clone, PartialEq)]
pub enum DropVerdict {
    /// Put a hull on the map at a starting position. A hull the exercise does
    /// not hold yet is assigned on the way in — the drop that has always
    /// meant both, since placing IS assigning.
    Deploy { unit_id: i64, fx: f64, fy: f64 },
    /// Hand a hull to a person as its commander.
    AssignCommander { unit_id: i64, user_id: i64 },
    /// Put a hull aboard a host hull.
    Embark { host: i64, child: i64 },
    /// Put a hull into a node of the task organisation.
    Enlist { group_id: i64, unit_id: i64 },
    /// Move a group inside another group. The hierarchy ranks are the
    /// server's rule and [`crate::groups`]' — this only says the gesture.
    NestGroup { child: i64, parent: i64 },
    /// Hand a group to a person as its commander.
    CommandGroup { group_id: i64, user_id: i64 },
    /// Put a person in the room: the roster takes a row for them.
    ///
    /// The ROLE is not named here, because the role a seat takes is the
    /// operator's decision, made on the row the person lands in. A drop says
    /// "this person is in the room", and which seat they hold is the next
    /// click — so the verdict carries the person and the shell picks the
    /// default role, the same way a seat form does.
    Seat { user_id: i64 },
    /// The client refused the gesture, with the sentence to say out loud.
    Refused { why: String },
}

impl DropVerdict {
    /// Whether the gesture landed. The shell reads this twice: once per frame
    /// for the hover indicator, once on release for the sound.
    pub fn accepted(&self) -> bool {
        !matches!(self, DropVerdict::Refused { .. })
    }

    /// The refusal's sentence, when there is one — `None` for a landing.
    pub fn refusal(&self) -> Option<&str> {
        match self {
            DropVerdict::Refused { why } => Some(why),
            _ => None,
        }
    }
}

/// What dropping `entity` on `target` means.
///
/// Every pair answers. A gesture that has no meaning is a refusal, not a
/// silent no-op, because a drag the operator can complete and that then does
/// nothing is the failure this vocabulary exists to remove — the truth is
/// available at hover time, and a pointer that cannot tell the difference
/// between "wrong place" and "broken" is the thing that makes drag-and-drop
/// feel like guesswork.
pub fn resolve_drop(entity: &DragEntity, target: &DropTarget) -> DropVerdict {
    match (entity, target) {
        // ── A hull ────────────────────────────────────────────────────────
        (DragEntity::Unit { unit_id, .. }, DropTarget::Map { fx, fy }) => DropVerdict::Deploy {
            unit_id: *unit_id,
            fx: *fx,
            fy: *fy,
        },
        (DragEntity::Unit { unit_id, .. }, DropTarget::Unit { unit_id: host }) => {
            if unit_id == host {
                DropVerdict::Refused {
                    why: "a hull does not embark on itself".to_string(),
                }
            } else {
                DropVerdict::Embark {
                    host: *host,
                    child: *unit_id,
                }
            }
        }
        (DragEntity::Unit { unit_id, .. }, DropTarget::Player { user_id }) => {
            DropVerdict::AssignCommander {
                unit_id: *unit_id,
                user_id: *user_id,
            }
        }
        (DragEntity::Unit { unit_id, .. }, DropTarget::Group { group_id }) => DropVerdict::Enlist {
            group_id: *group_id,
            unit_id: *unit_id,
        },
        (DragEntity::Unit { .. }, DropTarget::Roster) => DropVerdict::Refused {
            why: "a hull is not a seat \u{2014} drop it on a player to make them its commander"
                .to_string(),
        },

        // ── A person ──────────────────────────────────────────────────────
        (DragEntity::Player { user_id, .. }, DropTarget::Unit { unit_id }) => {
            DropVerdict::AssignCommander {
                unit_id: *unit_id,
                user_id: *user_id,
            }
        }
        (DragEntity::Player { user_id, .. }, DropTarget::Group { group_id }) => {
            DropVerdict::CommandGroup {
                group_id: *group_id,
                user_id: *user_id,
            }
        }
        (DragEntity::Player { .. }, DropTarget::Map { .. }) => DropVerdict::Refused {
            why: "a person is a seat, not a piece \u{2014} there is no map position to give"
                .to_string(),
        },
        (DragEntity::Player { .. }, DropTarget::Player { .. }) => DropVerdict::Refused {
            why: "a person commands pieces, not people".to_string(),
        },
        (DragEntity::Player { user_id, .. }, DropTarget::Roster) => DropVerdict::Seat {
            user_id: *user_id,
        },

        // ── A group ───────────────────────────────────────────────────────
        (DragEntity::Group { group_id, .. }, DropTarget::Group { group_id: parent }) => {
            if group_id == parent {
                DropVerdict::Refused {
                    why: "a group does not sit inside itself".to_string(),
                }
            } else {
                DropVerdict::NestGroup {
                    child: *group_id,
                    parent: *parent,
                }
            }
        }
        (DragEntity::Group { .. }, DropTarget::Unit { .. }) => DropVerdict::Refused {
            why: "a group is a task-organisation node, not a hull".to_string(),
        },
        (DragEntity::Group { .. }, DropTarget::Player { .. }) => DropVerdict::Refused {
            why: "drop a person ON the group to give it a commander".to_string(),
        },
        (DragEntity::Group { .. }, DropTarget::Map { .. }) => DropVerdict::Refused {
            why: "a group has no map position \u{2014} it lives in the task organisation".to_string(),
        },
        (DragEntity::Group { .. }, DropTarget::Roster) => DropVerdict::Refused {
            why: "a group is not a seat \u{2014} it lives in the task organisation".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(id: i64) -> DragEntity {
        DragEntity::Unit {
            unit_id: id,
            name: format!("hull {id}"),
        }
    }

    fn player(id: i64) -> DragEntity {
        DragEntity::Player {
            user_id: id,
            name: format!("person {id}"),
        }
    }

    fn group(id: i64) -> DragEntity {
        DragEntity::Group {
            group_id: id,
            name: format!("group {id}"),
        }
    }

    /// The drop the shell has always supported, unchanged: a hull on bare
    /// water is a starting position.
    #[test]
    fn a_hull_on_the_map_deploys() {
        assert_eq!(
            resolve_drop(&unit(7), &DropTarget::Map { fx: 0.4, fy: 0.6 }),
            DropVerdict::Deploy {
                unit_id: 7,
                fx: 0.4,
                fy: 0.6
            }
        );
    }

    /// The new entity-to-entity rule: a hull on another hull embarks.
    #[test]
    fn a_hull_on_another_hull_embarks() {
        assert_eq!(
            resolve_drop(&unit(7), &DropTarget::Unit { unit_id: 9 }),
            DropVerdict::Embark { host: 9, child: 7 }
        );
    }

    /// And the one gesture that would be a ring: a hull aboard itself.
    #[test]
    fn a_hull_on_itself_is_refused() {
        let v = resolve_drop(&unit(7), &DropTarget::Unit { unit_id: 7 });
        assert!(!v.accepted());
        assert!(v.refusal().unwrap().contains("itself"));
    }

    /// Dragging a hull onto a person's row hands it over — and the mirror
    /// direction, a person onto a hull, is the same act.
    #[test]
    fn a_hull_and_a_person_assign_in_both_directions() {
        let want = DropVerdict::AssignCommander {
            unit_id: 7,
            user_id: 3,
        };
        assert_eq!(
            resolve_drop(&unit(7), &DropTarget::Player { user_id: 3 }),
            want
        );
        assert_eq!(
            resolve_drop(&player(3), &DropTarget::Unit { unit_id: 7 }),
            want
        );
    }

    /// A hull on a group is an enlistment; a person on a group is its
    /// commander. Two different acts, two different verdicts.
    #[test]
    fn a_group_takes_a_hull_as_a_member_and_a_person_as_a_commander() {
        assert_eq!(
            resolve_drop(&unit(7), &DropTarget::Group { group_id: 5 }),
            DropVerdict::Enlist {
                group_id: 5,
                unit_id: 7
            }
        );
        assert_eq!(
            resolve_drop(&player(3), &DropTarget::Group { group_id: 5 }),
            DropVerdict::CommandGroup {
                group_id: 5,
                user_id: 3
            }
        );
    }

    /// A group nests inside another group — and not inside itself.
    #[test]
    fn a_group_nests_in_a_group_but_not_in_itself() {
        assert_eq!(
            resolve_drop(&group(1), &DropTarget::Group { group_id: 2 }),
            DropVerdict::NestGroup {
                child: 1,
                parent: 2
            }
        );
        let v = resolve_drop(&group(1), &DropTarget::Group { group_id: 1 });
        assert!(!v.accepted());
        assert!(v.refusal().unwrap().contains("inside itself"));
    }

    /// A person carried onto the roster takes a seat. The verdict names the
    /// person and NOT a role: which seat they hold is the operator's next
    /// click, on the row they land in.
    #[test]
    fn a_person_on_the_roster_is_seated() {
        assert_eq!(
            resolve_drop(&player(3), &DropTarget::Roster),
            DropVerdict::Seat { user_id: 3 }
        );
    }

    /// The roster is a list of seats, not a place a hull or a group belongs:
    /// the two refusals say what to do instead, because a gesture that ends
    /// in silence is the failure this vocabulary exists to remove.
    #[test]
    fn only_a_person_belongs_on_the_roster() {
        for e in [unit(7), group(5)] {
            let v = resolve_drop(&e, &DropTarget::Roster);
            assert!(!v.accepted(), "{e:?} on the roster was accepted");
            assert!(
                v.refusal().is_some_and(|w| !w.is_empty()),
                "{e:?} on the roster refused with no sentence"
            );
        }
    }

    /// The refusals that keep the buzz honest: every gesture that has no
    /// meaning in this world says so rather than landing somewhere by
    /// accident. A test per row of the matrix, because a hole here is a drop
    /// that silently does nothing.
    #[test]
    fn the_meaningless_gestures_each_refuse_in_their_own_words() {
        let rows: Vec<(DragEntity, DropTarget)> = vec![
            (player(1), DropTarget::Map { fx: 0.5, fy: 0.5 }),
            (player(1), DropTarget::Player { user_id: 2 }),
            (group(1), DropTarget::Unit { unit_id: 7 }),
            (group(1), DropTarget::Player { user_id: 1 }),
            (group(1), DropTarget::Map { fx: 0.5, fy: 0.5 }),
            (group(1), DropTarget::Roster),
            (unit(7), DropTarget::Roster),
        ];
        for (e, t) in rows {
            let v = resolve_drop(&e, &t);
            assert!(!v.accepted(), "{e:?} on {t:?} was accepted");
            assert!(
                v.refusal().is_some_and(|w| !w.is_empty()),
                "{e:?} on {t:?} refused with no sentence"
            );
        }
    }

    /// An accepted drop carries no refusal and vice versa — the two readers
    /// of this value (indicator and sound) can never disagree about which
    /// kind of event they are looking at.
    #[test]
    fn accepted_and_refused_are_mutually_exclusive() {
        let landed = resolve_drop(&unit(7), &DropTarget::Map { fx: 0.0, fy: 0.0 });
        assert!(landed.accepted());
        assert!(landed.refusal().is_none());
        let refused = resolve_drop(&unit(7), &DropTarget::Unit { unit_id: 7 });
        assert!(!refused.accepted());
        assert!(refused.refusal().is_some());
    }
}
