//! The task organisation as the operator has it, before Minos has heard of
//! it.
//!
//! AUTHORING THE TREE IS A LOCAL ACT. A node used to be one backend write
//! fired the moment it was named, so a console whose roles could not yet
//! write planned against a game it could not touch, and the refusal arrived
//! in the middle of the draft. The operator stages the whole forest here
//! instead, and a stage advance is the moment [`plan`] is written through.
//! Nothing in this module talks to the network, which is the point: the
//! rules about what a legal draft is are pure functions over plain data, so
//! they are testable without a session.
//!
//! THE DRAFT NEVER HOLDS A TREE THE SERVER WOULD REFUSE. Three rules are
//! checked at STAGE time, not at flush time, in the model's own words:
//! a child sits at the level immediately above its parent (`GroupKind::sits_under`),
//! a group may not become its own ancestor, and a unit belongs to one group —
//! and a hull (an Unsur) only ever to a Satuan Tugas. A refusal the operator
//! meets while their cursor is still in the field is a refusal they can act on.
//!
//! THE ORDER OF [`plan`] IS LOAD-BEARING:
//!
//! 1. creates, parents before children — a child's parent has to exist on
//!    the server before the child can name it;
//! 2. a created node's commander;
//! 3. edits to held nodes (rename, re-parent, commander);
//! 4. units leaving a held node;
//! 5. units placed, after the node that holds them exists (creates land
//!    first, so a placement may name a node the same flush just made);
//! 6. removals last — nothing names a node that is about to go.
//!
//! Unit removals precede unit placements so a hull moving between groups is
//! never in two nodes at once.

use std::collections::{BTreeMap, BTreeSet};

use super::model::{
    GroupKind, cycle_refusal, duplicate_unit_refusal, hull_refusal, rank_refusal,
    repeated_unit_refusal,
};

/// One group as Minos last held it: the last thing a read left behind.
///
/// `kind` is the local rank vocabulary, resolved from the server's echelon
/// name by the caller ([`GroupKind::from_echelon`]); the server names the
/// level, never its rank.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveGroup {
    pub group_id: i64,
    pub name: String,
    pub kind: GroupKind,
    pub parent: Option<i64>,
    pub commander_id: Option<i64>,
    pub units: Vec<i64>,
}

/// One group being authored, before the flush has written it through.
///
/// A group the flush is about to create has a NEGATIVE `local_id` — never
/// colliding with a server id — which becomes a real id once the draft is
/// re-seeded from a read. `echelon_id` is the master-data `id_unit_hierarchy`
/// the create must send; it is zero on a row seeded from the server, because
/// only a create carries it and the read names the level instead.
#[derive(Debug, Clone, PartialEq)]
pub struct StagedGroup {
    /// Negative while new; the server id once the draft is re-seeded.
    pub local_id: i64,
    /// The echelon's `id_unit_hierarchy`, for a create. Zero when seeded.
    pub echelon_id: i64,
    pub name: String,
    pub kind: GroupKind,
    pub parent: Option<i64>,
    pub commander_id: Option<i64>,
    pub units: Vec<i64>,
}

/// Seed a staged row from what the server last said.
fn staged_from_live(live: &LiveGroup) -> StagedGroup {
    StagedGroup {
        local_id: live.group_id,
        // The read carries the echelon NAME, not its master-data id; no
        // frame row is ever created, so the id stays unset.
        echelon_id: 0,
        name: live.name.clone(),
        kind: live.kind,
        parent: live.parent,
        commander_id: live.commander_id,
        units: live.units.clone(),
    }
}

/// The operator's task organisation: new groups, edits to held groups, and
/// removals, over a frame of what the server last said.
///
/// The frame is the tree as Minos held it — the reference the rank, cycle and
/// unit rules read, because those are rules about a whole tree and a staging
/// call has to see the whole tree to refuse. `seed` sets it from a read;
/// `prune` reconciles it when the server has moved on. Staged work is only
/// ever the edits, the new groups, and the removals: a freshly seeded draft
/// plans nothing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GroupsDraft {
    /// What the server last said, in staged shape so validation can read it.
    frame: BTreeMap<i64, StagedGroup>,
    /// Groups being created, by negative local id.
    news: BTreeMap<i64, StagedGroup>,
    /// Edits to held groups: a full staged view per server id.
    edits: BTreeMap<i64, StagedGroup>,
    /// Held groups the operator has taken out of the tree.
    removed: BTreeSet<i64>,
    next_local: i64,
}

impl GroupsDraft {
    pub fn new() -> Self {
        Self::default()
    }

    fn fresh_local(&mut self) -> i64 {
        self.next_local -= 1;
        self.next_local
    }

    /// Set the frame from a read. Staged work is left alone: a re-read
    /// happens while the operator is still authoring, and discarding the
    /// draft on every poll would throw away work the flush has not written.
    /// Edits against groups the server no longer lists are dropped by
    /// [`Self::prune`].
    pub fn seed(&mut self, live: &[LiveGroup]) {
        self.frame = live
            .iter()
            .map(|l| {
                let staged = staged_from_live(l);
                (staged.local_id, staged)
            })
            .collect();
        let ids: Vec<i64> = live.iter().map(|l| l.group_id).collect();
        self.reconcile(&ids);
    }

    /// Drop staged work against groups the server no longer lists, so a
    /// deleted group cannot hold orphaned edits. A removed-group mark for a
    /// vanished group is moot; any node that pointed at it becomes a root.
    pub fn prune(&mut self, live_ids: &[i64]) {
        self.frame.retain(|id, _| live_ids.contains(id));
        self.reconcile(live_ids);
    }

    fn reconcile(&mut self, live_ids: &[i64]) {
        self.edits.retain(|id, _| live_ids.contains(id));
        self.removed.retain(|id| live_ids.contains(id));
        self.clear_dangling_parents(live_ids);
    }

