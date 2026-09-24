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
        -- `image_url` USED to live here and has been dropped from the
        -- synced columns: a picture belongs to a HULL, and Minos
        -- retired the class picture. The column is left in place
        -- rather than dropped so an existing database file still
        -- opens, but nothing writes it and nothing reads it — it is
        -- not a candidate source of visual truth.
        CREATE TABLE IF NOT EXISTS unit_classes (
            id INTEGER PRIMARY KEY, name TEXT NOT NULL,
            id_name TEXT NOT NULL DEFAULT '', type_id INTEGER,
            turn_rate REAL, hull_count INTEGER NOT NULL DEFAULT 0
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
        -- The map layer's own table, NOT a mirror of anything: which
        -- symbol a unit type draws as. Keyed on the mirrored
        -- unit_types.id, so a renamed type keeps its shape and a
        -- removed type simply stops drawing as itself. Holds no names.
        CREATE TABLE IF NOT EXISTS unit_type_symbols (
            type_id INTEGER PRIMARY KEY, symbol INTEGER NOT NULL
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

/// Mapped-category count per service branch (Others fence): the
/// declared ownership from branch_categories, keyed by branch id.
/// A zero means unmapped — the CMS owns the mapping, and the client
/// labels the branch instead of treating it as a fault or a bucket.
pub fn branch_mapped_counts(conn: &Connection) -> std::collections::HashMap<i64, i64> {
    let mut out = std::collections::HashMap::new();
    let Ok(mut stmt) = conn.prepare(
        "SELECT branch_id, COUNT(*) FROM branch_categories GROUP BY branch_id",
    ) else {
        return out;
    };
    if let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))) {
        for row in rows.flatten() {
            out.insert(row.0, row.1);
        }
    }
    out
}

/// Service-branch display name from the helpers mirror (English +
/// Indonesian as the drill shows them), for branch labels.
pub fn branch_label(conn: &Connection, branch_id: i64) -> Option<(String, String)> {
    conn.query_row(
        "SELECT name, id_name FROM helpers WHERE table_name = 'service_branches' AND id = ?1",
        [branch_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .unwrap_or(None)
}
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
                    u.class_id, COALESCE(c.name, ''), u.branch_id,
                    c.type_id, t.category_id, u.domain_id
             FROM units u LEFT JOIN unit_classes c ON c.id = u.class_id
             LEFT JOIN unit_types t ON t.id = c.type_id
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
                branch_id: r.get::<_, Option<i64>>(5).unwrap_or(None),
                type_id: r.get::<_, Option<i64>>(6).unwrap_or(None),
                category_id: r.get::<_, Option<i64>>(7).unwrap_or(None),
                domain_id: r.get::<_, Option<i64>>(8).unwrap_or(None),
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
    /// Owning service branch, if the hull declares one. Drives the
    /// unmapped-branch fence — matched by mapping, never by branch
    /// NAME (helper rows are operator vocabulary).
    pub branch_id: Option<i64>,
    /// Unit type, the level the map symbol is keyed on. `None` when
    /// the hull's class declares no type.
    pub type_id: Option<i64>,
    /// Unit category, the fallback the map symbol falls back to.
    /// `None` when the chain is broken anywhere above the hull.
    pub category_id: Option<i64>,
    /// Movement domain (air / surface / subsurface / land), a
    /// secondary signal for the symbol only.
    pub domain_id: Option<i64>,
}

impl StoreUnit {
    /// Stable taxonomy projection used by symbol resolution. Keeping
    /// it here means unit readers cannot disagree about which ids a
    /// hull contributes.
    pub fn taxonomy(&self) -> UnitTaxonomy {
        UnitTaxonomy {
            type_id: self.type_id,
            category_id: self.category_id,
            domain_id: self.domain_id,
        }
    }
}

/// How a unit is drawn when its image is not on the map — a symbol,
/// not a photograph.
///
/// Ordered coarse-to-fine within each family so the renderer can
/// treat "is this a ship" as a range check. The discriminants are
/// stable integers: they are STORED (in `unit_type_symbols`), so
/// reordering the variants must not renumber them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum MapSymbol {
    /// A hull of a type the operator has not assigned a symbol.
    UnknownShip = 0,
    /// Surface combatant, size unspecified.
    Destroyer = 1,
    Frigate = 2,
    Corvette = 3,
    /// Support ship: tanker, supply, repair.
    Auxiliary = 4,
    /// Anything that carries people ashore: LST, landing ship.
    Landing = 5,
    Submarine = 6,
    /// Aircraft.
    Plane = 7,
    /// Armour and other land units.
    GroundUnit = 8,
    /// A harbour or base rather than a vehicle.
    Port = 9,
}

