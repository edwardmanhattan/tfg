//! THE FLEET REGISTER, as a CASCADE (`§5.1`).
//!
//! Stage 02's register is five columns that narrow each other — DOMAIN,
//! KATEGORI, TIPE, KELAS, ASET — and the sheet is emphatic about what they are
//! not: **a cascade, not tabs**. Picking a rung truncates the path below it,
//! because a class cannot be answered before its type and a type cannot be
//! answered before its category.
//!
//! The API is five reads (`/units/asset-groups`, `/unit-categories`,
//! `/unit-types`, `/unit-classes`, `/units`: `§11`), but the console syncs the
//! whole register anyway, so the cascade is a FOLD over what is already in
//! memory. That is why this module is pure: no reads, no requests, a table in
//! and columns out, which is also what lets the counts be tested.
//!
//! TWO RULES THAT COME FROM THE SHEET AND ARE NOT OBVIOUS:
//!
//! - **A zero is a number.** Every group the endpoint returns is drawn, even
//!   one holding nothing (`Aset Lainnya` is returned empty on purpose). A
//!   column that hid its zeroes would be a column that looks like a read
//!   failure.
//! - **Search ignores the cascade.** It searches the REGISTER, matching hull
//!   name, hull number and class name — the placeholder's own promise
//!   (`SEARCH HULL, CLASS, PENNANT …`). `GET /units?search=` matches only two
//!   of those three, which is why the search is local.

/// One rung of the cascade, ready to draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rung {
    pub id: i64,
    pub label: String,
    /// How many hulls are reachable through this rung.
    pub count: usize,
}

/// Where the operator is in the cascade.
///
/// `None` at a rung means the rung has not been answered yet; the columns
/// below an unanswered rung are empty, which is what a cascade does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Path {
    pub domain: Option<i64>,
    pub category: Option<i64>,
    pub kind: Option<i64>,
    pub class: Option<i64>,
}

impl Path {
    /// Pick a rung, TRUNCATING everything below it.
    ///
    /// The truncation is the whole difference between a cascade and a set of
    /// filters: picking a different domain cannot leave a class selected that
    /// domain does not contain.
    pub fn pick(&self, rung: usize, id: i64) -> Path {
        let mut next = self.clone();
        match rung {
            0 => {
                next.domain = Some(id);
                next.category = None;
                next.kind = None;
                next.class = None;
            }
            1 => {
                next.category = Some(id);
                next.kind = None;
                next.class = None;
            }
            2 => {
                next.kind = Some(id);
                next.class = None;
            }
            3 => next.class = Some(id),
            _ => {}
        }
        next
    }

    /// How many rungs are answered.
    pub fn depth(&self) -> usize {
        usize::from(self.domain.is_some())
            + usize::from(self.category.is_some())
            + usize::from(self.kind.is_some())
            + usize::from(self.class.is_some())
    }
}

/// One group: `Aset Laut`, `Aset Udara`, `Aset Darat`, `Aset Lainnya`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub id: i64,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Category {
    pub id: i64,
    pub group: i64,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kind {
    pub id: i64,
    pub category: i64,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HullClass {
    pub id: i64,
    pub kind: i64,
    pub label: String,
}

/// One hull of the register: what a card in the ASET column shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hull {
    pub id: i64,
    pub class: i64,
    /// `KRI AHMAD YANI` — the card's bold line.
    pub name: String,
    /// `AHMAD YANI` — the class's own name, on the card's third line.
    pub class_label: String,
    /// `351` — the pennant, right-aligned on the card.
    pub number: String,
}

/// The whole register, as one foldable table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Register {
    pub groups: Vec<Group>,
    pub categories: Vec<Category>,
    pub kinds: Vec<Kind>,
    pub classes: Vec<HullClass>,
    pub hulls: Vec<Hull>,
}

impl Register {
    pub fn hull_total(&self) -> usize {
        self.hulls.len()
    }

    /// The hulls in `class`, in pennant order — the ASET column's card order.
    pub fn hulls_of(&self, class: i64) -> Vec<&Hull> {
        let mut at: Vec<&Hull> = self.hulls.iter().filter(|h| h.class == class).collect();
        at.sort_by(|a, b| a.number.cmp(&b.number));
        at
    }