    /// Nodes whose parent has left the tree become roots, so the draft stays
    /// a forest. A frame row is promoted to an edit so the flush sees it.
    fn clear_dangling_parents(&mut self, live_ids: &[i64]) {
        let dangling: Vec<i64> = self
            .view_ids()
            .into_iter()
            .filter(|id| match self.parent_of(*id) {
                Some(p) if p < 0 => !self.news.contains_key(&p),
                Some(p) => !live_ids.contains(&p),
                None => false,
            })
            .collect();
        for id in dangling {
            let _ = self.set_parent(id, None);
        }
    }

    /// Stage a new group. Answers its negative local id, so the caller can
    /// open it like any other row. Fails loud when the tree would not be one
    /// the server accepts: a blank or duplicate name, an unknown parent, a
    /// rank violation, a hull outside a Satuan Tugas, or a unit another
    /// group already holds.
    pub fn stage_group(
        &mut self,
        echelon_id: i64,
        name: &str,
        kind: GroupKind,
        parent: Option<i64>,
        commander_id: Option<i64>,
        units: Vec<i64>,
    ) -> Result<i64, String> {
        self.check_name(name, None)?;
        if let Some(parent) = parent {
            let parent_kind = self
                .view(parent)
                .map(|g| g.kind)
                .ok_or_else(|| format!("unknown group `{parent}`"))?;
            if !kind.sits_under(parent_kind) {
                return Err(rank_refusal(name, kind, parent_kind));
            }
        }
        if !units.is_empty() && kind != GroupKind::SatuanTugas {
            return Err(hull_refusal(kind));
        }
        self.check_units(None, &units)?;
        let local_id = self.fresh_local();
        self.news.insert(
            local_id,
            StagedGroup {
                local_id,
                echelon_id,
                name: name.to_string(),
                kind,
                parent,
                commander_id,
                units,
            },
        );
        Ok(local_id)
    }

    /// Rename a group. A blank or duplicate name is refused now.
    pub fn rename(&mut self, group_id: i64, name: &str) -> Result<(), String> {
        self.check_name(name, Some(group_id))?;
        let Some(slot) = self.edit_slot(group_id) else {
            return Err(format!("unknown group `{group_id}`"));
        };
        slot.name = name.to_string();
        self.settle(group_id);
        Ok(())
    }

    /// Hand a group to a commander, or clear the appointment. A group with
    /// nobody steering it is a legal draft state, unlike a piece.
    pub fn set_commander(
        &mut self,
        group_id: i64,
        commander_id: Option<i64>,
    ) -> Result<(), String> {
        let Some(slot) = self.edit_slot(group_id) else {
            return Err(format!("unknown group `{group_id}`"));
        };
        slot.commander_id = commander_id;
        self.settle(group_id);
        Ok(())
    }

    /// Replace a group's unit list. Fails loud on a unit listed twice, a
    /// unit another group already holds, or a hull in anything but a
    /// Satuan Tugas — a hull is an Unsur, and it sits only in the level
    /// immediately above it.
    pub fn set_units(&mut self, group_id: i64, units: Vec<i64>) -> Result<(), String> {
        let Some(group) = self.view(group_id) else {
            return Err(format!("unknown group `{group_id}`"));
        };
        if !units.is_empty() && group.kind != GroupKind::SatuanTugas {
            return Err(hull_refusal(group.kind));
        }
        self.check_units(Some(group_id), &units)?;
        let Some(slot) = self.edit_slot(group_id) else {
            return Err(format!("unknown group `{group_id}`"));
        };
        slot.units = units;
        self.settle(group_id);
        Ok(())
    }

    /// UNGROUP A HULL: take it out of the body that musters it, and leave it
    /// in the force. Answers the body it left, or `None` when no body held
    /// it.
    ///
    /// THE GROUP STAYS. A Satuan Tugas that just lost its last hull is a
    /// legal, empty command — deleting it here would also delete its
    /// commander's post, which the operator did not ask for. The hull is the
    /// thing being moved, so it is the only thing that moves.
    pub fn remove_unit(&mut self, unit_id: i64) -> Option<i64> {
        let owner = self.unit_owner(unit_id)?;
        let rest: Vec<i64> = self
            .view(owner)
            .map(|group| {
                group
                    .units
                    .iter()
                    .copied()
                    .filter(|unit| *unit != unit_id)
                    .collect()
            })
            .unwrap_or_default();
        self.set_units(owner, rest).ok()?;
        Some(owner)
    }

    /// Re-parent a group, or make it a root with `None`. Fails loud on an
    /// unknown parent, a cycle, or a rank violation.
    pub fn set_parent(&mut self, group_id: i64, parent: Option<i64>) -> Result<(), String> {
        if self.view(group_id).is_none() {
            return Err(format!("unknown group `{group_id}`"));
        }
        self.check_parent(group_id, parent)?;
        let Some(slot) = self.edit_slot(group_id) else {
            return Err(format!("unknown group `{group_id}`"));
        };
        slot.parent = parent;
        self.settle(group_id);
        Ok(())
    }

    /// Take a group out of the draft. A staged-new group simply vanishes; a
    /// held group is marked for deletion at the advance. Any node that
    /// pointed at it becomes a root, so the flush reparents the children
    /// before the delete lands.
    pub fn remove_group(&mut self, group_id: i64) -> bool {
        let existed = self.news.remove(&group_id).is_some()
            | self.edits.remove(&group_id).is_some()
            | self.frame.remove(&group_id).is_some();
        if !existed {
            return false;
        }
        if group_id > 0 {
            self.removed.insert(group_id);
        }
        let orphans: Vec<i64> = self
            .view_ids()
            .into_iter()
            .filter(|id| self.parent_of(*id) == Some(group_id))
            .collect();
        for orphan in orphans {
            let _ = self.set_parent(orphan, None);
        }
        true
    }

    /// Empty the draft: a released hold, a closed session, a hold that moved
    /// to another game.
    pub fn clear(&mut self) {
        self.frame.clear();
        self.news.clear();
        self.edits.clear();
        self.removed.clear();
        self.next_local = 0;
    }

    /// Whether the operator has staged anything at all. A frame alone is not
    /// work: a freshly seeded draft is empty in this sense.
    pub fn is_empty(&self) -> bool {
        self.news.is_empty() && self.edits.is_empty() && self.removed.is_empty()
    }

