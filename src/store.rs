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
        -- Operator-authored branch → category ownership (setup-overhaul
        -- picker): declared mapping, not derived from hulls. Synced per
        -- branch from GET /unit-categories?id_service_branch=.
        CREATE TABLE IF NOT EXISTS branch_categories (
            branch_id INTEGER NOT NULL, category_id INTEGER NOT NULL,
            PRIMARY KEY (branch_id, category_id)
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
        "branch_categories",
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
    // Declared branch → category ownership (setup-overhaul picker):
    // one filtered fetch per branch, ids only (rows already mirrored).
    let branch_ids: Vec<i64> = {
        let mut stmt = conn
            .prepare("SELECT id FROM helpers WHERE table_name = 'service_branches' ORDER BY id")
            .map_err(|e| e.to_string())?;
        stmt.query_map([], |r| r.get(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<i64>, _>>()
            .map_err(|e| e.to_string())?
    };
    let mut map_n = 0;
    for bid in branch_ids {
        let ids = master.category_ids_for_branch(token, bid)?;
        map_n += replace_branch_categories(conn, bid, &ids)?;
    }
    counts.push(("branch_categories".to_string(), map_n));
    meta_set(conn, "last_sync", &crate::backend::now_ts())?;
    Ok(counts)
}

/// Replace one branch's declared category set (server wins).
pub fn replace_branch_categories(
    conn: &Connection,
    branch_id: i64,
    category_ids: &[i64],
) -> Result<usize, String> {
    conn.execute("DELETE FROM branch_categories WHERE branch_id = ?1", [branch_id])
        .map_err(|e| e.to_string())?;
    let mut n = 0;
    for cid in category_ids {
        conn.execute(
            "INSERT OR IGNORE INTO branch_categories (branch_id, category_id) VALUES (?1, ?2)",
            rusqlite::params![branch_id, cid],
        )
        .map_err(|e| e.to_string())?;
        n += 1;
    }
    Ok(n)
}

/// One drill-down row (setup-overhaul picker): taxonomy label in both
/// languages plus the header count for the next level.
#[derive(Debug, Clone, PartialEq)]
pub struct TaxRow {
    pub id: i64,
    pub name: String,
    pub id_name: String,
    pub count: i64,
}

fn tax_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<TaxRow> {
    Ok(TaxRow { id: r.get(0)?, name: r.get(1)?, id_name: r.get(2)?, count: r.get(3)? })
}

fn tax_rows(conn: &Connection, sql: &str, param: Option<i64>) -> Result<Vec<TaxRow>, String> {
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    // One fn item for both arms: two inline closures would be two
    // distinct types and the match would not compile.
    let rows = match param {
        Some(p) => stmt.query_map([p], tax_row),
        None => stmt.query_map([], tax_row),
    }
    .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// Branches with live local hull counts (COUNT of mirrored units).
pub fn tax_branches(conn: &Connection) -> Result<Vec<TaxRow>, String> {
    tax_rows(
        conn,
        "SELECT h.id, h.name, h.id_name, COUNT(u.id)
         FROM helpers h LEFT JOIN units u ON u.branch_id = h.id
         WHERE h.table_name = 'service_branches'
         GROUP BY h.id ORDER BY h.name",
        None,
    )
}

/// One branch's declared categories, with server type counts.
pub fn tax_categories(conn: &Connection, branch_id: i64) -> Result<Vec<TaxRow>, String> {
    tax_rows(
        conn,
        "SELECT c.id, c.name, c.id_name, c.type_count
         FROM unit_categories c JOIN branch_categories b ON b.category_id = c.id
         WHERE b.branch_id = ?1 ORDER BY c.name",
        Some(branch_id),
    )
}

/// One category's types, with server class counts.
pub fn tax_types(conn: &Connection, category_id: i64) -> Result<Vec<TaxRow>, String> {
    tax_rows(
        conn,
        "SELECT id, name, id_name, class_count FROM unit_types
         WHERE category_id = ?1 ORDER BY name",
        Some(category_id),
    )
}

/// One type's classes, with server hull counts.
pub fn tax_classes(conn: &Connection, type_id: i64) -> Result<Vec<TaxRow>, String> {
    tax_rows(
        conn,
        "SELECT id, name, id_name, hull_count FROM unit_classes
         WHERE type_id = ?1 ORDER BY name",
        Some(type_id),
    )
}

/// One class's hulls (leaf of the drill).
pub fn tax_units(conn: &Connection, class_id: i64) -> Result<Vec<StoreUnit>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT u.id, u.name, COALESCE(u.hull_number, ''),
                    u.class_id, COALESCE(c.name, '')
             FROM units u LEFT JOIN unit_classes c ON c.id = u.class_id
             WHERE u.class_id = ?1 ORDER BY u.name",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([class_id], |r| {
            let id: i64 = r.get(0)?;
            Ok(StoreUnit {
                id: id.to_string(),
                name: r.get(1)?,
                hull: r.get(2)?,
                class_id: r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                class_name: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// Whole-tree search hit (setup-overhaul grill): the hull plus its
/// breadcrumb path for the jump-to-match list.
#[derive(Debug, Clone, PartialEq)]
pub struct DrillHit {
    pub id: String,
    pub name: String,
    pub hull: String,
    pub class_name: String,
    pub type_name: String,
    pub category_name: String,
    pub branch_name: String,
}

impl DrillHit {
    /// Breadcrumb for the match list (id_name-first rendering happens
    /// in the UI; the mirror holds display names as synced).
    pub fn trail(&self) -> String {
        [self.branch_name.clone(), self.category_name.clone(), self.type_name.clone()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" / ")
    }
}

/// Hulls matching name/hull/class anywhere in the tree, capped.
pub fn tax_search(conn: &Connection, query: &str) -> Result<Vec<DrillHit>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT u.id, u.name, COALESCE(u.hull_number, ''), COALESCE(c.name, ''),
                    COALESCE(t.name, ''), COALESCE(cat.name, ''), COALESCE(h.name, '')
             FROM units u
             LEFT JOIN unit_classes c ON c.id = u.class_id
             LEFT JOIN unit_types t ON t.id = c.type_id
             LEFT JOIN unit_categories cat ON cat.id = t.category_id
             LEFT JOIN branch_categories bc ON bc.category_id = cat.id
             LEFT JOIN helpers h ON h.table_name = 'service_branches' AND h.id = bc.branch_id
             WHERE LOWER(u.name || ' ' || COALESCE(u.hull_number, '') || ' ' || COALESCE(c.name, ''))
                   LIKE ?1
             GROUP BY u.id ORDER BY u.name LIMIT 200",
        )
        .map_err(|e| e.to_string())?;
    let like = format!("%{}%", query.to_lowercase());
    let rows = stmt
        .query_map([like], |r| {
            let id: i64 = r.get(0)?;
            Ok(DrillHit {
                id: id.to_string(),
                name: r.get(1)?,
                hull: r.get(2)?,
                class_name: r.get(3)?,
                type_name: r.get(4)?,
                category_name: r.get(5)?,
                branch_name: r.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// Drill precondition: synced taxonomy present (independent of hulls —
/// the register below types may legitimately be empty).
pub fn has_taxonomy(conn: &Connection) -> Result<bool, String> {
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM unit_categories", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    Ok(n > 0)
}

/// One lookup vocabulary (session-users ticket): id, both names, and
/// the judge-side flag — e.g. `game_roles` for the role pick.
pub fn helper_list(
    conn: &Connection,
    table: &str,
) -> Result<Vec<(i64, String, String, bool)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, id_name, is_judge_side FROM helpers
             WHERE table_name = ?1 ORDER BY name",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([table], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)? != 0,
            ))
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
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

/// One register hull for the picker: Minos identity plus the class name
/// for display and stats resolution.
#[derive(Debug, Clone, PartialEq)]
pub struct StoreUnit {
    pub id: String,
    pub name: String,
    pub hull: String,
    pub class_id: i64,
    pub class_name: String,
}

/// Picker rows: register hulls with their class names, in name order.
pub fn fleet_units(conn: &Connection) -> Result<Vec<StoreUnit>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT u.id, u.name, COALESCE(u.hull_number, ''),
                    u.class_id, COALESCE(c.name, '')
             FROM units u LEFT JOIN unit_classes c ON c.id = u.class_id
             ORDER BY u.name",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            let id: i64 = r.get(0)?;
            Ok(StoreUnit {
                id: id.to_string(),
                name: r.get(1)?,
                hull: r.get(2)?,
                class_id: r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                class_name: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// One register hull by id (decimal string form).
pub fn fleet_unit(conn: &Connection, id: &str) -> Result<Option<StoreUnit>, String> {
    let Ok(want) = id.parse::<i64>() else {
        return Ok(None);
    };
    let mut stmt = conn
        .prepare(
            "SELECT u.id, u.name, COALESCE(u.hull_number, ''),
                    u.class_id, COALESCE(c.name, '')
             FROM units u LEFT JOIN unit_classes c ON c.id = u.class_id
             WHERE u.id = ?1",
        )
        .map_err(|e| e.to_string())?;
    stmt.query_row([want], |r| {
        Ok(StoreUnit {
            id: want.to_string(),
            name: r.get(1)?,
            hull: r.get(2)?,
            class_id: r.get::<_, Option<i64>>(3)?.unwrap_or(0),
            class_name: r.get(4)?,
        })
    })
    .optional()
    .map_err(|e| e.to_string())
}

/// Class dropdown source for the register picker.
pub fn unit_class_names(conn: &Connection) -> Result<Vec<(i64, String)>, String> {
    let mut stmt = conn
        .prepare("SELECT id, name FROM unit_classes ORDER BY name")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<(i64, String)>, _>>().map_err(|e| e.to_string())
}

/// Register size: the picker source switch (synced register wins,
// bundled assets seed).
pub fn units_count(conn: &Connection) -> Result<i64, String> {
    conn.query_row("SELECT COUNT(*) FROM units", [], |r| r.get(0))
        .map_err(|e| e.to_string())
}

/// All register hull ids, for the spec backfill.
pub fn unit_ids(conn: &Connection) -> Result<Vec<i64>, String> {
    let mut stmt = conn
        .prepare("SELECT id FROM units ORDER BY id")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<i64>, _>>().map_err(|e| e.to_string())
}

/// Store one hull's figures (spec-sync ticket): versioned JSON body.
/// Versions are immutable server-side, so rows only ever append.
pub fn store_spec(
    conn: &Connection,
    unit_id: i64,
    version: i64,
    is_current: bool,
    body: &str,
) -> Result<(), String> {
    // One current version at a time: a newer fetch demotes the old row.
    if is_current {
        conn.execute("UPDATE unit_specs SET is_current = 0 WHERE unit_id = ?1", [unit_id])
            .map_err(|e| e.to_string())?;
    }
    conn.execute(
        "INSERT INTO unit_specs (unit_id, version, is_current, body)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(unit_id, version) DO UPDATE SET
           is_current = excluded.is_current, body = excluded.body",
        rusqlite::params![unit_id, version, if is_current { 1 } else { 0 }, body],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Known spec versions for one hull (skip-list for the backfill).
pub fn spec_versions(conn: &Connection, unit_id: i64) -> Result<Vec<i64>, String> {
    let mut stmt = conn
        .prepare("SELECT version FROM unit_specs WHERE unit_id = ?1 ORDER BY version")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([unit_id], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<i64>, _>>().map_err(|e| e.to_string())
}

/// One hull's stored figures, parsed back out of the versioned body.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredFigures {
    pub unit_id: i64,
    pub version: i64,
    pub class_id: i64,
    pub class_name: String,
    pub speed_kn: Option<f64>,
    pub cruise_kn: Option<f64>,
    pub range_nm: Option<f64>,
}

/// Current figures for every stored hull (boot restore + picker).
/// Unparseable bodies are skipped loudly through the error — a corrupt
/// row must not poison the whole restore.
pub fn current_figures(conn: &Connection) -> Result<Vec<StoredFigures>, String> {
    let mut stmt = conn
        .prepare("SELECT unit_id, version, body FROM unit_specs WHERE is_current = 1")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            let unit_id: i64 = r.get(0)?;
            let version: i64 = r.get(1)?;
            let body: String = r.get(2)?;
            let v: serde_json::Value =
                serde_json::from_str(&body).map_err(|e| rusqlite::Error::FromSqlConversionFailure(
                    2,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                ))?;
            let num = |key: &str| v.get(key).and_then(|x| x.as_f64());
            Ok(StoredFigures {
                unit_id,
                version,
                class_id: v.get("class_id").and_then(|x| x.as_i64()).unwrap_or(0),
                class_name: v
                    .get("class_name")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                speed_kn: num("speed_kn"),
                cruise_kn: num("cruise_kn"),
                range_nm: num("range_nm"),
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
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

    #[test]
    fn fleet_units_joins_class_names() {
        let mut conn = open(&std::path::PathBuf::from(":memory:")).expect("open");
        replace_all(
            &mut conn,
            "unit_classes",
            "id, name",
            "?1, ?2",
            vec![vec![StoredValue::Int(5), StoredValue::Text("Sigma".into())]],
        )
        .expect("classes");
        replace_all(
            &mut conn,
            "units",
            "id, name, hull_number, class_id",
            "?1, ?2, ?3, ?4",
            vec![
                vec![
                    StoredValue::Int(13),
                    StoredValue::Text("KRI Ahmad Yani".into()),
                    StoredValue::Text("KRI-AH-YN".into()),
                    StoredValue::Int(5),
                ],
                vec![
                    StoredValue::Int(14),
                    StoredValue::Text("Bare Hull".into()),
                    StoredValue::Null,
                    StoredValue::Null,
                ],
            ],
        )
        .expect("units");
        assert_eq!(units_count(&conn).expect("count"), 2);
        let rows = fleet_units(&conn).expect("rows");
        // Name order: Bare Hull sorts before KRI Ahmad Yani.
        assert_eq!(rows[0].id, "14");
        assert_eq!(rows[0].class_name, "");
        assert_eq!(rows[1].class_name, "Sigma");
        let one = fleet_unit(&conn, "13").expect("lookup").expect("found");
        assert_eq!(one.name, "KRI Ahmad Yani");
        assert!(fleet_unit(&conn, "kri-x").expect("non-decimal").is_none());
        assert!(fleet_unit(&conn, "999").expect("unknown").is_none());
        let classes = unit_class_names(&conn).expect("classes");
        assert_eq!(classes, vec![(5, "Sigma".to_string())]);
    }

    #[test]
    fn specs_version_and_restore() {
        let conn = open(&std::path::PathBuf::from(":memory:")).expect("open");
        assert!(spec_versions(&conn, 13).expect("versions").is_empty());
        store_spec(&conn, 13, 1, true, r#"{"class_name":"Sigma","speed_kn":20.0}"#)
            .expect("store v1");
        store_spec(&conn, 13, 2, true, r#"{"class_name":"Sigma","speed_kn":22.0}"#)
            .expect("store v2");
        assert_eq!(spec_versions(&conn, 13).expect("versions"), vec![1, 2]);
        // Only one current version survives the overwrite.
        let figs = current_figures(&conn).expect("figures");
        assert_eq!(figs.len(), 1);
        assert_eq!(figs[0].version, 2);
        assert_eq!(figs[0].speed_kn, Some(22.0));
        assert_eq!(figs[0].class_name, "Sigma");
    }

    #[test]
    fn drill_readers_walk_branch_to_hull() {
        let conn = open(&std::path::PathBuf::from(":memory:")).expect("open");
        assert!(!has_taxonomy(&conn).expect("empty store"));
        conn.execute(
            "INSERT INTO helpers (table_name, id, name, id_name) VALUES ('service_branches', 1, 'Navy', 'TNI Angkatan Laut')",
            [],
        )
        .expect("branch");
        conn.execute(
            "INSERT INTO unit_categories (id, name, id_name, type_count) VALUES (2, 'Frigate', 'Frigat', 1)",
            [],
        )
        .expect("category");
        conn.execute(
            "INSERT INTO unit_types (id, name, id_name, category_id, class_count) VALUES (3, 'FFG', 'Frigat', 2, 1)",
            [],
        )
        .expect("type");
        conn.execute(
            "INSERT INTO unit_classes (id, name, id_name, type_id, hull_count) VALUES (5, 'Sigma', 'Sigma', 3, 1)",
            [],
        )
        .expect("class");
        conn.execute(
            "INSERT INTO units (id, name, hull_number, class_id, branch_id) VALUES (13, 'KRI Ahmad Yani', '381', 5, 1)",
            [],
        )
        .expect("hull");
        assert!(has_taxonomy(&conn).expect("tax"));
        // Mapping replace is idempotent over duplicate ids.
        replace_branch_categories(&conn, 1, &[2, 2]).expect("mapping");
        let branches = tax_branches(&conn).expect("branches");
        assert_eq!(branches.len(), 1);
        assert_eq!(branches[0].count, 1, "local hull count, not server");
        let cats = tax_categories(&conn, 1).expect("cats");
        assert_eq!(cats, vec![TaxRow { id: 2, name: "Frigate".into(), id_name: "Frigat".into(), count: 1 }]);
        assert!(tax_categories(&conn, 9).expect("other branch").is_empty());
        assert_eq!(tax_types(&conn, 2).expect("types").len(), 1);
        assert_eq!(tax_classes(&conn, 3).expect("classes").len(), 1);
        assert_eq!(tax_units(&conn, 5).expect("hulls").len(), 1);
        let hits = tax_search(&conn, "yani").expect("search");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].trail(), "Navy / Frigate / FFG");
        assert!(tax_search(&conn, "nope").expect("miss").is_empty());
    }

    #[test]
    fn helper_list_reads_game_roles() {
        let conn = open(&std::path::PathBuf::from(":memory:")).expect("open");
        assert!(helper_list(&conn, "game_roles").expect("empty").is_empty());
        conn.execute(
            "INSERT INTO helpers (table_name, id, name, id_name, is_judge_side) VALUES
             ('game_roles', 1, 'Commando', 'Kommando', 0),
             ('game_roles', 4, 'Referee', 'Wasit', 1)",
            [],
        )
        .expect("roles");
        assert_eq!(
            helper_list(&conn, "game_roles").expect("roles"),
            vec![
                (1, "Commando".to_string(), "Kommando".to_string(), false),
                (4, "Referee".to_string(), "Wasit".to_string(), true),
            ]
        );
    }
}
