//! Local-first master-data store (master-data ticket).
//!
//! The backend owns reference truth; this sqlite file owns the working
//! copy. Reads serve the UI from disk, `sync_now` replaces whole tables
//! from the backend (server wins — v1 mirrors read-only, no local edits
//! flow upward), and vessel positions upsert per poll round so the last
//! known picture survives restarts. Session journals stay JSONL; this
//! file holds reference + standing data only.

use rusqlite::{Connection, OptionalExtension, params};

/// Local sqlite path, next to the session journals (same precedent).
pub fn local_db_path() -> std::path::PathBuf {
    std::path::PathBuf::from(format!(
        "{}/target/tfg-local.db",
        env!("CARGO_MANIFEST_DIR")
    ))
}

/// Open (creating) and migrate the store.
pub fn open(path: &std::path::Path) -> Result<Connection, String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS helpers (
            table_name TEXT NOT NULL, id INTEGER NOT NULL,
            name TEXT NOT NULL, id_name TEXT NOT NULL DEFAULT '',
            description_en TEXT NOT NULL DEFAULT '',
            is_system INTEGER NOT NULL DEFAULT 0,
            is_judge_side INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (table_name, id)
        );
        CREATE TABLE IF NOT EXISTS unit_categories (
            id INTEGER PRIMARY KEY, name TEXT NOT NULL,
            id_name TEXT NOT NULL DEFAULT '', description_en TEXT NOT NULL DEFAULT '',
            is_system INTEGER NOT NULL DEFAULT 0, type_count INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS unit_types (
            id INTEGER PRIMARY KEY, name TEXT NOT NULL,
            id_name TEXT NOT NULL DEFAULT '', description_en TEXT NOT NULL DEFAULT '',
            is_system INTEGER NOT NULL DEFAULT 0, category_id INTEGER,
            class_count INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS unit_classes (
            id INTEGER PRIMARY KEY, name TEXT NOT NULL,
            id_name TEXT NOT NULL DEFAULT '', type_id INTEGER,
            turn_rate REAL, hull_count INTEGER NOT NULL DEFAULT 0,
            image_url TEXT
        );
        CREATE TABLE IF NOT EXISTS units (
            id INTEGER PRIMARY KEY, name TEXT NOT NULL, hull_number TEXT,
            class_id INTEGER, status_id INTEGER,
            branch_id INTEGER, domain_id INTEGER
        );
        CREATE TABLE IF NOT EXISTS unit_specs (
            unit_id INTEGER NOT NULL, version INTEGER NOT NULL,
            is_current INTEGER NOT NULL DEFAULT 0, body TEXT NOT NULL,
            PRIMARY KEY (unit_id, version)
        );
        CREATE TABLE IF NOT EXISTS hierarchy_echelons (
            id INTEGER PRIMARY KEY, name TEXT NOT NULL,
            id_name TEXT NOT NULL DEFAULT '', description_en TEXT NOT NULL DEFAULT '',
            echelon_rank INTEGER NOT NULL DEFAULT 0,
            is_system INTEGER NOT NULL DEFAULT 0, note TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE IF NOT EXISTS vessel_positions (
            id_unit INTEGER PRIMARY KEY, name TEXT NOT NULL, hull_number TEXT,
            latitude REAL NOT NULL, longitude REAL NOT NULL,
            speed_kn REAL, course_deg REAL, accuracy_m REAL,
            recorded_at TEXT NOT NULL, received_at TEXT NOT NULL,
            backfilled INTEGER NOT NULL DEFAULT 0, synced_at TEXT NOT NULL
        );
        ",
    )
    .map_err(|e| e.to_string())?;
    Ok(conn)
}

/// Whole-table replace from a fresh fetch (server wins). Rows are built
/// by the caller as ordered params vectors per table.
pub fn replace_all(
    conn: &mut Connection,
    table: &str,
    columns: &str,
    placeholders: &str,
    rows: Vec<Vec<StoredValue>>,
) -> Result<usize, String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute(&format!("DELETE FROM {table}"), [])
        .map_err(|e| e.to_string())?;
    let mut stmt = tx
        .prepare(&format!("INSERT INTO {table} ({columns}) VALUES ({placeholders})"))
        .map_err(|e| e.to_string())?;
    let mut n = 0;
    for row in &rows {
        let params: Vec<&dyn rusqlite::ToSql> =
            row.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
        stmt.execute(params.as_slice()).map_err(|e| e.to_string())?;
        n += 1;
    }
    drop(stmt);
    tx.commit().map_err(|e| e.to_string())?;
    Ok(n)
}

/// A storable cell: ints, floats, text, or null.
#[derive(Debug, Clone)]
pub enum StoredValue {
    Int(i64),
    Real(f64),
    Text(String),
    Null,
}

impl rusqlite::ToSql for StoredValue {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        match self {
            StoredValue::Int(i) => Ok((*i).into()),
            StoredValue::Real(f) => Ok((*f).into()),
            StoredValue::Text(s) => Ok(s.as_str().into()),
            StoredValue::Null => Ok(rusqlite::types::Null.into()),
        }
    }
}