    /// How many writes are still owed to the server.
    pub fn pending(&self, live: &[LiveGroup]) -> usize {
        plan(self, live).len()
    }

    /// The operator's view of one group, held or staged-new.
    pub fn get(&self, group_id: i64) -> Option<&StagedGroup> {
        self.view(group_id)
    }

    pub fn contains(&self, group_id: i64) -> bool {
        self.view(group_id).is_some()
    }

    /// Whether this id is a group the flush is about to create.
    pub fn is_new(&self, group_id: i64) -> bool {
        self.news.contains_key(&group_id)
    }

    pub fn is_removed(&self, group_id: i64) -> bool {
        self.removed.contains(&group_id)
    }

    /// Every group the operator has staged: new groups and edits to held
    /// ones. The frame is not here — it is what the server said, not work.
    pub fn staged_groups(&self) -> impl Iterator<Item = &StagedGroup> {
        self.news.values().chain(self.edits.values())
    }

    /// Every group the server holds in the draft's frame.
    pub fn frame_groups(&self) -> impl Iterator<Item = &StagedGroup> {
        self.frame.values()
    }

    /// Held groups staged for deletion, in id order.
    pub fn removed_groups(&self) -> impl Iterator<Item = i64> + '_ {
        self.removed.iter().copied()
    }

    /// The group mustering a unit, if any. A removed group is no owner.
    pub fn unit_owner(&self, unit_id: i64) -> Option<i64> {
        self.news
            .values()
            .chain(self.edits.values())
            .chain(self.frame.values())
            .find(|g| g.units.contains(&unit_id))
            .map(|g| g.local_id)
    }

    pub fn unit_in_group(&self, unit_id: i64) -> bool {
        self.unit_owner(unit_id).is_some()
    }

    // -- internals --------------------------------------------------------

    fn view(&self, id: i64) -> Option<&StagedGroup> {
        self.news
            .get(&id)
            .or_else(|| self.edits.get(&id))
            .or_else(|| self.frame.get(&id))
    }

    fn view_ids(&self) -> Vec<i64> {
        let mut ids: BTreeSet<i64> = BTreeSet::new();
        ids.extend(self.news.keys().copied());
        ids.extend(self.edits.keys().copied());
        ids.extend(self.frame.keys().copied());
        ids.into_iter().collect()
    }

    fn parent_of(&self, id: i64) -> Option<i64> {
        self.view(id).and_then(|g| g.parent)
    }

    fn name_of(&self, id: i64) -> String {
        self.view(id)
            .map(|g| g.name.clone())
            .unwrap_or_else(|| format!("#{id}"))
    }

    /// The staged row to edit: a held group is seeded from the frame on first
    /// touch, so an edit is always a whole group rather than a patch.
    fn edit_slot(&mut self, id: i64) -> Option<&mut StagedGroup> {
        if id < 0 {
            return self.news.get_mut(&id);
        }
        if !self.edits.contains_key(&id) {
            let seed = self.frame.get(&id).cloned()?;
            self.edits.insert(id, seed);
        }
        self.edits.get_mut(&id)
    }

    /// Drop an edit that has come back to what the server holds, so a
    /// there-and-back change leaves no write and no pending count.
    fn settle(&mut self, id: i64) {
        if id > 0 && self.edits.get(&id) == self.frame.get(&id) {
            self.edits.remove(&id);
        }
    }

    fn check_name(&self, name: &str, ignore: Option<i64>) -> Result<(), String> {
        if name.trim().is_empty() {
            return Err("group needs a name".into());
        }
        let clash = self
            .news
            .values()
            .chain(self.edits.values())
            .chain(self.frame.values())
            .any(|g| Some(g.local_id) != ignore && g.name == name);
        if clash {
            return Err(format!("duplicate group name `{name}`"));
        }
        Ok(())
    }

    fn check_units(&self, id: Option<i64>, units: &[i64]) -> Result<(), String> {
        let mut seen: BTreeSet<i64> = BTreeSet::new();
        for unit in units {
            if !seen.insert(*unit) {
                return Err(repeated_unit_refusal(&unit.to_string()));
            }
            let owner = self
                .news
                .values()
                .chain(self.edits.values())
                .chain(self.frame.values())
                .find(|g| Some(g.local_id) != id && g.units.contains(unit));
            if let Some(owner) = owner {
                return Err(duplicate_unit_refusal(&unit.to_string(), &owner.name));
            }
        }
        Ok(())
    }

    fn check_parent(&self, id: i64, parent: Option<i64>) -> Result<(), String> {
        let Some(parent_id) = parent else {
            return Ok(());
        };
        let Some(parent_group) = self.view(parent_id) else {
            return Err(format!("unknown group `{parent_id}`"));
        };
        // A new parent that is the group itself or inside its own subtree is
        // a cycle: walk the parent chain and see whether it reaches id.
        let mut cursor = Some(parent_id);
        let mut walked: BTreeSet<i64> = BTreeSet::new();
        while let Some(step) = cursor {
            if step == id {
                return Err(cycle_refusal(&self.name_of(id)));
            }
            if !walked.insert(step) {
                break; // a malformed frame cannot loop us forever
            }
            cursor = self.parent_of(step);
        }
        let child = self.view(id).expect("the group being staged exists");
        if !child.kind.sits_under(parent_group.kind) {
            return Err(rank_refusal(&child.name, child.kind, parent_group.kind));
        }
        Ok(())
    }

    /// New groups in an order that puts every parent before its child. The
    /// frame cannot hold a staged parent, so the only barrier is another new
    /// group; the staged rules make the recursion terminate.
    fn creates_in_order(&self) -> Vec<&StagedGroup> {
        let mut out = Vec::new();
        let mut done: BTreeSet<i64> = BTreeSet::new();
        for id in self.news.keys() {
            self.emit_create(*id, &mut out, &mut done);
        }
        out
    }

    fn emit_create<'a>(
        &'a self,
        id: i64,
        out: &mut Vec<&'a StagedGroup>,
        done: &mut BTreeSet<i64>,
    ) {
        if !done.insert(id) {
            return;
        }
        let Some(group) = self.news.get(&id) else {
            return;
        };
        if let Some(parent) = group.parent {
            if self.news.contains_key(&parent) {
                self.emit_create(parent, out, done);
            }
        }
        out.push(group);
    }

    /// Removals deep-first, so a child is deleted before the parent that
    /// would otherwise refuse to go while it still holds it.
    fn removals_in_order(&self, live: &[LiveGroup]) -> Vec<i64> {
        let mut removed: Vec<(u8, i64)> = self
            .removed
            .iter()
            .filter_map(|id| {
                live.iter()
                    .find(|l| l.group_id == *id)
                    .map(|l| (l.kind.rank(), *id))
            })
            .collect();
        removed.sort();
        removed.into_iter().map(|(_, id)| id).collect()
    }
}