    /// The five columns for `path`.
    ///
    /// A column is drawn only when the rung above it is answered — an empty
    /// column is the honest picture of "not answered yet", and the sheet draws
    /// them all anyway because it is explaining the shape.
    pub fn cascade(&self, path: &Path) -> Cascade {
        let mut columns: Vec<Vec<Rung>> = Vec::with_capacity(4);

        columns.push(
            self.groups
                .iter()
                .map(|g| Rung {
                    id: g.id,
                    label: g.label.clone(),
                    count: self.hulls_below_group(g.id),
                })
                .collect(),
        );

        let categories: Vec<Rung> = match path.domain {
            None => Vec::new(),
            Some(domain) => self
                .categories
                .iter()
                .filter(|c| c.group == domain)
                .map(|c| Rung {
                    id: c.id,
                    label: c.label.clone(),
                    count: self.hulls_below_category(c.id),
                })
                .collect(),
        };
        columns.push(categories);

        let kinds: Vec<Rung> = match path.category {
            None => Vec::new(),
            Some(category) => self
                .kinds
                .iter()
                .filter(|k| k.category == category)
                .map(|k| Rung {
                    id: k.id,
                    label: k.label.clone(),
                    count: self.hulls_below_kind(k.id),
                })
                .collect(),
        };
        columns.push(kinds);

        let classes: Vec<Rung> = match path.kind {
            None => Vec::new(),
            Some(kind) => self
                .classes
                .iter()
                .filter(|c| c.kind == kind)
                .map(|c| Rung {
                    id: c.id,
                    label: c.label.clone(),
                    count: self.hulls_of(c.id).len(),
                })
                .collect(),
        };
        columns.push(classes);

        Cascade {
            columns,
            breadcrumb: self.breadcrumb(path),
            total: self.hull_total(),
            hulls: match path.class {
                None => Vec::new(),
                Some(class) => self.hulls_of(class).into_iter().cloned().collect(),
            },
        }
    }

    /// `LAUT ▸ FRIGATE ▸ FRIGAT SERBAGUNA ▸ AHMAD YANI ▸ 6 HULLS`.
    pub fn breadcrumb(&self, path: &Path) -> Vec<Crumb> {
        let mut at = Vec::new();
        if let Some(id) = path.domain {
            if let Some(g) = self.groups.iter().find(|g| g.id == id) {
                at.push(Crumb {
                    rung: 0,
                    label: g.label.clone(),
                    count: self.hulls_below_group(id),
                });
            }
        }
        if let Some(id) = path.category {
            if let Some(c) = self.categories.iter().find(|c| c.id == id) {
                at.push(Crumb {
                    rung: 1,
                    label: c.label.clone(),
                    count: self.hulls_below_category(id),
                });
            }
        }
        if let Some(id) = path.kind {
            if let Some(k) = self.kinds.iter().find(|k| k.id == id) {
                at.push(Crumb {
                    rung: 2,
                    label: k.label.clone(),
                    count: self.hulls_below_kind(id),
                });
            }
        }
        if let Some(id) = path.class {
            if let Some(c) = self.classes.iter().find(|c| c.id == id) {
                at.push(Crumb {
                    rung: 3,
                    label: c.label.clone(),
                    count: self.hulls_of(id).len(),
                });
            }
        }
        at
    }

    /// The longest valid prefix of `path`.
    ///
    /// A PATH CAN GO STALE: the fixture's ids and the live drill's ids are not
    /// the same numbers, and a stage that opened on a fixture path would draw
    /// a breadcrumb naming rungs the account does not have — or, worse, a
    /// selection on a group that happens to share an id. Resolving keeps what
    /// still exists and drops the rest, so the cascade is always a walk over
    /// THIS register.
    pub fn resolve(&self, path: &Path) -> Path {
        let mut at = Path::default();
        let Some(domain) = path.domain else {
            return at;
        };
        if !self.groups.iter().any(|g| g.id == domain) {
            return at;
        }
        at = at.pick(0, domain);
        let Some(category) = path.category else {
            return at;
        };
        if !self
            .categories
            .iter()
            .any(|c| c.id == category && c.group == domain)
        {
            return at;
        }
        at = at.pick(1, category);
        let Some(kind) = path.kind else {
            return at;
        };
        if !self
            .kinds
            .iter()
            .any(|k| k.id == kind && k.category == category)
        {
            return at;
        }
        at = at.pick(2, kind);
        let Some(class) = path.class else {
            return at;
        };
        if self.classes.iter().any(|c| c.id == class && c.kind == kind) {
            at = at.pick(3, class);
        }
        at
    }

    /// The hulls matching `term` in NAME, NUMBER or CLASS NAME.
    ///
    /// LOCAL, over the synced register, because the placeholder promises three
    /// fields and the endpoint matches two. Empty term matches nothing: the
    /// search state is a state, not the whole register.
    pub fn search(&self, term: &str) -> Vec<&Hull> {
        let term = term.trim().to_lowercase();
        if term.is_empty() {
            return Vec::new();
        }
        let mut at: Vec<&Hull> = self
            .hulls
            .iter()
            .filter(|h| {
                h.name.to_lowercase().contains(&term)
                    || h.number.to_lowercase().contains(&term)
                    || h.class_label.to_lowercase().contains(&term)
            })
            .collect();
        at.sort_by(|a, b| a.number.cmp(&b.number));
        at
    }

