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

    /// A class stat value; falls back to `default` when the key is
    /// absent (optional keys, or schemas without the stat).
    pub fn stat(class: &Class, key: &str, default: f64) -> f64 {
        class.stats.get(key).copied().unwrap_or(default)
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