/// Attach the member hulls to the groups they sit in.
///
/// The server publishes a unit's group on the UNIT's own row and nowhere
/// else, so the membership is a join away — and without it every group
/// reads as "0 hulls" and the diff cannot see a unit the server already
/// places. A unit that names a group this read does not list is a member
/// of nothing here: it is dropped rather than given a group nobody holds.
///
/// `members` is `(unit id, group id)` per unit, in the exercise's own order.
/// One group as the board draws it: the row, and how deep it sits.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeNode {
    pub group: StagedGroup,
    pub depth: usize,
}

/// The staged groups as a DRAW ORDER: roots first, each parent before its
/// children, siblings in descending echelon rank and then by name (`§6.1`).
///
/// A TREE IS DRAWN BY WALKING IT, not by sorting a list and hoping the order
/// reads as a tree. Two things this has to survive, because both are reachable
/// by dragging:
///
/// - **An orphan.** A parent that is not in the staged set (removed, or never
///   seeded) makes its child a root here rather than a group that silently
///   vanishes from the board.
/// - **A cycle.** Two groups naming each other as parent would make the walk
///   infinite; the second one becomes a root, which is the least surprising
///   thing to draw and keeps the board finite.
/// The staged groups' children, by parent — the same edges [`tree`] walks, as
/// a graph rather than as rows.
///
/// THE RING PACK, THE MAP'S LINKS AND THE BASKET'S COUNTS all read the
/// organisation as "what hangs under this body", and all three need the same
/// answer in the same order. One function, so a hull that a drop moved under a
/// different body cannot be packed under one parent and counted under another.
///
/// A parent that is not in the staged set makes its child a root, exactly as
/// [`tree`] draws it: the map must not link a hull to a body that is not there.
/// A group naming itself is its own root, and the caller's walk is bounded by
/// the rank ladder rather than by trusting this map.
pub fn children_map(groups: &[StagedGroup]) -> BTreeMap<i64, Vec<i64>> {
    let present: BTreeSet<i64> = groups.iter().map(|g| g.local_id).collect();
    let by_id: BTreeMap<i64, &StagedGroup> =
        groups.iter().map(|g| (g.local_id, g)).collect();
    let mut out: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
    for group in groups {
        let Some(parent) = group
            .parent
            .filter(|p| present.contains(p) && *p != group.local_id)
        else {
            continue;
        };
        out.entry(parent).or_default().push(group.local_id);
    }
    // The tree's own sibling order — descending rank, then by name (`§6.1`),
    // so a pack and a board list a body's children the same way round.
    for bucket in out.values_mut() {
        bucket.sort_by(|a, b| {
            let (a, b) = (by_id.get(a), by_id.get(b));
            match (a, b) {
                (Some(a), Some(b)) => b
                    .kind
                    .rank()
                    .cmp(&a.kind.rank())
                    .then_with(|| a.name.cmp(&b.name)),
                _ => std::cmp::Ordering::Equal,
            }
        });
    }
    out
}

pub fn tree(groups: &[StagedGroup]) -> Vec<TreeNode> {
    use std::collections::HashSet;
    let present: HashSet<i64> = groups.iter().map(|g| g.local_id).collect();
    let mut children: std::collections::HashMap<Option<i64>, Vec<&StagedGroup>> =
        std::collections::HashMap::new();
    for group in groups {
        let parent = match group.parent {
            Some(parent) if present.contains(&parent) => Some(parent),
            _ => None,
        };
        children.entry(parent).or_default().push(group);
    }
    for bucket in children.values_mut() {
        bucket.sort_by(|a, b| {
            b.kind
                .rank()
                .cmp(&a.kind.rank())
                .then_with(|| a.name.cmp(&b.name))
        });
    }
    let mut out = Vec::with_capacity(groups.len());
    let mut seen: HashSet<i64> = HashSet::new();
    fn walk(
        parent: Option<i64>,
        depth: usize,
        children: &std::collections::HashMap<Option<i64>, Vec<&StagedGroup>>,
        seen: &mut std::collections::HashSet<i64>,
        out: &mut Vec<TreeNode>,
    ) {
        let Some(bucket) = children.get(&parent) else {
            return;
        };
        for group in bucket {
            if !seen.insert(group.local_id) {
                // A cycle: this group is already on the board above itself.
                continue;
            }
            out.push(TreeNode {
                group: (*group).clone(),
                depth,
            });
            walk(Some(group.local_id), depth + 1, children, seen, out);
        }
    }
    walk(None, 0, &children, &mut seen, &mut out);
    // Anything a cycle made unreachable is drawn at the root, so the board
    // never hides a group the operator made.
    for group in groups {
        if !seen.contains(&group.local_id) {
            out.push(TreeNode {
                group: group.clone(),
                depth: 0,
            });
        }
    }
    out
}

pub fn join_members(groups: &mut [LiveGroup], members: &[(i64, i64)]) {
    for group in groups.iter_mut() {
        group.units.clear();
    }
    for (unit_id, group_id) in members {
        if let Some(group) = groups.iter_mut().find(|g| g.group_id == *group_id) {
            group.units.push(*unit_id);
        }
    }
    for group in groups.iter_mut() {
        group.units.sort_unstable();
        group.units.dedup();
    }
}

