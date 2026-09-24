//! Unit taxonomy catalog (grill #18, ADR-0005; fleet data task #28).
//!
//! Three levels, dev-type stripped:
//! - **Category** — what the unit is in the world (`Ship`/`Plane`/`Tank`/
//!   `Port`). Determines WHICH stat keys exist (the schema).
//! - **Class** — the stat VALUES (abilities): `mandau-psm-mk-iii` 33 kn vs
//!   `cakra-type-209-1300` 11 kn. All types under a class behave identically
//!   in the sim.
//! - **Type** — flavor only: the player-facing designation, no mechanics.
//!
//! Backed by `assets/catalog.json` (data-driven, like scenarios), generated
//! from the TNI AL fleet sheet by `scripts/fleet_import.py` (task #28):
//! 41 ship classes, one per sheet Kelas. Stats are a generic map (user
//! decision) with schema validation at load: every class must define all
//! `required_stats` of its category, fails loud otherwise.
//!
//! The sim reads only `speed_kn` (order cap); `cruise_kn`/`range_nm` ride
//! along as validated capabilities for the next slice (default transit
//! speed, patrol radius). Hull instances live in `assets/fleet.json`
//! (see [`crate::fleet`]), not here.

use std::collections::HashMap;

use serde::Deserialize;

/// What a unit is in the world. Reserves all four from the #14 sketch;
/// only Ship is implemented in the sandbox for now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Ship,
    Plane,
    Tank,
    Port,
}

/// Category schema: which stat keys a unit of this kind must/may carry.
#[derive(Debug, Clone, Deserialize)]
pub struct StatSchema {
    #[serde(default)]
    pub required_stats: Vec<String>,
    #[serde(default)]
    pub optional_stats: Vec<String>,
}

/// A catalog class: the stat VALUES (abilities) for all units of this
/// family, plus the flavor types grouped under it.
#[derive(Debug, Clone, Deserialize)]
pub struct Class {
    pub id: String,
    pub category: Category,
    pub name: String,
    pub stats: HashMap<String, f64>,
    #[serde(default)]
    pub types: Vec<String>,
    /// Minos spec version this class was synced from (H10). Zero for
    /// bundled asset classes — the asset carries no version, and a zero
    /// version never compares as newer than anything.
    #[serde(default)]
    pub version: i64,
}

impl Class {
    /// A player-facing type label: the first flavor type, or the class
    /// name when the catalog lists none.
    pub fn display_type(&self) -> &str {
        self.types.first().map(String::as_str).unwrap_or(&self.name)
    }
}

/// The loaded catalog: category schemas + class stat rows.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    schemas: HashMap<Category, StatSchema>,
    classes: Vec<Class>,
}

impl Catalog {
    /// Parse + validate a catalog document. Fails loud: unknown class
    /// category, or a class missing a required stat key of its category.
    pub fn parse(text: &str) -> Result<Self, String> {
        #[derive(Deserialize)]
        struct Doc {
            categories: HashMap<Category, StatSchema>,
            classes: Vec<Class>,
        }
        let doc: Doc = serde_json::from_str(text).map_err(|e| e.to_string())?;
        for class in &doc.classes {
            let Some(schema) = doc.categories.get(&class.category) else {
                return Err(format!("class {}: unknown category", class.id));
            };
            for key in &schema.required_stats {
                if !class.stats.contains_key(key) {
                    return Err(format!(
                        "class {} ({}): missing required stat `{key}`",
                        class.id, class.name
                    ));
                }
            }
        }
        Ok(Self { schemas: doc.categories, classes: doc.classes })
    }