impl MapSymbol {
    /// Every symbol, for a picker or a test sweep.
    pub const ALL: [MapSymbol; 10] = [
        MapSymbol::UnknownShip,
        MapSymbol::Destroyer,
        MapSymbol::Frigate,
        MapSymbol::Corvette,
        MapSymbol::Auxiliary,
        MapSymbol::Landing,
        MapSymbol::Submarine,
        MapSymbol::Plane,
        MapSymbol::GroundUnit,
        MapSymbol::Port,
    ];

    /// From the stored discriminant. An unknown or unassigned value
    /// is the generic ship, never a panic and never a guess.
    pub fn from_ordinal(v: i32) -> MapSymbol {
        MapSymbol::ALL
            .iter()
            .copied()
            .find(|s| *s as i32 == v)
            .unwrap_or(MapSymbol::UnknownShip)
    }

    /// True for the water-going hulls, so a renderer can ask "is this
    /// a ship" without listing them.
    pub fn is_ship(self) -> bool {
        matches!(
            self,
            MapSymbol::UnknownShip
                | MapSymbol::Destroyer
                | MapSymbol::Frigate
                | MapSymbol::Corvette
                | MapSymbol::Auxiliary
                | MapSymbol::Landing
                | MapSymbol::Submarine
        )
    }
}

/// The taxonomy ids a symbol decision reads, plus the assignment
/// table, all as data.
#[derive(Debug, Clone, Default)]
pub struct SymbolResolver {
    /// `unit_type.id -> symbol`, the finest signal.
    pub by_type: std::collections::HashMap<i64, MapSymbol>,
    /// `unit_categories.id -> symbol`, the fallback when a type has
    /// no assignment.
    pub by_category: std::collections::HashMap<i64, MapSymbol>,
    /// `movement_domain.id -> symbol`, consulted only when both of
    /// the above are silent — a hint, never a decision on its own.
    pub by_domain: std::collections::HashMap<i64, MapSymbol>,
}

impl SymbolResolver {
    /// Build a resolver from mirrored assignments.
    pub fn new(
        by_type: Vec<(i64, MapSymbol)>,
        by_category: Vec<(i64, MapSymbol)>,
        by_domain: Vec<(i64, MapSymbol)>,
    ) -> Self {
        SymbolResolver {
            by_type: by_type.into_iter().collect(),
            by_category: by_category.into_iter().collect(),
            by_domain: by_domain.into_iter().collect(),
        }
    }

    /// Resolve a unit's symbol from its taxonomy, most specific first:
    /// type, then category, then movement domain, then the generic
    /// fallback. Every branch terminates in a symbol, so a unit the
    /// operator has never classified still draws.
    pub fn resolve(&self, tax: UnitTaxonomy) -> MapSymbol {
        if let Some(id) = tax.type_id {
            if let Some(s) = self.by_type.get(&id) {
                return *s;
            }
        }
        if let Some(id) = tax.category_id {
            if let Some(s) = self.by_category.get(&id) {
                return *s;
            }
        }
        if let Some(id) = tax.domain_id {
            if let Some(s) = self.by_domain.get(&id) {
                return *s;
            }
        }
        // No type, category, or domain assignment can identify the
        // family. The generic ship remains the honest last resort;
        // ground/port/plane are selected by the same id maps above,
        // never guessed from a display name here.
        MapSymbol::UnknownShip
    }
}