    fn hulls_below_group(&self, group: i64) -> usize {
        let categories: Vec<i64> = self
            .categories
            .iter()
            .filter(|c| c.group == group)
            .map(|c| c.id)
            .collect();
        let kinds: Vec<i64> = self
            .kinds
            .iter()
            .filter(|k| categories.contains(&k.category))
            .map(|k| k.id)
            .collect();
        let classes: Vec<i64> = self
            .classes
            .iter()
            .filter(|c| kinds.contains(&c.kind))
            .map(|c| c.id)
            .collect();
        self.hulls
            .iter()
            .filter(|h| classes.contains(&h.class))
            .count()
    }

    fn hulls_below_category(&self, category: i64) -> usize {
        let kinds: Vec<i64> = self
            .kinds
            .iter()
            .filter(|k| k.category == category)
            .map(|k| k.id)
            .collect();
        let classes: Vec<i64> = self
            .classes
            .iter()
            .filter(|c| kinds.contains(&c.kind))
            .map(|c| c.id)
            .collect();
        self.hulls
            .iter()
            .filter(|h| classes.contains(&h.class))
            .count()
    }

    fn hulls_below_kind(&self, kind: i64) -> usize {
        let classes: Vec<i64> = self
            .classes
            .iter()
            .filter(|c| c.kind == kind)
            .map(|c| c.id)
            .collect();
        self.hulls
            .iter()
            .filter(|h| classes.contains(&h.class))
            .count()
    }
}

/// One rung of the breadcrumb: what it says, and how many hulls it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crumb {
    pub rung: usize,
    pub label: String,
    pub count: usize,
}

/// The five columns, the breadcrumb and the cards, for one path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cascade {
    pub columns: Vec<Vec<Rung>>,
    pub breadcrumb: Vec<Crumb>,
    pub total: usize,
    pub hulls: Vec<Hull>,
}

impl Cascade {
    /// `6 HULLS`, the breadcrumb's last term.
    pub fn hulls_label(&self) -> String {
        match self.hulls.len() {
            1 => "1 HULL".to_string(),
            n => format!("{n} HULLS"),
        }
    }

