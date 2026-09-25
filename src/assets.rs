//! Read-only resources compiled into every tfg executable.
//!
//! Keeping these bytes behind one module makes installed builds independent
//! of the source tree while development tools can still update the files
//! from which Cargo compiles them.

pub const CATALOG_JSON: &str = include_str!("../assets/catalog.json");
pub const FLEET_JSON: &str = include_str!("../assets/fleet.json");
pub const LAND_JSON: &str = include_str!("../assets/ne_50m_land.json");
pub const MAP_SEED: &[u8] = include_bytes!("../assets/tiles-cache.seed.sqlite");

/// Built-in offline scenarios and their JSON. A caller supplies the
/// scenario stem, with or without `.json`; unknown names fail rather than
/// reading a path.
pub const BUILT_IN_SCENARIOS: [(&str, &str); 4] = [
    ("empty", include_str!("../scenarios/empty.json")),
    ("surge", include_str!("../scenarios/surge.json")),
    ("ghost", include_str!("../scenarios/ghost.json")),
    ("dark", include_str!("../scenarios/dark.json")),
];

/// Return one built-in scenario's JSON, or `None` for an unknown name.
pub fn scenario_json(name: &str) -> Option<&'static str> {
    let stem = name.trim().strip_suffix(".json").unwrap_or(name.trim());
    BUILT_IN_SCENARIOS
        .iter()
        .find_map(|(candidate, json)| (*candidate == stem).then_some(*json))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_built_in_scenario_is_valid_json() {
        for (name, text) in BUILT_IN_SCENARIOS {
            let fixture: serde_json::Value = serde_json::from_str(text).expect("valid fixture");
            assert!(fixture["frames"].is_array(), "{name} has frames");
            assert_eq!(scenario_json(&format!("{name}.json")), Some(text));
        }
        assert_eq!(scenario_json("missing"), None);
    }
}