/// A unit's place in Minos' taxonomy, by STABLE ID.
///
/// Everything here is an id from the synced mirror, never a display
/// name: the operator may rename any lookup row, and a symbol keyed on
/// a name would change shape the moment somebody corrected a spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UnitTaxonomy {
    pub type_id: Option<i64>,
    pub category_id: Option<i64>,
    pub domain_id: Option<i64>,
}

/// `unit_type.id -> MapSymbol` assignments, as read from the local
/// mirror. Populated from the taxonomy sync, not hardcoded in source.
pub fn unit_type_symbols(conn: &Connection) -> Result<Vec<(i64, MapSymbol)>, String> {
    let mut raw = conn
        .prepare("SELECT type_id, symbol FROM unit_type_symbols ORDER BY type_id")
        .map_err(|e| e.to_string())?;
    let rows = raw
        .query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i32>(1)?))
        })
        .map_err(|e| e.to_string())?;
    let out: Vec<(i64, MapSymbol)> = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|(id, ord)| (id, MapSymbol::from_ordinal(ord)))
        .collect();
    Ok(out)
}



/// Set or clear one type's explicit far-map symbol. `None` removes
/// the override and lets category/domain resolution speak again. One
/// row is changed in place; unrelated operator assignments survive.
pub fn set_unit_type_symbol(
    conn: &mut Connection,
    type_id: i64,
    symbol: Option<MapSymbol>,
) -> Result<(), String> {
    match symbol {
        Some(symbol) => conn.execute(
            "INSERT INTO unit_type_symbols (type_id, symbol) VALUES (?1, ?2) \
             ON CONFLICT(type_id) DO UPDATE SET symbol = excluded.symbol",
            params![type_id, symbol as i32],
        ),
        None => conn.execute("DELETE FROM unit_type_symbols WHERE type_id = ?1", [type_id]),
    }
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// Type names for the assignment editor. They are labels for a stable
/// id, never inputs to symbol resolution.
pub fn unit_type_names(conn: &Connection) -> Result<Vec<(i64, String)>, String> {
    let mut raw = conn
        .prepare("SELECT id, name FROM unit_types ORDER BY name")
        .map_err(|e| e.to_string())?;
    let rows = raw
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Category fallbacks derived from stable ids: a category resolves
/// only when every assigned type in it resolves to the same symbol. A
/// mixed category stays silent and resolution continues to the domain
/// rather than inventing a majority winner.
pub fn unit_category_symbols(conn: &Connection) -> Result<Vec<(i64, MapSymbol)>, String> {
    let mut raw = conn
        .prepare(
            "SELECT t.category_id, MIN(s.symbol) \
             FROM unit_type_symbols s \
             JOIN unit_types t ON t.id = s.type_id \
             WHERE t.category_id IS NOT NULL \
             GROUP BY t.category_id \
             HAVING COUNT(DISTINCT s.symbol) = 1 \
             ORDER BY t.category_id",
        )
        .map_err(|e| e.to_string())?;
    let rows = raw
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i32>(1)?)))
        .map_err(|e| e.to_string())?;
    Ok(rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|(id, ord)| (id, MapSymbol::from_ordinal(ord)))
        .collect())
}

/// Movement-domain fallbacks derived through the same stable-id
/// joins. The domain is a secondary hint: it is used only for a domain
/// whose assigned population agrees unanimously.
pub fn unit_movement_domain_symbols(
    conn: &Connection,
) -> Result<Vec<(i64, MapSymbol)>, String> {
    let mut raw = conn
        .prepare(
            "SELECT u.domain_id, MIN(s.symbol) \
             FROM unit_type_symbols s \
             JOIN unit_classes c ON c.type_id = s.type_id \
             JOIN units u ON u.class_id = c.id \
             WHERE u.domain_id IS NOT NULL \
             GROUP BY u.domain_id \
             HAVING COUNT(DISTINCT s.symbol) = 1 \
             ORDER BY u.domain_id",
        )
        .map_err(|e| e.to_string())?;
    let rows = raw
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i32>(1)?)))
        .map_err(|e| e.to_string())?;
    Ok(rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|(id, ord)| (id, MapSymbol::from_ordinal(ord)))
        .collect())
}