    /// Load the committed catalog asset.
    pub fn from_default_asset() -> Result<Self, String> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/catalog.json");
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("catalog asset missing ({path:?}): {e}"))?;
        Self::parse(&text)
    }

    /// Ship classes, in catalog order (UI selector source).
    pub fn ship_classes(&self) -> Vec<&Class> {
        self.classes.iter().filter(|c| c.category == Category::Ship).collect()
    }

    pub fn class(&self, id: &str) -> Option<&Class> {
        self.classes.iter().find(|c| c.id == id)
    }

    /// Stats resolution for register hulls (picker cutover ticket):
    /// match a Minos class name to a catalog class (case-insensitive).
    /// None means no sim stats exist for that class — placement refuses
    /// loudly rather than inventing abilities.
    pub fn find_class_by_name(&self, name: &str) -> Option<&Class> {
        let want = name.trim().to_lowercase();
        self.classes.iter().find(|c| c.name.to_lowercase() == want)
    }

    /// Minos-synced class by the register's class id (H10): the
    /// namespaced `minos-<id>` row. Prefer this over
    /// [`find_class_by_name`](Self::find_class_by_name) whenever the
    /// id is known — a bundled asset class sharing the name must never
    /// shadow the synced figures.
    pub fn find_runtime_class(&self, minos_class_id: i64) -> Option<&Class> {
        let id = format!("minos-{minos_class_id}");
        self.classes.iter().find(|c| c.id == id)
    }

    /// Where a class's figures come from (H10): Minos spec versions
    /// ride runtime rows, bundled rows carry the asset. Recorded per
    /// class so placement and order surfaces can name their authority.
    pub fn class_source(class: &Class) -> &'static str {
        if class.id.starts_with("minos-") {
            "Minos spec"
        } else {
            "bundled asset"
        }
    }

    /// A class stat value; falls back to `default` when the key is
    /// absent (optional keys, or schemas without the stat).
    pub fn stat(class: &Class, key: &str, default: f64) -> f64 {
        class.stats.get(key).copied().unwrap_or(default)
    }

    /// Register a runtime class from synced spec figures (spec-sync
    /// ticket): a Minos hull's own numbers as a sim-drivable class.
    /// Id is namespaced (`minos-<class id>`) so register classes never
    /// collide with asset ids; re-fetch overwrites in place. The spec
    /// version rides along as the class's authority record (H10).
    pub fn upsert_runtime_class(
        &mut self,
        minos_class_id: i64,
        name: String,
        version: i64,
        speed_kn: f64,
        cruise_kn: f64,
        range_nm: f64,
    ) {
        let id = format!("minos-{minos_class_id}");
        let class = Class {
            id: id.clone(),
            category: Category::Ship,
            name,
            stats: HashMap::from([
                ("speed_kn".to_string(), speed_kn),
                ("cruise_kn".to_string(), cruise_kn),
                ("range_nm".to_string(), range_nm),
            ]),
            types: Vec::new(),
            version,
        };
        match self.classes.iter_mut().find(|c| c.id == id) {
            Some(slot) => *slot = class,
            None => self.classes.push(class),
        }
    }

    #[allow(dead_code)]
    pub fn schema(&self, category: Category) -> Option<&StatSchema> {
        self.schemas.get(&category)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_asset_loads_and_validates() {
        let cat = Catalog::from_default_asset().expect("asset parses + validates");
        let ships = cat.ship_classes();
        assert_eq!(ships.len(), 41, "one class per sheet Kelas");
        let mandau = cat.class("mandau-psm-mk-iii").unwrap();
        assert_eq!(mandau.category, Category::Ship);
        assert_eq!(Catalog::stat(mandau, "speed_kn", 0.0), 33.0);
        assert_eq!(mandau.display_type(), "Kapal cepat rudal/torpedo");
    }

    #[test]
    fn every_ship_class_carries_cruise_and_range() {
        // Generator contract (task #28): the sheet gives max/cruise/range
        // per hull, so every class row carries all three.
        let cat = Catalog::from_default_asset().expect("asset parses");
        for c in cat.ship_classes() {
            assert!(c.stats.contains_key("cruise_kn"), "{}: missing cruise_kn", c.id);
            assert!(c.stats.contains_key("range_nm"), "{}: missing range_nm", c.id);
            assert!(
                c.stats["cruise_kn"] <= c.stats["speed_kn"],
                "{}: cruise above max", c.id
            );
        }
    }

    #[test]
    fn find_class_by_name_matches_case_insensitively() {
        let cat = Catalog::from_default_asset().expect("asset parses");
        let yani = cat
            .find_class_by_name("ahmad yani / van speijk")
            .expect("Ahmad Yani found");
        assert_eq!(
            cat.find_class_by_name("  Ahmad Yani / Van Speijk ").map(|c| &c.id),
            Some(&yani.id)
        );
        assert!(cat.find_class_by_name("No Such Class").is_none());
    }

    #[test]
    fn runtime_class_registers_and_overwrites() {
        let mut cat = Catalog::from_default_asset().expect("asset parses");
        let n = cat.ship_classes().len();
        cat.upsert_runtime_class(7, "Ahmad Yani".into(), 3, 28.0, 18.0, 4000.0);
        assert_eq!(cat.ship_classes().len(), n + 1);
        let c = cat.class("minos-7").expect("runtime class");
        assert_eq!(Catalog::stat(c, "speed_kn", 0.0), 28.0);
        assert_eq!(c.version, 3, "spec version rides the class");
        assert_eq!(Catalog::class_source(c), "Minos spec");
        // Name match hits the runtime row for register resolution.
        assert_eq!(cat.find_class_by_name("ahmad yani").map(|c| &c.id), Some(&c.id));
        cat.upsert_runtime_class(7, "Ahmad Yani".into(), 4, 30.0, 18.0, 4000.0);
        assert_eq!(cat.ship_classes().len(), n + 1, "overwrite, no duplicate");
        assert_eq!(Catalog::stat(cat.class("minos-7").unwrap(), "speed_kn", 0.0), 30.0);
    }

    #[test]
    fn runtime_id_beats_bundled_name() {
        // H10: a bundled asset sharing the Minos class name must not
        // shadow the synced figures — resolve by namespaced id first.
        let mut cat = Catalog::from_default_asset().expect("asset parses");
        let bundled_name = "Martadinata / SIGMA 10514 PKR";
        let bundled_id = cat
            .find_class_by_name(bundled_name)
            .map(|c| c.id.clone())
            .expect("bundled row with this name");
        let bundled_speed = Catalog::stat(cat.class(&bundled_id).unwrap(), "speed_kn", 0.0);
        cat.upsert_runtime_class(9, bundled_name.into(), 2, bundled_speed + 10.0, 10.0, 1000.0);
        // Name lookup now hits one of the two same-named rows — the
        // namespaced id is the unambiguous one, and it carries Minos.
        let rt = cat.find_runtime_class(9).expect("namespaced lookup");
        assert_eq!(Catalog::stat(rt, "speed_kn", 0.0), bundled_speed + 10.0);
        assert_eq!(rt.version, 2);
        assert_eq!(Catalog::class_source(rt), "Minos spec");
        assert_eq!(
            Catalog::class_source(cat.class(&bundled_id).unwrap()),
            "bundled asset"
        );
    }

    #[test]
    fn missing_required_stat_fails_loud() {
        let doc = r#"{
            "categories": { "ship": { "required_stats": ["speed_kn"] } },
            "classes": [
                { "id": "broken", "category": "ship", "name": "Broken",
                  "stats": {}, "types": [] }
            ]
        }"#;
        let err = Catalog::parse(doc).unwrap_err();
        assert!(err.contains("missing required stat `speed_kn`"), "{err}");
    }

    #[test]
    fn unknown_category_fails_loud() {
        let doc = r#"{
            "categories": {},
            "classes": [
                { "id": "x", "category": "ship", "name": "X", "stats": {} }
            ]
        }"#;
        let err = Catalog::parse(doc).unwrap_err();
        assert!(err.contains("unknown category"), "{err}");
    }

    #[test]
    fn optional_stats_default_when_absent() {
        let cat = Catalog::from_default_asset().unwrap();
        let mandau = cat.class("mandau-psm-mk-iii").unwrap();
        // `capacity_t` is not a ship stat: absent -> fallback.
        assert_eq!(Catalog::stat(mandau, "capacity_t", 0.0), 0.0);
    }
}