/// A node a write names as a parent: one the same flush is about to create
/// (negative, resolved from the create answer at pump time), or one the
/// server already holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupRef {
    New(i64),
    Live(i64),
}

impl GroupRef {
    fn of(id: i64) -> Self {
        if id < 0 {
            GroupRef::New(id)
        } else {
            GroupRef::Live(id)
        }
    }

    /// The id this ref names, staged-negative or server-positive.
    pub fn id(self) -> i64 {
        match self {
            GroupRef::New(id) | GroupRef::Live(id) => id,
        }
    }
}

/// One tree write the flush owes Minos. Each is one endpoint.
#[derive(Debug, Clone, PartialEq)]
pub enum GroupWrite {
    /// Create a node. The answer carries the server id the staged id
    /// resolves against. `parent` is `None` for a root.
    CreateNode {
        local_id: i64,
        name: String,
        echelon_id: i64,
        parent: Option<GroupRef>,
    },
    /// Rename a held node.
    Rename { group_id: i64, name: String },
    /// Delete a held node.
    DeleteNode { group_id: i64 },
    /// Re-parent a held node under a node that exists by then.
    MoveNode { group_id: i64, parent: GroupRef },
    /// Make a held node a root.
    ClearParent { group_id: i64 },
    /// Appoint a node's commander.
    SetCommander { group: GroupRef, user_id: i64 },
    /// Clear a held node's commander.
    ClearCommander { group_id: i64 },
    /// Place a hull in a node (its create lands first, if it is new).
    PlaceUnit { group: GroupRef, unit_id: i64 },
    /// Take a hull out of its node.
    UnplaceUnit { unit_id: i64 },
}

impl GroupWrite {
    /// What this write is about, for a line that has to say which node held
    /// the whole advance up.
    pub fn label(&self) -> String {
        match self {
            Self::CreateNode { name, .. } => format!("create node {name:?}"),
            Self::Rename { group_id, name } => format!("rename node {group_id} to {name:?}"),
            Self::DeleteNode { group_id } => format!("delete node {group_id}"),
            Self::MoveNode { group_id, .. } => format!("move node {group_id}"),
            Self::ClearParent { group_id } => format!("unparent node {group_id}"),
            Self::SetCommander { group, .. } => format!("command node {}", group.id()),
            Self::ClearCommander { group_id } => format!("clear commander on node {group_id}"),
            Self::PlaceUnit { group, unit_id } => {
                format!("place unit {unit_id} in node {}", group.id())
            }
            Self::UnplaceUnit { unit_id } => format!("unplace unit {unit_id}"),
        }
    }
}

/// The writes that carry `draft` onto a server holding `live`, in the order
/// the server accepts them.
///
/// New groups are created parents-first, so a child's create can name a
/// parent the server already has. Commander and unit writes follow, and a
/// placement may name a node created a moment earlier. Edits to held nodes
/// follow so a re-parent may name a fresh node. Removals land last, so
/// nothing names a node that is going away. A held group whose id is no
/// longer in `live` is skipped rather than written at a row that is not
/// there.
pub fn plan(draft: &GroupsDraft, live: &[LiveGroup]) -> Vec<GroupWrite> {
    let mut writes = Vec::new();
    let creates = draft.creates_in_order();

    // 1. Creates, parents before children.
    for group in &creates {
        writes.push(GroupWrite::CreateNode {
            local_id: group.local_id,
            name: group.name.clone(),
            echelon_id: group.echelon_id,
            parent: group.parent.map(GroupRef::of),
        });
    }

    // 2. A created node's commander.
    for group in &creates {
        if let Some(user_id) = group.commander_id {
            writes.push(GroupWrite::SetCommander {
                group: GroupRef::New(group.local_id),
                user_id,
            });
        }
    }

    // Edits to held nodes, the ones still in live.
    let held: Vec<(i64, &StagedGroup, &LiveGroup)> = draft
        .edits
        .iter()
        .filter_map(|(id, staged)| {
            live.iter()
                .find(|l| l.group_id == *id)
                .map(|here| (*id, staged, here))
        })
        .collect();

    // 3. Name, parent and commander of held nodes.
    for (group_id, staged, here) in &held {
        if staged.name != here.name {
            writes.push(GroupWrite::Rename {
                group_id: *group_id,
                name: staged.name.clone(),
            });
        }
        if staged.parent != here.parent {
            match staged.parent {
                Some(parent) => writes.push(GroupWrite::MoveNode {
                    group_id: *group_id,
                    parent: GroupRef::of(parent),
                }),
                None => writes.push(GroupWrite::ClearParent {
                    group_id: *group_id,
                }),
            }
        }
        match (staged.commander_id, here.commander_id) {
            (Some(user_id), current) if current != Some(user_id) => {
                writes.push(GroupWrite::SetCommander {
                    group: GroupRef::Live(*group_id),
                    user_id,
                });
            }
            (None, Some(_)) => writes.push(GroupWrite::ClearCommander {
                group_id: *group_id,
            }),
            _ => {}
        }
    }

    // 4. Units leaving a held node: before any placement, so a hull moving
    //    between groups is never in two nodes at once.
    for (_, staged, here) in &held {
        for unit_id in &here.units {
            if !staged.units.contains(unit_id) {
                writes.push(GroupWrite::UnplaceUnit { unit_id: *unit_id });
            }
        }
    }

    // 5. Units placed, after the node that holds them exists: a created
    //    node landed in step 1, a held node already existed.
    for group in &creates {
        for unit_id in &group.units {
            writes.push(GroupWrite::PlaceUnit {
                group: GroupRef::New(group.local_id),
                unit_id: *unit_id,
            });
        }
    }
    for (group_id, staged, here) in &held {
        for unit_id in &staged.units {
            if !here.units.contains(unit_id) {
                writes.push(GroupWrite::PlaceUnit {
                    group: GroupRef::Live(*group_id),
                    unit_id: *unit_id,
                });
            }
        }
    }

    // 6. Removals last.
    for group_id in draft.removals_in_order(live) {
        writes.push(GroupWrite::DeleteNode { group_id });
    }

    writes
}