    /// The count on a column's header: `DOMAIN 3`, `KATEGORI 10`.
    pub fn column_count(&self, rung: usize) -> usize {
        self.columns.get(rung).map(Vec::len).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A register with the sheet's own shape: three groups, and the cascade
    /// that runs off them. Small enough to count by hand.
    fn register() -> Register {
        Register {
            groups: vec![
                Group {
                    id: 1,
                    label: "LAUT".into(),
                },
                Group {
                    id: 2,
                    label: "UDARA".into(),
                },
                Group {
                    id: 3,
                    label: "DARAT".into(),
                },
                Group {
                    id: 4,
                    label: "LAINNYA".into(),
                },
            ],
            categories: vec![
                Category {
                    id: 10,
                    group: 1,
                    label: "KAPAL PERANG".into(),
                },
                Category {
                    id: 11,
                    group: 2,
                    label: "PESAWAT".into(),
                },
            ],
            kinds: vec![
                Kind {
                    id: 20,
                    category: 10,
                    label: "FRIGATE".into(),
                },
                Kind {
                    id: 21,
                    category: 10,
                    label: "CORVETTE".into(),
                },
                Kind {
                    id: 22,
                    category: 11,
                    label: "PATROL UDARA".into(),
                },
            ],
            classes: vec![
                HullClass {
                    id: 30,
                    kind: 20,
                    label: "FRIGAT SERBAGUNA".into(),
                },
                HullClass {
                    id: 31,
                    kind: 21,
                    label: "SIGMAS".into(),
                },
                HullClass {
                    id: 32,
                    kind: 22,
                    label: "CN-235".into(),
                },
            ],
            hulls: vec![
                Hull {
                    id: 100,
                    class: 30,
                    name: "KRI AHMAD YANI".into(),
                    class_label: "AHMAD YANI".into(),
                    number: "351".into(),
                },
                Hull {
                    id: 101,
                    class: 30,
                    name: "KRI SLAMET RIYADI".into(),
                    class_label: "AHMAD YANI".into(),
                    number: "352".into(),
                },
                Hull {
                    id: 102,
                    class: 31,
                    name: "KRI SIGMAS".into(),
                    class_label: "SIGMAS".into(),
                    number: "401".into(),
                },
                Hull {
                    id: 103,
                    class: 32,
                    name: "PESAWAT UDARA 1".into(),
                    class_label: "CN-235".into(),
                    number: "A-01".into(),
                },
            ],
        }
    }

    /// A ZERO IS A NUMBER: the fourth group holds nothing and is still drawn,
    /// because the endpoint returns it and a missing row reads as a failure.
    #[test]
    fn an_empty_group_is_drawn_with_a_zero() {
        let cascade = register().cascade(&Path::default());
        let domain = &cascade.columns[0];
        assert_eq!(domain.len(), 4, "every group the endpoint returns");
        assert_eq!(domain[3].label, "LAINNYA");
        assert_eq!(domain[3].count, 0);
        assert_eq!(domain[0].count, 3, "LAUT holds the three surface hulls");
    }

    /// PICKING A RUNG TRUNCATES THE PATH BELOW IT — the whole difference
    /// between a cascade and a set of tabs.
    #[test]
    fn picking_a_rung_truncates_everything_below() {
        let path = Path::default()
            .pick(0, 1)
            .pick(1, 10)
            .pick(2, 20)
            .pick(3, 30);
        assert_eq!(path.depth(), 4);
        let next = path.pick(0, 2);
        assert_eq!(next.depth(), 1, "a new domain answers nothing below it");
        assert_eq!(next.category, None);
        assert_eq!(next.kind, None);
        assert_eq!(next.class, None);
    }

    /// The columns narrow each other, and the counts are the hulls reachable
    /// through each rung — not the rung's own row count.
    #[test]
    fn the_columns_narrow_and_the_counts_are_hulls() {
        let path = Path::default()
            .pick(0, 1)
            .pick(1, 10)
            .pick(2, 20)
            .pick(3, 30);
        let cascade = register().cascade(&path);
        assert_eq!(cascade.column_count(0), 4);
        assert_eq!(cascade.column_count(1), 1, "LAUT has one category here");
        assert_eq!(cascade.column_count(2), 2, "FRIGATE and CORVETTE");
        assert_eq!(cascade.column_count(3), 1, "FRIGAT SERBAGUNA");
        assert_eq!(cascade.hulls.len(), 2, "AHMAD YANI and SLAMET RIYADI");
        assert_eq!(cascade.hulls_label(), "2 HULLS");
    }

    /// An unanswered rung draws an empty column rather than a guess.
    #[test]
    fn an_unanswered_rung_is_an_empty_column() {
        let cascade = register().cascade(&Path::default());
        assert_eq!(cascade.column_count(1), 0);
        assert!(cascade.breadcrumb.is_empty());
        assert_eq!(cascade.total, 4);
    }

    /// The breadcrumb is the path, with a count at every rung.
    #[test]
    fn the_breadcrumb_is_the_path_with_counts() {
        let path = Path::default().pick(0, 1).pick(1, 10).pick(2, 20);
        let crumb = register().breadcrumb(&path);
        assert_eq!(crumb.len(), 3);
        assert_eq!(crumb[0].label, "LAUT");
        assert_eq!(crumb[0].count, 3);
        assert_eq!(crumb[2].label, "FRIGATE");
        assert_eq!(crumb[2].count, 2);
    }

    /// SEARCH IGNORES THE CASCADE and matches name, number AND class — the
    /// placeholder's three fields, which is why it cannot be the endpoint's
    /// two-field search.
    #[test]
    fn search_matches_name_number_and_class() {
        let register = register();
        // Two: the hull's own name AND its class's name, which the placeholder
        // promises as separate fields and the endpoint would not match at all.
        assert_eq!(register.search("ahmad").len(), 2, "by hull name and class");
        assert_eq!(register.search("401").len(), 1, "by pennant");
        assert_eq!(
            register.search("sigmas").len(),
            1,
            "the hull, by class name too"
        );
        assert_eq!(register.search("  ").len(), 0, "an empty search is a state");
        assert_eq!(register.search("zzz").len(), 0);
    }

    /// A PATH FROM ANOTHER REGISTER RESOLVES TO ITS VALID PREFIX, not to a
    /// selection on a lookalike id.
    #[test]
    fn a_stale_path_keeps_only_what_still_exists() {
        let register = register();
        // 1 and 10 exist here; 20 is a foreign kind (it is 21 in this table).
        let stale = Path::default()
            .pick(0, 1)
            .pick(1, 10)
            .pick(2, 999)
            .pick(3, 30);
        assert_eq!(register.resolve(&stale).depth(), 2);
        let foreign = Path::default().pick(0, 42);
        assert_eq!(register.resolve(&foreign).depth(), 0, "an unknown group");
        let good = Path::default()
            .pick(0, 1)
            .pick(1, 10)
            .pick(2, 20)
            .pick(3, 30);
        assert_eq!(register.resolve(&good).depth(), 4, "a valid path is kept");
    }

    /// Cards are in pennant order, so the grid does not reshuffle between
    /// renders.
    #[test]
    fn the_hulls_are_in_pennant_order() {
        let register = register();
        let at = register.hulls_of(30);
        assert_eq!(at[0].number, "351");
        assert_eq!(at[1].number, "352");
    }
}
