//! Runtime self-check (`--check-runtime`), shared by both consoles.
//!
//! Moved off the egui binary for the cutover (U8 prep A), verbatim but for
//! one parameter: the binary name in the report line, because the check
//! proves the RUNTIME (assets, tile seed, local store) rather than the
//! toolkit, and both consoles ship the same runtime. `tfg` prints `tfg`,
//! `tfg-gpui` prints `tfg-gpui`.

use crate::assets;
use crate::backend::FileReplay;
use crate::catalog::Catalog;
use crate::fleet::Fleet;
use crate::land::Land;
use crate::map_render;
use crate::paths::AppPaths;
use crate::store;

fn validate_map_seed(path: &std::path::Path) -> Result<(), String> {
    map_render::validate_cache(path)
}

pub fn runtime_self_check(paths: &AppPaths, binary: &str) -> Result<(), String> {
    let catalog = Catalog::from_default_asset()?;
    let fleet = Fleet::from_default_asset()?;
    Land::from_default_asset()?;
    let scenario = if std::env::var("TFG_BACKEND_URL").is_ok() {
        "empty".to_string()
    } else {
        std::env::var("TFG_SCENARIO").unwrap_or_else(|_| "empty".to_string())
    };
    for (name, json) in assets::BUILT_IN_SCENARIOS {
        FileReplay::from_json(json).map_err(|e| format!("embedded scenario {name}: {e}"))?;
    }
    let replay = FileReplay::from_json(
        assets::scenario_json(&scenario)
            .ok_or_else(|| format!("unknown built-in scenario {scenario}"))?,
    )?;
    let embedded_seed = paths.data_dir.join(format!(
        "maps/.tfg-seed-check-{}.sqlite",
        std::process::id()
    ));
    std::fs::write(&embedded_seed, assets::MAP_SEED)
        .map_err(|e| format!("embedded map seed check write failed: {e}"))?;
    let embedded_result = validate_map_seed(&embedded_seed);
    let _ = std::fs::remove_file(&embedded_seed);
    embedded_result?;
    let map_cache = map_render::prepare_runtime_cache(&paths.map_cache)?;
    validate_map_seed(&map_cache)?;
    let _store = store::open(&paths.local_db)?;
    println!(
        "{binary} {} runtime ok: {} classes, {} fleet units, land loaded, {} replay frames",
        env!("CARGO_PKG_VERSION"),
        catalog.ship_classes().len(),
        fleet.len(),
        replay.frame_count(),
    );
    println!("data: {}", paths.data_dir.display());
    println!("map cache: {}", map_cache.display());
    Ok(())
}