#[cfg(test)]
mod tests {

    fn row(local_id: i64, name: &str, kind: GroupKind, parent: Option<i64>) -> StagedGroup {
        StagedGroup {
            local_id,
            echelon_id: 0,
            name: name.to_string(),
            kind,
            parent,
            commander_id: None,
            units: Vec::new(),
        }
    }

    /// A TREE IS WALKED: parents before children, siblings by rank then name.
    #[test]
    fn the_board_walks_parents_before_children() {
        let groups = vec![
            row(-1, "OPGAB", GroupKind::OperasiGabungan, None),
            row(-2, "GUS-1", GroupKind::Gugus, Some(-1)),
            row(-3, "TG-2", GroupKind::SatuanTugas, Some(-2)),
            row(-4, "TG-1", GroupKind::SatuanTugas, Some(-2)),
        ];
        let at = tree(&groups);
        assert_eq!(
            at.iter()
                .map(|n| (n.group.name.as_str(), n.depth))
                .collect::<Vec<_>>(),
            vec![("OPGAB", 0), ("GUS-1", 1), ("TG-1", 2), ("TG-2", 2)],
            "a parent is drawn before its children, siblings by name"
        );
    }

    /// AN ORPHAN IS A ROOT, not a group that vanishes from the board.
    #[test]
    fn an_orphan_is_drawn_at_the_root() {
        let groups = vec![row(-5, "TG-9", GroupKind::SatuanTugas, Some(-99))];
        let at = tree(&groups);
        assert_eq!(at.len(), 1);
        assert_eq!(at[0].depth, 0);
    }

    /// A CYCLE STAYS FINITE: the walk visits each group once and the board
    /// still holds all of them.
    #[test]
    fn a_cycle_cannot_loop_the_walk() {
        let groups = vec![
            row(-6, "A", GroupKind::Gugus, Some(-7)),
            row(-7, "B", GroupKind::Gugus, Some(-6)),
        ];
        let at = tree(&groups);
        assert_eq!(at.len(), 2, "both groups are on the board");
    }
    use super::*;

    fn live_group(
        group_id: i64,
        name: &str,
        kind: GroupKind,
        parent: Option<i64>,
        commander_id: Option<i64>,
        units: &[i64],
    ) -> LiveGroup {
        LiveGroup {
            group_id,
            name: name.into(),
            kind,
            parent,
            commander_id,
            units: units.to_vec(),
        }
    }

    #[test]
    fn an_empty_draft_plans_nothing() {
        assert_eq!(plan(&GroupsDraft::new(), &[]), Vec::new());
    }

    /// A group the server has never heard of costs exactly one create, and
    /// its local id is negative so it can never be mistaken for a server id.
    #[test]
    fn a_staged_root_is_one_create() {
        let mut draft = GroupsDraft::new();
        let id = draft
            .stage_group(
                7,
                "Operasi Gabungan",
                GroupKind::OperasiGabungan,
                None,
                None,
                vec![],
            )
            .expect("a bare root stages");
        assert!(id < 0, "a staged group never collides with a server id");
        assert_eq!(
            plan(&draft, &[]),
            vec![GroupWrite::CreateNode {
                local_id: id,
                name: "Operasi Gabungan".into(),
                echelon_id: 7,
                parent: None,
            }]
        );
    }

    /// A child's create names its parent, so the parent's create has to
    /// come first or the server 400s on an unknown parent.
    #[test]
    fn a_staged_child_is_created_after_its_parent() {
        let mut draft = GroupsDraft::new();
        let parent = draft
            .stage_group(30, "Gugus", GroupKind::Gugus, None, None, vec![])
            .unwrap();
        let child = draft
            .stage_group(
                20,
                "Satgas A",
                GroupKind::SatuanTugas,
                Some(parent),
                None,
                vec![],
            )
            .unwrap();
        let writes = plan(&draft, &[]);
        assert!(matches!(
            &writes[0],
            GroupWrite::CreateNode { local_id, parent: None, .. } if *local_id == parent
        ));
        assert!(matches!(
            &writes[1],
            GroupWrite::CreateNode { local_id, parent: Some(GroupRef::New(p)), .. }
                if *local_id == child && *p == parent
        ));
    }

    /// The model's own words, not a second copy: a child must sit at the
    /// level immediately above its parent, so a Satgas under an Unsur is
    /// refused before any write is planned.
    #[test]
    fn a_rank_violation_is_refused_at_stage_time() {
        let mut draft = GroupsDraft::new();
        draft.seed(&[live_group(1, "Unsur A", GroupKind::Unsur, None, None, &[])]);
        let err = draft
            .stage_group(
                20,
                "Satgas under Unsur",
                GroupKind::SatuanTugas,
                Some(1),
                None,
                vec![],
            )
            .unwrap_err();
        assert!(err.contains("rank violation"), "{err}");
    }

    /// A SKIPPED LEVEL IS NOT A PARENT (`§6.3`). A Satuan Tugas belongs in a
    /// Gugus; an Operasi Gabungan is above it but is not the level above it,
    /// and the refusal names both ranks so the operator can see the gap.
    #[test]
    fn a_skipped_level_is_refused() {
        let mut draft = GroupsDraft::new();
        draft.seed(&[live_group(
            1,
            "OpGab",
            GroupKind::OperasiGabungan,
            None,
            None,
            &[],
        )]);
        let err = draft
            .stage_group(
                20,
                "Satgas A",
                GroupKind::SatuanTugas,
                Some(1),
                None,
                vec![],
            )
            .unwrap_err();
        assert!(err.contains("rank violation"), "{err}");
        assert!(err.contains("SatuanTugas:20"), "{err}");
        assert!(err.contains("OperasiGabungan:40"), "{err}");
    }