/// Replace the whole `unit_type.id -> symbol` assignment table.
///
/// The table is read whole on every symbol lookup, so a half-applied
/// table would draw some hulls with yesterday's symbol; the delete
/// and the inserts therefore belong in one transaction.
pub fn replace_unit_type_symbols(
    conn: &mut Connection,
    assignments: &[(i64, MapSymbol)],
) -> Result<usize, String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM unit_type_symbols", [])
        .map_err(|e| e.to_string())?;
    let mut n = 0;
    for (type_id, symbol) in assignments {
        tx.execute(
            "INSERT INTO unit_type_symbols (type_id, symbol) VALUES (?1, ?2)",
            rusqlite::params![type_id, *symbol as i32],
        )
        .map_err(|e| e.to_string())?;
        n += 1;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(n)
}

/// Picker rows: register hulls with their class names, in name order.
pub fn fleet_units(conn: &Connection) -> Result<Vec<StoreUnit>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT u.id, u.name, COALESCE(u.hull_number, ''),
                    u.class_id, COALESCE(c.name, ''), u.branch_id,
                    c.type_id, t.category_id, u.domain_id
             FROM units u LEFT JOIN unit_classes c ON c.id = u.class_id
             LEFT JOIN unit_types t ON t.id = c.type_id
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
                branch_id: r.get::<_, Option<i64>>(5).unwrap_or(None),
                type_id: r.get::<_, Option<i64>>(6).unwrap_or(None),
                category_id: r.get::<_, Option<i64>>(7).unwrap_or(None),
                domain_id: r.get::<_, Option<i64>>(8).unwrap_or(None),
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
                    u.class_id, COALESCE(c.name, ''), u.branch_id,
                    c.type_id, t.category_id, u.domain_id
             FROM units u LEFT JOIN unit_classes c ON c.id = u.class_id
             LEFT JOIN unit_types t ON t.id = c.type_id
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
            branch_id: r.get::<_, Option<i64>>(5).unwrap_or(None),
                type_id: r.get::<_, Option<i64>>(6).unwrap_or(None),
                category_id: r.get::<_, Option<i64>>(7).unwrap_or(None),
                domain_id: r.get::<_, Option<i64>>(8).unwrap_or(None),
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
    /// Physical measurements, mirrored from the same spec body the
    /// sim figures come from. `None` means the backend published
    /// none — the map layer falls back rather than inventing a size.
    pub loa_m: Option<f64>,
    pub beam_m: Option<f64>,
    pub draft_m: Option<f64>,
    pub displacement_standard_t: Option<f64>,
    pub displacement_full_t: Option<f64>,
    pub turn_rate_max_deg_s: Option<f64>,
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
                loa_m: num("loa_m"),
                beam_m: num("beam_m"),
                draft_m: num("draft_m"),
                displacement_standard_t: num("displacement_standard_t"),
                displacement_full_t: num("displacement_full_t"),
                turn_rate_max_deg_s: num("turn_rate_max_deg_s"),
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

    /// The map's figures must survive the mirror round-trip, because
    /// the render layer reads them from a restored session rather
    /// than re-fetching every hull. Absent measurements stay absent.
    #[test]
    fn specs_mirror_physical_measurements() {
        let conn = open(&std::path::PathBuf::from(":memory:")).expect("open");
        store_spec(
            &conn,
            13,
            1,
            true,
            r#"{"class_name":"Sigma","speed_kn":30.0,"loa_m":120.5,"beam_m":16.2,
                "draft_m":4.1,"displacement_standard_t":3900.0,
                "displacement_full_t":5200.0,"turn_rate_max_deg_s":2.5}"#,
        )
        .expect("store sized");
        store_spec(&conn, 14, 1, true, r#"{"class_name":"Bare","speed_kn":9.0}"#)
            .expect("store unsized");

        let figs = current_figures(&conn).expect("figures");
        let sized = figs.iter().find(|f| f.unit_id == 13).expect("13");
        assert_eq!(sized.loa_m, Some(120.5));
        assert_eq!(sized.beam_m, Some(16.2));
        assert_eq!(sized.draft_m, Some(4.1));
        assert_eq!(sized.displacement_standard_t, Some(3900.0));
        assert_eq!(sized.displacement_full_t, Some(5200.0));
        assert_eq!(sized.turn_rate_max_deg_s, Some(2.5));
        // The sim figures are unaffected by the map's.
        assert_eq!(sized.speed_kn, Some(30.0));

        // A hull whose published spec omits every measurement reads
        // None, not 0.0 — a zero would draw a zero-metre ship.
        let bare = figs.iter().find(|f| f.unit_id == 14).expect("14");
        assert_eq!(bare.loa_m, None);
        assert_eq!(bare.beam_m, None);
        assert_eq!(bare.speed_kn, Some(9.0));
    }

    /// Taxonomy ids reach the map layer. A hull joined to its class,
    /// type and category must carry all three, because the far-zoom
    /// symbol is keyed on exactly these and never on a name.
    #[test]
    fn unit_rows_carry_taxonomy_ids() {
        let mut conn = open(&std::path::PathBuf::from(":memory:")).expect("open");
        conn.execute(
            "INSERT INTO unit_categories (id, name, id_name, type_count) VALUES (7, 'Ship', 'Kapal', 2)",
            [],
        )
        .expect("category");
        conn.execute(
            "INSERT INTO unit_types (id, name, id_name, category_id, class_count) VALUES (3, 'Corvette', 'Korvet', 7, 1)",
            [],
        )
        .expect("type");
        conn.execute(
            "INSERT INTO unit_classes (id, name, id_name, type_id, hull_count) VALUES (5, 'Sigma', 'Sigma', 3, 1)",
            [],
        )
        .expect("class");
        conn.execute(
            "INSERT INTO units (id, name, hull_number, class_id, branch_id, domain_id) VALUES (13, 'KRI Selat', '521', 5, 1, 4)",
            [],
        )
        .expect("hull");

        let row = fleet_unit(&conn, "13").expect("query").expect("found");
        assert_eq!(row.class_id, 5);
        assert_eq!(row.type_id, Some(3), "class declares the type");
        assert_eq!(row.category_id, Some(7), "type declares the category");
        assert_eq!(row.domain_id, Some(4), "movement domain rides along");
        assert_eq!(row.branch_id, Some(1));

        // A hull whose class declares no type still reads, with the
        // upper levels absent rather than zero — zero is a real id.
        conn.execute(
            "INSERT INTO unit_classes (id, name, id_name, type_id, hull_count) VALUES (6, 'Orphan', 'Orphan', NULL, 1)",
            [],
        )
        .expect("typeless class");
        conn.execute(
            "INSERT INTO units (id, name, hull_number, class_id) VALUES (99, 'Typeless', NULL, 6)",
            [],
        )
        .expect("typeless hull");
        let bare = fleet_unit(&conn, "99").expect("query").expect("found");
        assert_eq!(bare.type_id, None);
        assert_eq!(bare.category_id, None);
        assert_eq!(bare.domain_id, None);
    }

    /// The symbol table round-trips through SQLite as discriminants
    /// and comes back as symbols, including a stored ordinal the enum
    /// no longer knows.
    #[test]
    fn type_symbol_table_round_trips() {
        let mut conn = open(&std::path::PathBuf::from(":memory:")).expect("open");
        let written = replace_unit_type_symbols(
            &mut conn,
            &[(3, MapSymbol::Corvette), (4, MapSymbol::Auxiliary)],
        )
        .expect("write");
        assert_eq!(written, 2);
        let back = unit_type_symbols(&conn).expect("read");
        assert_eq!(
            back,
            vec![(3, MapSymbol::Corvette), (4, MapSymbol::Auxiliary)]
        );
        // A stale ordinal from a future or corrupted row is the
        // generic ship, never a panic.
        conn.execute(
            "INSERT INTO unit_type_symbols (type_id, symbol) VALUES (9, 4242)",
            [],
        )
        .expect("stale ordinal");
        let all = unit_type_symbols(&conn).expect("read");
        assert_eq!(
            all.iter().find(|(id, _)| *id == 9).map(|(_, s)| *s),
            Some(MapSymbol::UnknownShip)
        );
        // Re-writing replaces wholesale, never merges.
        replace_unit_type_symbols(&mut conn, &[(3, MapSymbol::Frigate)]).expect("rewrite");
        assert_eq!(
            unit_type_symbols(&conn).expect("read"),
            vec![(3, MapSymbol::Frigate)]
        );
    }

    /// The assignment editor changes one stable type id in place and
    /// can clear it back to category/domain resolution. Other rows
    /// are never disturbed.
    #[test]
    fn one_type_assignment_can_change_or_clear() {
        let mut conn = open(&std::path::PathBuf::from(":memory:")).expect("open");
        conn.execute(
            "INSERT INTO unit_types (id, name) VALUES (3, 'Corvette'), (4, 'Auxiliary')",
            [],
        )
        .expect("types");
        replace_unit_type_symbols(
            &mut conn,
            &[(3, MapSymbol::Corvette), (4, MapSymbol::Auxiliary)],
        )
        .expect("initial");

        set_unit_type_symbol(&mut conn, 3, Some(MapSymbol::Landing)).expect("change");
        assert_eq!(
            unit_type_symbols(&conn).expect("read"),
            vec![(3, MapSymbol::Landing), (4, MapSymbol::Auxiliary)]
        );
        set_unit_type_symbol(&mut conn, 3, None).expect("clear");
        assert_eq!(
            unit_type_symbols(&conn).expect("read"),
            vec![(4, MapSymbol::Auxiliary)]
        );
    }

    /// Category and movement-domain fallbacks are derived from the
    /// type assignment table by stable-id joins. They appear only
    /// where the assigned population agrees unanimously; a mixed
    /// parent never wins by majority.
    #[test]
    fn category_and_domain_fallbacks_derive_from_type_assignments() {
        let mut conn = open(&std::path::PathBuf::from(":memory:")).expect("open");
        conn.execute_batch(
            "INSERT INTO unit_categories (id, name) VALUES (10, 'Ships'), (11, 'Mixed');
             INSERT INTO unit_types (id, name, category_id) VALUES
               (20, 'FF', 10), (21, 'FFG', 10), (22, 'FS', 11), (23, 'AH', 11);
             INSERT INTO unit_classes (id, name, type_id) VALUES
               (30, 'A', 20), (31, 'B', 21), (32, 'C', 22), (33, 'D', 23);
             INSERT INTO units (id, name, class_id, domain_id) VALUES
               (40, 'a', 30, 1), (41, 'b', 31, 1),
               (42, 'c', 32, 2), (43, 'd', 33, 2);",
        )
        .expect("taxonomy");
        replace_unit_type_symbols(
            &mut conn,
            &[
                (20, MapSymbol::Frigate),
                (21, MapSymbol::Frigate),
                (22, MapSymbol::Corvette),
                (23, MapSymbol::Auxiliary),
            ],
        )
        .expect("assign");

        assert_eq!(unit_category_symbols(&conn).expect("categories"), vec![(10, MapSymbol::Frigate)]);
        assert_eq!(
            unit_movement_domain_symbols(&conn).expect("domains"),
            vec![(1, MapSymbol::Frigate)]
        );

        let resolver = SymbolResolver::new(
            unit_type_symbols(&conn).expect("types"),
            unit_category_symbols(&conn).expect("categories"),
            unit_movement_domain_symbols(&conn).expect("domains"),
        );
        assert_eq!(
            resolver.resolve(UnitTaxonomy {
                type_id: Some(99),
                category_id: Some(10),
                domain_id: Some(2),
            }),
            MapSymbol::Frigate,
            "unassigned type falls back to its unanimous category"
        );
        assert_eq!(
            resolver.resolve(UnitTaxonomy {
                type_id: None,
                category_id: Some(11),
                domain_id: Some(2),
            }),
            MapSymbol::UnknownShip,
            "mixed category and mixed domain both stay silent"
        );
    }

    /// Resolution is most-specific-first, and every unknown path
    /// terminates in a drawable symbol.
    #[test]
    fn symbol_resolution_prefers_type_then_category_then_domain() {
        let r = SymbolResolver::new(
            vec![(3, MapSymbol::Corvette)],
            vec![(7, MapSymbol::Destroyer), (8, MapSymbol::Plane)],
            vec![(4, MapSymbol::Submarine)],
        );
        // Type wins over the category it belongs to.
        assert_eq!(
            r.resolve(UnitTaxonomy {
                type_id: Some(3),
                category_id: Some(7),
                domain_id: Some(4)
            }),
            MapSymbol::Corvette
        );
        // An unassigned type falls back to its category.
        assert_eq!(
            r.resolve(UnitTaxonomy {
                type_id: Some(99),
                category_id: Some(7),
                domain_id: None
            }),
            MapSymbol::Destroyer
        );
        // Both silent: the domain is a hint, consulted last.
        assert_eq!(
            r.resolve(UnitTaxonomy {
                type_id: None,
                category_id: Some(99),
                domain_id: Some(4)
            }),
            MapSymbol::Submarine
        );
        // Everything unknown draws the generic ship. No panic, no
        // error, no absence.
        assert_eq!(
            r.resolve(UnitTaxonomy::default()),
            MapSymbol::UnknownShip
        );
    }

    /// Two different known types must be able to draw differently,
    /// which is the whole point of keying on the type.
    #[test]
    fn distinct_types_resolve_to_distinct_symbols() {
        let r = SymbolResolver::new(
            vec![(3, MapSymbol::Corvette), (4, MapSymbol::Auxiliary)],
            vec![(7, MapSymbol::UnknownShip)],
            vec![],
        );
        let corvette = r.resolve(UnitTaxonomy {
            type_id: Some(3),
            category_id: Some(7),
            domain_id: None,
        });
        let auxiliary = r.resolve(UnitTaxonomy {
            type_id: Some(4),
            category_id: Some(7),
            domain_id: None,
        });
        assert_ne!(corvette, auxiliary, "a corvette is not an auxiliary");
        assert_eq!(corvette, MapSymbol::Corvette);
        assert_eq!(auxiliary, MapSymbol::Auxiliary);
        // Same type, same symbol — the mapping is a function.
        let again = r.resolve(UnitTaxonomy {
            type_id: Some(3),
            category_id: Some(7),
            domain_id: None,
        });
        assert_eq!(corvette, again);
    }

    /// An empty resolver — the state before any assignment is
    /// declared — still draws every unit.
    #[test]
    fn empty_resolver_still_draws() {
        let r = SymbolResolver::default();
        for tax in [
            UnitTaxonomy::default(),
            UnitTaxonomy { type_id: Some(1), category_id: None, domain_id: None },
            UnitTaxonomy { type_id: None, category_id: Some(2), domain_id: Some(3) },
        ] {
            assert_eq!(r.resolve(tax), MapSymbol::UnknownShip, "{tax:?}");
        }
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