pub fn opt_int(v: Option<i64>) -> StoredValue {
    v.map(StoredValue::Int).unwrap_or(StoredValue::Null)
}

pub fn opt_real(v: Option<f64>) -> StoredValue {
    v.map(StoredValue::Real).unwrap_or(StoredValue::Null)
}

pub fn opt_text(v: Option<String>) -> StoredValue {
    v.map(StoredValue::Text).unwrap_or(StoredValue::Null)
}

/// Meta reads/writes (last_sync, per-table counts).
pub fn meta_set(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = ?2",
        params![key, value],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn meta_get(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| {
        r.get(0)
    })
    .optional()
    .map_err(|e| e.to_string())
}

/// Row counts per mirrored table, for the sync status line.
pub fn table_counts(conn: &Connection) -> Result<Vec<(String, i64)>, String> {
    let mut out = Vec::new();
    for t in [
        "helpers",
        "unit_categories",
        "unit_types",
        "unit_classes",
        "units",
        "unit_specs",
        "hierarchy_echelons",
        "vessel_positions",
    ] {
        let n: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        out.push((t.to_string(), n));
    }
    Ok(out)
}

/// Full sync (server wins, whole-table replace): helpers, hierarchy,
/// taxonomy, units. Returns per-table row counts. Records last_sync.
pub fn sync_from(
    master: &crate::backend::MinosMaster,
    token: &str,
    conn: &mut Connection,
) -> Result<Vec<(String, usize)>, String> {
    let mut counts = Vec::new();
    for table in [
        master.helpers(token)?,
        master.hierarchy(token)?,
        master.categories(token)?,
        master.types(token)?,
        master.classes(token)?,
        master.units(token)?,
    ] {
        let n = replace_all(conn, table.table, table.columns, table.placeholders, table.rows)?;
        counts.push((table.table.to_string(), n));
    }
    meta_set(conn, "last_sync", &crate::backend::now_ts())?;
    Ok(counts)
}

/// Upsert the standing picture (one row per vessel): wire fixes seen
/// this round. Cheap enough per poll tick; restarts resume warm.
pub fn upsert_positions(
    conn: &Connection,
    fixes: &[crate::geo::track::Fix],
    now: &str,
) -> Result<usize, String> {
    let mut n = 0;
    for f in fixes {
        // Decimal wire ids only: sim/replay names have no unit row and
        // must never collapse onto id 0 together.
        let Ok(id_unit) = f.ship_id.parse::<i64>() else {
            continue;
        };
        conn.execute(
            "INSERT INTO vessel_positions
             (id_unit, name, hull_number, latitude, longitude, speed_kn, course_deg,
              accuracy_m, recorded_at, received_at, backfilled, synced_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(id_unit) DO UPDATE SET
              name = excluded.name, hull_number = excluded.hull_number,
              latitude = excluded.latitude, longitude = excluded.longitude,
              speed_kn = excluded.speed_kn, course_deg = excluded.course_deg,
              accuracy_m = excluded.accuracy_m, recorded_at = excluded.recorded_at,
              received_at = excluded.received_at, backfilled = excluded.backfilled,
              synced_at = excluded.synced_at",
            rusqlite::params![
                id_unit,
                f.name.clone(),
                f.hull_number.clone(),
                f.position.latitude,
                f.position.longitude,
                f.speed_kn.map(|v| v as f64),
                f.heading_deg.map(|v| v as f64),
                f.accuracy_m.map(|v| v as f64),
                f.ts.clone(),
                f.received_at.clone(),
                if f.backfilled { 1 } else { 0 },
                now.to_string(),
            ],
        )
        .map_err(|e| e.to_string())?;
        n += 1;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_and_count_round_trip() {
        let mut conn = open(&std::path::PathBuf::from(":memory:")).expect("open");
        let n = replace_all(
            &mut conn,
            "unit_categories",
            "id, name",
            "?1, ?2",
            vec![
                vec![StoredValue::Int(1), StoredValue::Text("Ship".into())],
                vec![StoredValue::Int(2), StoredValue::Text("Plane".into())],
            ],
        )
        .expect("replace");
        assert_eq!(n, 2);
        let counts = table_counts(&conn).expect("counts");
        let cats = counts.iter().find(|(t, _)| t == "unit_categories").unwrap();
        assert_eq!(cats.1, 2);
        // Second replace wins wholesale (no merge, no ghosts).
        replace_all(
            &mut conn,
            "unit_categories",
            "id, name",
            "?1, ?2",
            vec![vec![StoredValue::Int(9), StoredValue::Text("Tank".into())]],
        )
        .expect("replace again");
        let counts = table_counts(&conn).expect("counts");
        let cats = counts.iter().find(|(t, _)| t == "unit_categories").unwrap();
        assert_eq!(cats.1, 1);
        meta_set(&conn, "last_sync", "2026-09-18T00:00:00Z").expect("meta set");
        assert_eq!(
            meta_get(&conn, "last_sync").expect("meta get").as_deref(),
            Some("2026-09-18T00:00:00Z")
        );
        assert_eq!(meta_get(&conn, "nope").expect("meta miss"), None);
    }
}