    /// A HULL IS AN UNSUR AND SITS ONLY IN A SATUAN TUGAS. The basket's cards
    /// are hulls, so a drop on a Gugus or an Operasi Gabungan is refused with
    /// the sentence that says which level does hold them.
    #[test]
    fn a_hull_sits_only_in_a_satuan_tugas() {
        let mut draft = GroupsDraft::new();
        let err = draft
            .stage_group(30, "Gugus A", GroupKind::Gugus, None, None, vec![7])
            .unwrap_err();
        assert!(err.contains("hulls are Unsur:10"), "{err}");
        assert!(err.contains("Gugus:30"), "{err}");

        let satgas = draft
            .stage_group(20, "Satgas A", GroupKind::SatuanTugas, None, None, vec![7])
            .unwrap();
        assert!(draft.contains(satgas));

        // And the same rule on an edit: a group that already exists cannot
        // be handed a hull unless it is the level that holds hulls.
        let gugus = draft
            .stage_group(30, "Gugus B", GroupKind::Gugus, None, None, vec![])
            .unwrap();
        let err = draft.set_units(gugus, vec![8]).unwrap_err();
        assert!(err.contains("hulls are Unsur:10"), "{err}");
        // Emptying a group is always legal: a body may hold nothing.
        draft.set_units(gugus, vec![]).unwrap();
    }

    /// A GROUP RE-PARENTS ONLY ONE LEVEL. A Satgas already under a Gugus
    /// cannot be moved straight to the Operasi Gabungan above it.
    #[test]
    fn a_group_reparents_only_to_its_immediate_upper_level() {
        let live = vec![
            live_group(1, "OpGab", GroupKind::OperasiGabungan, None, None, &[]),
            live_group(2, "Gugus A", GroupKind::Gugus, Some(1), None, &[]),
            live_group(3, "Satgas A", GroupKind::SatuanTugas, Some(2), None, &[]),
        ];
        let mut draft = GroupsDraft::new();
        draft.seed(&live);
        let err = draft.set_parent(3, Some(1)).unwrap_err();
        assert!(err.contains("rank violation"), "{err}");
        // The level above it is the one that takes it, and it was already
        // there: the draft settles to nothing.
        draft.set_parent(3, Some(2)).unwrap();
        assert!(draft.is_empty());
    }

    /// A new parent inside the group's own subtree is a cycle. It is named
    /// as a cycle rather than as a rank violation, because that is what the
    /// operator tried to do.
    #[test]
    fn a_cycle_is_refused_at_stage_time() {
        let mut draft = GroupsDraft::new();
        draft.seed(&[
            live_group(1, "Gugus A", GroupKind::Gugus, None, None, &[]),
            live_group(2, "Satgas B", GroupKind::SatuanTugas, Some(1), None, &[]),
        ]);
        let err = draft.set_parent(1, Some(2)).unwrap_err();
        assert!(err.contains("its own ancestor"), "{err}");
    }

    /// Jurisdiction is unambiguous: a hull the server already places is not
    /// placed again in a staged group.
    #[test]
    fn a_unit_in_two_groups_is_refused() {
        let mut draft = GroupsDraft::new();
        draft.seed(&[live_group(
            1,
            "Satgas A",
            GroupKind::SatuanTugas,
            None,
            None,
            &[7],
        )]);
        let err = draft
            .stage_group(20, "Satgas B", GroupKind::SatuanTugas, None, None, vec![7])
            .unwrap_err();
        assert!(err.contains("already in group"), "{err}");
    }

    /// A draft seeded from what the server holds has no work to do, and its
    /// frame is not work: the flush must post nothing on a clean poll.
    #[test]
    fn a_synced_tree_plans_nothing() {
        let live = vec![
            live_group(1, "Gugus A", GroupKind::Gugus, None, Some(3), &[]),
            live_group(2, "Satgas A", GroupKind::SatuanTugas, Some(1), None, &[7]),
        ];
        let mut draft = GroupsDraft::new();
        draft.seed(&live);
        assert_eq!(plan(&draft, &live), Vec::new());
        assert_eq!(draft.pending(&live), 0);
        assert!(draft.is_empty(), "a frame alone is not staged work");
    }

    /// A re-parent is one move, naming the new parent by ref; coming back to
    /// where it started owes nothing at all.
    #[test]
    fn a_reparent_is_one_move() {
        let live = vec![
            live_group(1, "Gugus A", GroupKind::Gugus, None, None, &[]),
            live_group(2, "Satgas A", GroupKind::SatuanTugas, Some(1), None, &[]),
        ];
        let mut draft = GroupsDraft::new();
        draft.seed(&live);
        draft.set_parent(2, None).unwrap();
        assert_eq!(
            plan(&draft, &live),
            vec![GroupWrite::ClearParent { group_id: 2 }]
        );
        // There and back again: the draft settles, so nothing is owed.
        draft.set_parent(2, Some(1)).unwrap();
        assert_eq!(plan(&draft, &live), Vec::new());
        assert!(draft.is_empty());
    }

    /// Handing a held group to a commander is one write; taking it back is
    /// one different write.
    #[test]
    fn setting_and_clearing_a_commander() {
        let live = vec![live_group(
            1,
            "Satgas A",
            GroupKind::SatuanTugas,
            None,
            None,
            &[],
        )];
        let mut draft = GroupsDraft::new();
        draft.seed(&live);
        draft.set_commander(1, Some(9)).unwrap();
        assert_eq!(
            plan(&draft, &live),
            vec![GroupWrite::SetCommander {
                group: GroupRef::Live(1),
                user_id: 9
            }]
        );

        let live = vec![live_group(
            1,
            "Satgas A",
            GroupKind::SatuanTugas,
            None,
            Some(9),
            &[],
        )];
        let mut draft = GroupsDraft::new();
        draft.seed(&live);
        draft.set_commander(1, None).unwrap();
        assert_eq!(
            plan(&draft, &live),
            vec![GroupWrite::ClearCommander { group_id: 1 }]
        );
    }

    /// A placement names its node, so a node created in the same flush has
    /// to be created first.
    #[test]
    fn a_unit_is_placed_after_its_new_node_is_created() {
        let mut draft = GroupsDraft::new();
        let id = draft
            .stage_group(20, "Satgas A", GroupKind::SatuanTugas, None, None, vec![7])
            .unwrap();
        let writes = plan(&draft, &[]);
        assert!(matches!(
            &writes[0],
            GroupWrite::CreateNode { local_id, .. } if *local_id == id
        ));
        assert_eq!(
            writes[1],
            GroupWrite::PlaceUnit {
                group: GroupRef::New(id),
                unit_id: 7
            }
        );
    }

    /// A deletion is a write, and it lands after every other write, so
    /// nothing names a node that is about to go.
    #[test]
    fn a_removed_held_group_is_a_delete_last() {
        let live = vec![
            live_group(1, "OpGab", GroupKind::OperasiGabungan, None, None, &[]),
            live_group(2, "Satgas A", GroupKind::SatuanTugas, Some(1), None, &[]),
        ];
        let mut draft = GroupsDraft::new();
        draft.seed(&live);
        draft.rename(1, "OpGab Jaya").unwrap();
        assert!(draft.remove_group(2));
        assert_eq!(
            plan(&draft, &live),
            vec![
                GroupWrite::Rename {
                    group_id: 1,
                    name: "OpGab Jaya".into()
                },
                GroupWrite::DeleteNode { group_id: 2 },
            ]
        );
    }

    /// Deleting a node that still holds children re-roots them first, so the
    /// delete lands on a node the server will let go.
    #[test]
    fn a_removed_parent_re_roots_its_children_first() {
        let live = vec![
            live_group(1, "Gugus A", GroupKind::Gugus, None, None, &[]),
            live_group(2, "Satgas A", GroupKind::SatuanTugas, Some(1), None, &[]),
        ];
        let mut draft = GroupsDraft::new();
        draft.seed(&live);
        assert!(draft.remove_group(1));
        assert_eq!(
            plan(&draft, &live),
            vec![
                GroupWrite::ClearParent { group_id: 2 },
                GroupWrite::DeleteNode { group_id: 1 },
            ]
        );
    }

    /// Removing a staged-new group leaves no write at all: nothing was ever
    /// created.
    #[test]
    fn removing_a_staged_new_group_leaves_no_write() {
        let mut draft = GroupsDraft::new();
        let id = draft
            .stage_group(20, "Satgas A", GroupKind::SatuanTugas, None, None, vec![])
            .unwrap();
        assert!(draft.remove_group(id));
        assert_eq!(plan(&draft, &[]), Vec::new());
    }

    /// Prune drops staged work against a group the server no longer lists,
    /// so a deleted group cannot hold orphaned edits.
    #[test]
    fn prune_drops_staged_ops_against_deleted_groups() {
        let mut draft = GroupsDraft::new();
        draft.seed(&[live_group(
            5,
            "Satgas A",
            GroupKind::SatuanTugas,
            None,
            None,
            &[],
        )]);
        draft.set_commander(5, Some(9)).unwrap();
        draft.prune(&[]);
        assert_eq!(plan(&draft, &[]), Vec::new());
        assert!(draft.is_empty());
    }

    /// Pending counts writes, not groups: two edits on one group are two.
    #[test]
    fn pending_counts_writes_not_groups() {
        let live = vec![live_group(
            5,
            "Satgas A",
            GroupKind::SatuanTugas,
            None,
            None,
            &[],
        )];
        let mut draft = GroupsDraft::new();
        draft.seed(&live);
        draft.rename(5, "Satgas Alpha").unwrap();
        draft.set_commander(5, Some(9)).unwrap();
        assert_eq!(draft.pending(&live), 2);
    }

    /// The read accessors a UI draws from: staged rows are the work, the
    /// frame is what the server said, and a unit's owner is one answer.
    #[test]
    fn read_accessors_separate_staged_work_from_the_frame() {
        let live = vec![live_group(
            5,
            "Satgas A",
            GroupKind::SatuanTugas,
            None,
            None,
            &[7],
        )];
        let mut draft = GroupsDraft::new();
        draft.seed(&live);
        assert_eq!(draft.unit_owner(7), Some(5));
        assert!(draft.unit_in_group(7));
        assert_eq!(draft.frame_groups().count(), 1);
        assert_eq!(draft.staged_groups().count(), 0);
        draft.set_commander(5, Some(9)).unwrap();
        assert_eq!(draft.staged_groups().count(), 1);
        assert_eq!(draft.get(5).map(|g| g.commander_id), Some(Some(9)));
        assert!(!draft.is_new(5));
    }

    // -- the membership join ----------------------------------------------
    //
    // The server publishes a unit's group on the unit's own row, so the
    // membership is a join away. Without the join every group reads as
    // "0 hulls" and the diff cannot see a unit the server already places.

    /// A unit that names a group joins that group, and one that names a
    /// group nobody holds joins nothing.
    #[test]
    fn a_units_group_is_joined_onto_the_node() {
        let mut groups = vec![
            live_group(1, "Satgas A", GroupKind::SatuanTugas, None, None, &[]),
            live_group(2, "Satgas B", GroupKind::SatuanTugas, None, None, &[]),
        ];
        join_members(&mut groups, &[(7, 2), (8, 1), (9, 99)]);
        assert_eq!(
            groups
                .iter()
                .find(|g| g.group_id == 2)
                .map(|g| g.units.clone()),
            Some(vec![7]),
            "the hull that names the node is in it"
        );
        assert_eq!(
            groups
                .iter()
                .find(|g| g.group_id == 1)
                .map(|g| g.units.clone()),
            Some(vec![8]),
            "and the hull that names the other root is in that one"
        );
    }

    /// A hull in no group is in none of them: the join does not invent a
    /// group for it.
    #[test]
    fn a_hull_in_no_group_belongs_to_none() {
        let mut groups = vec![live_group(
            1,
            "OpGab",
            GroupKind::OperasiGabungan,
            None,
            None,
            &[],
        )];
        join_members(&mut groups, &[]);
        assert!(groups[0].units.is_empty());
    }
}
