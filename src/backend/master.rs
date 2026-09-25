//! Master-data reads from Minos (backend split, step 3).
//!
//! - [`MinosMaster`]: helpers, taxonomy, units, hierarchy, games, users.
//! - [`TableData`]: one mirrored table: target, columns, rows.
//! - [`HullSpec`]: one hull's sim-driving figures, plus [`sync_spec`].
//!
//! Row structs (`BackendUser`, `GameRow`, `GameDetail`, `GamePlacement`,
//! `PlacementList`, `JoinResult`, `GameFix`, `GameHullPos`,
//! `PositionList`, `GameClock`, `GameClockSegment`, `InboxMsg`,
//! `MsgRecipient`, `MsgDraft`, `ScenarioRole`, `Participant`,
//! `GameUnit`) feed
//! the session-users panel. Reads stay bearer-authed and
//! envelope-unwrapped; orchestration lives in [`crate::store::sync_from`].

use super::{BackendError, unwrap_envelope, unwrap_envelope_page};

/// Minos master-data reads (master-data ticket): helpers, taxonomy,
/// units, hierarchy. Bearer-authed, envelope-unwrapped, paged where the
/// contract pages. Rows come out as store cells; orchestration lives in
/// [`crate::store::sync_from`].
pub struct MinosMaster {
    base_url: String,
    client: reqwest::blocking::Client,
}

/// One mirrored table: target, columns, and rows.
pub struct TableData {
    pub table: &'static str,
    pub columns: &'static str,
    pub placeholders: &'static str,
    pub rows: Vec<Vec<crate::store::StoredValue>>,
}

use crate::store::{StoredValue, opt_int, opt_real, opt_text};

impl MinosMaster {
    pub fn new(base_url: &str) -> Result<Self, BackendError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client,
        })
    }

    fn get(&self, token: &str, path: &str) -> Result<serde_json::Value, BackendError> {
        unwrap_envelope(
            self.client
                .get(&format!("{}{}", self.base_url, path))
                .bearer_auth(token)
                .send()?,
        )
    }

    fn str_of(v: &serde_json::Value, key: &str) -> Option<String> {
        v[key].as_str().map(|s| s.to_string())
    }

    fn int_of(v: &serde_json::Value, key: &str) -> Option<i64> {
        v[key].as_i64()
    }

    fn bool_of(v: &serde_json::Value, key: &str) -> bool {
        v[key].as_bool().unwrap_or(false)
    }

    fn helper_row(table: &str, h: &serde_json::Value) -> Vec<StoredValue> {
        vec![
            StoredValue::Text(table.to_string()),
            StoredValue::Int(h["id"].as_i64().unwrap_or(0)),
            StoredValue::Text(h["name"].as_str().unwrap_or("").to_string()),
            StoredValue::Text(h["id_name"].as_str().unwrap_or("").to_string()),
            StoredValue::Text(h["description_en"].as_str().unwrap_or("").to_string()),
            StoredValue::Int(b2i(Self::bool_of(h, "is_system"))),
            StoredValue::Int(b2i(Self::bool_of(h, "is_judge_side"))),
        ]
    }

    /// Every lookup table in one payload, keyed by table name.
    pub fn helpers(&self, token: &str) -> Result<TableData, BackendError> {
        let data = self.get(token, "/helpers")?;
        let helpers = data["helpers"].as_object().cloned().unwrap_or_default();
        let mut rows = Vec::new();
        let mut tables: Vec<String> = helpers.keys().cloned().collect();
        tables.sort();
        for t in tables {
            if let Some(list) = helpers[&t].as_array() {
                for h in list {
                    rows.push(Self::helper_row(&t, h));
                }
            }
        }
        Ok(TableData {
            table: "helpers",
            columns: "table_name, id, name, id_name, description_en, is_system, is_judge_side",
            placeholders: "?1, ?2, ?3, ?4, ?5, ?6, ?7",
            rows,
        })
    }

    /// Echelon vocabulary, lowest first. Order lives in echelon_rank.
    pub fn hierarchy(&self, token: &str) -> Result<TableData, BackendError> {
        let data = self.get(token, "/hierarchy")?;
        let list = data.as_array().cloned().unwrap_or_default();
        let rows = list
            .iter()
            .map(|h| {
                vec![
                    StoredValue::Int(h["id"].as_i64().unwrap_or(0)),
                    StoredValue::Text(h["name"].as_str().unwrap_or("").to_string()),
                    StoredValue::Text(h["id_name"].as_str().unwrap_or("").to_string()),
                    StoredValue::Text(h["description_en"].as_str().unwrap_or("").to_string()),
                    StoredValue::Int(h["echelon_rank"].as_i64().unwrap_or(0)),
                    StoredValue::Int(b2i(Self::bool_of(h, "is_system"))),
                    StoredValue::Text(h["note"].as_str().unwrap_or("").to_string()),
                ]
            })
            .collect();
        Ok(TableData {
            table: "hierarchy_echelons",
            columns: "id, name, id_name, description_en, echelon_rank, is_system, note",
            placeholders: "?1, ?2, ?3, ?4, ?5, ?6, ?7",
            rows,
        })
    }

    /// Categories with their type counts (no paging by design).
    pub fn categories(&self, token: &str) -> Result<TableData, BackendError> {
        let data = self.get(token, "/unit-categories")?;
        let list = data.as_array().cloned().unwrap_or_default();
        let rows = list
            .iter()
            .map(|c| {
                vec![
                    StoredValue::Int(c["id"].as_i64().unwrap_or(0)),
                    StoredValue::Text(c["name"].as_str().unwrap_or("").to_string()),
                    StoredValue::Text(c["id_name"].as_str().unwrap_or("").to_string()),
                    StoredValue::Text(c["description_en"].as_str().unwrap_or("").to_string()),
                    StoredValue::Int(b2i(Self::bool_of(c, "is_system"))),
                    StoredValue::Int(c["type_count"].as_i64().unwrap_or(0)),
                ]
            })
            .collect();
        Ok(TableData {
            table: "unit_categories",
            columns: "id, name, id_name, description_en, is_system, type_count",
            placeholders: "?1, ?2, ?3, ?4, ?5, ?6",
            rows,
        })
    }

    /// Full taxonomy types (paged defensively; small in practice).
    pub fn types(&self, token: &str) -> Result<TableData, BackendError> {
        let mut rows = Vec::new();
        for page in 1..=50 {
            let data = self.get(
                token,
                &format!("/unit-types?page_size=200&page_number={page}"),
            )?;
            let list = data.as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                break;
            }
            for t in &list {
                rows.push(vec![
                    StoredValue::Int(t["id"].as_i64().unwrap_or(0)),
                    StoredValue::Text(t["name"].as_str().unwrap_or("").to_string()),
                    StoredValue::Text(t["id_name"].as_str().unwrap_or("").to_string()),
                    StoredValue::Text(t["description_en"].as_str().unwrap_or("").to_string()),
                    StoredValue::Int(b2i(Self::bool_of(t, "is_system"))),
                    opt_int(Self::int_of(&t["category"], "id")),
                    StoredValue::Int(t["class_count"].as_i64().unwrap_or(0)),
                ]);
            }
            if list.len() < 200 {
                break;
            }
        }
        Ok(TableData {
            table: "unit_types",
            columns: "id, name, id_name, description_en, is_system, category_id, class_count",
            placeholders: "?1, ?2, ?3, ?4, ?5, ?6, ?7",
            rows,
        })
    }

    /// Classes with taxonomy link, turn rate, and hull count. Class
    /// pictures are intentionally not mirrored: Minos owns images per
    /// hull, and a class image can never stand in for a unit visual.
    pub fn classes(&self, token: &str) -> Result<TableData, BackendError> {
        let mut rows = Vec::new();
        for page in 1..=50 {
            let data = self.get(
                token,
                &format!("/unit-classes?page_size=200&page_number={page}"),
            )?;
            let list = data.as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                break;
            }
            for c in &list {
                rows.push(vec![
                    StoredValue::Int(c["id"].as_i64().unwrap_or(0)),
                    StoredValue::Text(c["name"].as_str().unwrap_or("").to_string()),
                    StoredValue::Text(c["id_name"].as_str().unwrap_or("").to_string()),
                    opt_int(Self::int_of(&c["unit_type"], "id")),
                    opt_real(c["default_turn_rate_max_deg_s"].as_f64()),
                    StoredValue::Int(c["hull_count"].as_i64().unwrap_or(0)),
                ]);
            }
            if list.len() < 200 {
                break;
            }
        }
        Ok(TableData {
            table: "unit_classes",
            columns: "id, name, id_name, type_id, turn_rate, hull_count",
            placeholders: "?1, ?2, ?3, ?4, ?5, ?6",
            rows,
        })
    }

    /// Hull register, paged. Nested taxonomy objects flatten to ids;
    /// per-version specifications stay server-side in v1 (one request
    /// per hull would turn every sync into a request storm).
    pub fn units(&self, token: &str) -> Result<TableData, BackendError> {
        let mut rows = Vec::new();
        for page in 1..=50 {
            let data = self.get(token, &format!("/units?page_size=200&page_number={page}"))?;
            let list = data.as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                break;
            }
            for u in &list {
                rows.push(vec![
                    StoredValue::Int(u["id"].as_i64().unwrap_or(0)),
                    StoredValue::Text(u["name"].as_str().unwrap_or("").to_string()),
                    opt_text(Self::str_of(u, "hull_number")),
                    opt_int(Self::int_of(&u["unit_class"], "id")),
                    opt_int(Self::int_of(&u["unit_status"], "id")),
                    opt_int(Self::int_of(&u["service_branch"], "id")),
                    opt_int(Self::int_of(&u["movement_domain"], "id")),
                ]);
            }
            if list.len() < 200 {
                break;
            }
        }
        Ok(TableData {
            table: "units",
            columns: "id, name, hull_number, class_id, status_id, branch_id, domain_id",
            placeholders: "?1, ?2, ?3, ?4, ?5, ?6, ?7",
            rows,
        })
    }

    /// Declared branch → category ownership (setup-overhaul picker):
    /// ids only — the rows themselves come from categories(). Empty is
    /// a real answer (a branch owning nothing renders a zero, not an
    /// error); non-numeric ids are refused upstream with 422.
    pub fn category_ids_for_branch(&self, token: &str, branch_id: i64) -> Result<Vec<i64>, BackendError> {
        let data = self.get(
            token,
            &format!("/unit-categories?id_service_branch={branch_id}"),
        )?;
        Ok(data
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|c| c["id"].as_i64())
            .filter(|id| *id > 0)
            .collect())
    }

    fn post(
        &self,
        token: &str,
        path: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, BackendError> {
        unwrap_envelope(
            self.client
                .post(&format!("{}{}", self.base_url, path))
                .bearer_auth(token)
                .json(&body)
                .send()?,
        )
    }

    /// A committed order is specifically HTTP 201. Other successful
    /// statuses are not interchangeable with a GameFixRecord and become
    /// an Unknown result at the command surface.
    fn post_created(
        &self,
        token: &str,
        path: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, BackendError> {
        let response = self
            .client
            .post(&format!("{}{}", self.base_url, path))
            .bearer_auth(token)
            .json(&body)
            .send()?;
        let status = response.status();
        if !status.is_success() {
            let body: serde_json::Value = response.json().unwrap_or(serde_json::Value::Null);
            return Err(BackendError::http_error(status.as_u16(), &body));
        }
        if status != reqwest::StatusCode::CREATED {
            return Err(BackendError::Other(format!(
                "Minos order response was HTTP {}, expected 201",
                status.as_u16()
            )));
        }
        let body: serde_json::Value = response.json()?;
        Ok(body["data"].clone())
    }

    fn patch(
        &self,
        token: &str,
        path: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, BackendError> {
        unwrap_envelope(
            self.client
                .patch(&format!("{}{}", self.base_url, path))
                .bearer_auth(token)
                .json(&body)
                .send()?,
        )
    }

    fn delete(&self, token: &str, path: &str) -> Result<serde_json::Value, BackendError> {
        unwrap_envelope(
            self.client
                .delete(&format!("{}{}", self.base_url, path))
                .bearer_auth(token)
                .send()?,
        )
    }

    fn put(
        &self,
        token: &str,
        path: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, BackendError> {
        unwrap_envelope(
            self.client
                .put(&format!("{}{}", self.base_url, path))
                .bearer_auth(token)
                .json(&body)
                .send()?,
        )
    }

    /// Bodiless PUT (readiness declare): no `.json()`, so no
    /// `Content-Type` and no empty object for the validator to trip on.
    fn put_empty(&self, token: &str, path: &str) -> Result<serde_json::Value, BackendError> {
        unwrap_envelope(
            self.client
                .put(&format!("{}{}", self.base_url, path))
                .bearer_auth(token)
                .send()?,
        )
    }

    /// Bodiless POST (pause, resume): the path is the whole request.
    fn post_empty(&self, token: &str, path: &str) -> Result<serde_json::Value, BackendError> {
        unwrap_envelope(
            self.client
                .post(&format!("{}{}", self.base_url, path))
                .bearer_auth(token)
                .send()?,
        )
    }

    /// One page walk for small paged endpoints (games, users): 100 rows
    /// a page, stops at the first short page. Permission failures (403)
    /// surface as errors — the UI degrades to the game roster instead.
    fn paged(&self, token: &str, path: &str) -> Result<Vec<serde_json::Value>, BackendError> {
        let mut out = Vec::new();
        let sep = if path.contains('?') { '&' } else { '?' };
        for page in 1..=10 {
            let data = self.get(
                token,
                &format!("{path}{sep}page_size=100&page_number={page}"),
            )?;
            let list = data.as_array().cloned().unwrap_or_default();
            let short = list.len() < 100;
            out.extend(list);
            if short {
                break;
            }
        }
        Ok(out)
    }

    fn enc(s: &str) -> String {
        let mut o = String::new();
        for b in s.bytes() {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                o.push(b as char);
            } else {
                o.push_str(&format!("%{b:02X}"));
            }
        }
        o
    }

    /// Games for the session-users picker (summary shape, newest first).
    pub fn games_list(&self, token: &str) -> Result<Vec<GameRow>, BackendError> {
        Ok(self
            .paged(token, "/games")?
            .iter()
            .filter_map(|g| {
                Some(GameRow {
                    id: g["id"].as_i64()?,
                    name: g["name"].as_str().unwrap_or("").to_string(),
                    state: g["state"].as_str().unwrap_or("").to_string(),
                })
            })
            .collect())
    }

    /// Backend user directory (session-users ticket): live-read, paged,
    /// optional search plus the latest contract's `id_user_status` /
    /// `id_app_role` filters (sent only when set). Needs `read` on
    /// `/system/users` — a 403 here is the roster-fallback signal, not
    /// a bug.
    pub fn users_list(
        &self,
        token: &str,
        search: &str,
        status_id: Option<i64>,
        app_role_id: Option<i64>,
    ) -> Result<Vec<BackendUser>, BackendError> {
        let mut qs: Vec<String> = Vec::new();
        if !search.trim().is_empty() {
            qs.push(format!("search={}", Self::enc(search.trim())));
        }
        if let Some(s) = status_id {
            qs.push(format!("id_user_status={s}"));
        }
        if let Some(r) = app_role_id {
            qs.push(format!("id_app_role={r}"));
        }
        let path = if qs.is_empty() {
            "/users".to_string()
        } else {
            format!("/users?{}", qs.join("&"))
        };
        Ok(self
            .paged(token, &path)?
            .iter()
            .filter_map(|u| {
                Some(BackendUser {
                    id: u["id"].as_i64()?,
                    username: Self::str_of(u, "username").unwrap_or_default(),
                    name: Self::str_of(u, "name").unwrap_or_default(),
                    nrp: Self::str_of(u, "nrp").unwrap_or_default(),
                    pangkat: Self::str_of(u, "pangkat").unwrap_or_default(),
                    satuan: Self::str_of(u, "satuan").unwrap_or_default(),
                    jabatan: Self::str_of(u, "jabatan").unwrap_or_default(),
                    status_id: u["status"]["id"].as_i64(),
                    status_name: u["status"]["name"].as_str().unwrap_or("").to_string(),
                })
            })
            .collect())
    }

    /// One roster row, parsed for the read and for every write that
    /// answers with the whole roster (latest contract: seat / role
    /// change / remove all return it — no read-after-write).
    fn parse_roster(v: &serde_json::Value) -> Vec<Participant> {
        v.as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|p| {
                Some(Participant {
                    user_id: p["id_user"].as_i64()?,
                    user_name: p["user_name"].as_str().unwrap_or("").to_string(),
                    role_id: p["id_game_role"].as_i64().unwrap_or(0),
                    role_name: p["role_name"].as_str().unwrap_or("").to_string(),
                    judge: p["is_judge_side"].as_bool().unwrap_or(false),
                    ready: p["is_ready"].as_bool().unwrap_or(false),
                })
            })
            .collect()
    }

    /// One game piece row, parsed for the read and for the commander
    /// write (whose response is the refreshed units array).
    fn parse_units(v: &serde_json::Value) -> Vec<GameUnit> {
        v.as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|u| {
                Some(GameUnit {
                    unit_id: u["id_unit"].as_i64()?,
                    unit_name: u["unit_name"].as_str().unwrap_or("").to_string(),
                    hull_number: u["hull_number"].as_str().unwrap_or("").to_string(),
                    commander_id: u["id_commander"].as_i64(),
                    commander_name: u["commander_name"].as_str().unwrap_or("").to_string(),
                    hierarchy_node: u["id_hierarchy_node"].as_i64(),
                })
            })
            .collect()
    }

    /// One game's roster (staff-facing): exercise side first by server order.
    pub fn game_participants(&self, token: &str, game_id: i64) -> Result<Vec<Participant>, BackendError> {
        let data = self.get(token, &format!("/games/{game_id}/participants"))?;
        Ok(Self::parse_roster(&data))
    }

    /// Seat somebody in a game (planning|preparation only, one role per
    /// person). A 409 names the world refusing (already seated, closed
    /// roster) — loud by contract. The response is the whole roster.
    pub fn add_participant(
        &self,
        token: &str,
        game_id: i64,
        user_id: i64,
        role_id: i64,
    ) -> Result<Vec<Participant>, BackendError> {
        let data = self.post(
            token,
            &format!("/games/{game_id}/participants"),
            serde_json::json!({ "id_user": user_id, "id_game_role": role_id }),
        )?;
        Ok(Self::parse_roster(&data))
    }

    /// Move somebody to another game role (planning|preparation). The
    /// change **clears that seat's readiness**; response is the whole
    /// roster. Same 409 as seating when already held.
    pub fn set_participant_role(
        &self,
        token: &str,
        game_id: i64,
        user_id: i64,
        role_id: i64,
    ) -> Result<Vec<Participant>, BackendError> {
        let data = self.patch(
            token,
            &format!("/games/{game_id}/participants/{user_id}"),
            serde_json::json!({ "id_game_role": role_id }),
        )?;
        Ok(Self::parse_roster(&data))
    }

    /// Take somebody off the roster (hard delete, idempotent — an absent
    /// id is not an error). Response is the whole roster.
    pub fn remove_participant(
        &self,
        token: &str,
        game_id: i64,
        user_id: i64,
    ) -> Result<Vec<Participant>, BackendError> {
        let data = self.delete(token, &format!("/games/{game_id}/participants/{user_id}"))?;
        Ok(Self::parse_roster(&data))
    }

    /// A game's order of battle with current commanders.
    pub fn game_units_list(&self, token: &str, game_id: i64) -> Result<Vec<GameUnit>, BackendError> {
        let data = self.get(token, &format!("/games/{game_id}/units"))?;
        Ok(Self::parse_units(&data))
    }

    /// Hand a game piece to a commander (planning|preparation). The new
    /// commander must be a participant and never judge-side — refused
    /// loudly otherwise. The response is the refreshed units array.
    pub fn set_unit_commander(
        &self,
        token: &str,
        game_id: i64,
        unit_id: i64,
        commander_id: i64,
    ) -> Result<Vec<GameUnit>, BackendError> {
        let data = self.patch(
            token,
            &format!("/games/{game_id}/units/{unit_id}"),
            serde_json::json!({ "id_commander": commander_id }),
        )?;
        Ok(Self::parse_units(&data))
    }

    /// Create a game session (the concept's "Create Game"): born in
    /// `planning`, nothing else exists yet — participants, units and
    /// hierarchy attach afterwards, while planning lasts. Only `name`
    /// is required; blank optionals are omitted, never sent empty.
    pub fn create_game(
        &self,
        token: &str,
        name: &str,
        description: &str,
        purpose: &str,
        target: &str,
        area: &str,
        map_tag: &str,
    ) -> Result<GameRow, BackendError> {
        let mut body = serde_json::json!({ "name": name, "mode": "maneuver" });
        for (k, v) in [
            ("description", description),
            ("purpose", purpose),
            ("target", target),
            ("area", area),
            ("map_tag", map_tag),
        ] {
            if !v.trim().is_empty() {
                body[k] = serde_json::Value::String(v.to_string());
            }
        }
        let data = self.post(token, "/games", body)?;
        Ok(GameRow {
            id: data["id"].as_i64().unwrap_or(0),
            name: data["name"].as_str().unwrap_or("").to_string(),
            state: data["state"].as_str().unwrap_or("planning").to_string(),
        })
    }

    /// Put a hull into a game as a commanded piece
    /// (planning|preparation). `id_commander` is required — a piece IS a
    /// hull commanded by a participant of this game, never judge-side —
    /// so players seat before fleet assigns. 409 when the hull is
    /// already in this game. Response is the refreshed units array.
    pub fn assign_unit(
        &self,
        token: &str,
        game_id: i64,
        unit_id: i64,
        commander_id: i64,
    ) -> Result<Vec<GameUnit>, BackendError> {
        let data = self.post(
            token,
            &format!("/games/{game_id}/units"),
            serde_json::json!({ "id_unit": unit_id, "id_commander": commander_id }),
        )?;
        Ok(Self::parse_units(&data))
    }

    /// Take a hull out of a game (hard delete, idempotent — an absent
    /// hull is not an error). Response is the refreshed units array.
    pub fn remove_unit(
        &self,
        token: &str,
        game_id: i64,
        unit_id: i64,
    ) -> Result<Vec<GameUnit>, BackendError> {
        let data = self.delete(token, &format!("/games/{game_id}/units/{unit_id}"))?;
        Ok(Self::parse_units(&data))
    }

    /// Move a game one step forward (planning→preparation→execution→
    /// closure). Game-Master authority, forward-only; the gate refusal
    /// names every missing ingredient at once — loud by contract.
    pub fn transition_game(
        &self,
        token: &str,
        game_id: i64,
        to: &str,
    ) -> Result<GameRow, BackendError> {
        let data = self.post(
            token,
            &format!("/games/{game_id}/transitions"),
            serde_json::json!({ "to": to }),
        )?;
        Ok(GameRow {
            id: data["id"].as_i64().unwrap_or(game_id),
            name: data["name"].as_str().unwrap_or("").to_string(),
            state: data["state"].as_str().unwrap_or(to).to_string(),
        })
    }

    /// One game's authoritative state (H1): `GET /games/{id}` unwraps
    /// to the detail projection — id, name, mode, state. The held game's
    /// local stage derives from this, never from a local flag alone;
    /// Minos is forward-only, so this read is the only way to learn a
    /// transition another client made.
    pub fn game_detail(
        &self,
        token: &str,
        game_id: i64,
    ) -> Result<GameDetail, BackendError> {
        let data = self.get(token, &format!("/games/{game_id}"))?;
        Ok(Self::parse_game_detail(&data, game_id))
    }

    /// One game detail read, shared by the hold projection and the
    /// update answer (the mutation re-reads and returns the same
    /// shape the detail endpoint serves).
    fn parse_game_detail(v: &serde_json::Value, fallback_id: i64) -> GameDetail {
        GameDetail {
            id: v["id"].as_i64().unwrap_or(fallback_id),
            name: v["name"].as_str().unwrap_or("").to_string(),
            mode: v["mode"].as_str().unwrap_or("").to_string(),
            state: v["state"].as_str().unwrap_or("").to_string(),
            actual_start: v["actual_start"].as_str().map(|s| s.to_string()),
            assumed_start: v["assumed_start"].as_str().map(|s| s.to_string()),
            time_factor: v["time_factor"].as_f64().unwrap_or(0.0),
            room_key: v["room_key"].as_str().map(|s| s.to_string()),
        }
    }

    /// Partial game edit (admin ticket): the six text fields the
    /// create form owns — absent leaves alone, empty area/map_tag
    /// clears (nullable columns). Admin-granted, planning-only; the
    /// answer is the re-read detail, applied like any bundle detail.
    /// Time fields stay server-side in this slice.
    pub fn update_game(
        &self,
        token: &str,
        game_id: i64,
        upd: &GameUpdate,
    ) -> Result<GameDetail, BackendError> {
        let mut body = serde_json::Map::new();
        let mut text = |key: &str, v: &Option<String>| {
            if let Some(s) = v {
                body.insert(key.to_string(), serde_json::Value::String(s.clone()));
            }
        };
        text("name", &upd.name);
        text("description", &upd.description);
        text("purpose", &upd.purpose);
        text("target", &upd.target);
        text("area", &upd.area);
        text("map_tag", &upd.map_tag);
        let data = self.patch(
            token,
            &format!("/games/{game_id}"),
            serde_json::Value::Object(body),
        )?;
        Ok(Self::parse_game_detail(&data, game_id))
    }

    /// Discard a game (admin ticket): planning-only, soft server-side
    /// with game-scoped roles revoked. The caller drops the hold —
    /// a deleted game reads exactly like a vanished one.
    pub fn delete_game(&self, token: &str, game_id: i64) -> Result<(), BackendError> {
        self.delete(token, &format!("/games/{game_id}"))?;
        Ok(())
    }

    /// The setup view of an exercise's map (C2): where the force has
    /// been put, plus the gate's own placement arithmetic. `unplaced`
    /// and `ready` come from the server — a client recomputing them
    /// from the two lists would be one off-by-one away from promising
    /// an advance the server will refuse.
    pub fn placements_list(
        &self,
        token: &str,
        game_id: i64,
    ) -> Result<PlacementList, BackendError> {
        let data = self.get(token, &format!("/games/{game_id}/placements"))?;
        Ok(Self::parse_placements(&data))
    }

    /// Put a hull at its starting coordinates (C2: planning|preparation
    /// only — after play begins the first fix leg is frozen and this
    /// refuses). Answers with the whole setup view, so the map screen
    /// learns immediately how much of the force is still to come.
    pub fn set_placement(
        &self,
        token: &str,
        game_id: i64,
        unit_id: i64,
        latitude: f64,
        longitude: f64,
    ) -> Result<PlacementList, BackendError> {
        let data = self.put(
            token,
            &format!("/games/{game_id}/units/{unit_id}/placement"),
            serde_json::json!({ "latitude": latitude, "longitude": longitude }),
        )?;
        Ok(Self::parse_placements(&data))
    }

    /// Take a hull off the map (idempotent — unplaced stays unplaced).
    /// The piece stays in the exercise. Answers with the setup view.
    pub fn clear_placement(
        &self,
        token: &str,
        game_id: i64,
        unit_id: i64,
    ) -> Result<PlacementList, BackendError> {
        let data = self.delete(token, &format!("/games/{game_id}/units/{unit_id}/placement"))?;
        Ok(Self::parse_placements(&data))
    }

    /// Enter a game room with its key (C2). The key is the only input —
    /// the client does not know the game id yet. Answers with the game
    /// plus the caller's own participant row, which also confirms the
    /// caller's user id for readiness matching. Unknown key is 404, a
    /// valid key without a seat is 403 (ask the Game Master), both loud.
    pub fn join_game(
        &self,
        token: &str,
        room_key: &str,
    ) -> Result<JoinResult, BackendError> {
        let data = self.post(
            token,
            "/games/join",
            serde_json::json!({ "room_key": room_key }),
        )?;
        Self::parse_join(&data)
    }

    /// Declare (PUT) or withdraw (DELETE) the CALLER's own readiness
    /// (C2). Nobody can declare for somebody else — there is no user id
    /// in the request. Judge-side callers are refused loudly (409):
    /// the gate exempts them, so accepting would lie. Answers with the
    /// game plus the caller's row.
    pub fn set_readiness(
        &self,
        token: &str,
        game_id: i64,
        ready: bool,
    ) -> Result<JoinResult, BackendError> {
        let data = if ready {
            self.put_empty(token, &format!("/games/{game_id}/readiness"))?
        } else {
            self.delete(token, &format!("/games/{game_id}/readiness"))?
        };
        Self::parse_join(&data)
    }

    /// Give a heading and speed to a hull the caller commands (C3).
    /// Heading 0 is due north, not missing; speed 0 is stop, not
    /// absence. The server owns the fix chain, assumed time, clamping,
    /// and history — the answer is the fix the order closed at, so the
    /// client learns where the hull had reached when the order landed.
    /// Refused loudly outside execution, while paused, or for a hull
    /// the caller does not command: never queued, never synthesised.
    pub fn order_unit(
        &self,
        token: &str,
        game_id: i64,
        unit_id: i64,
        heading_deg: f64,
        speed_kn: f64,
    ) -> Result<GameFix, BackendError> {
        let data = self.post_created(
            token,
            &format!("/games/{game_id}/units/{unit_id}/order"),
            serde_json::json!({ "heading_deg": heading_deg, "speed_kn": speed_kn }),
        )?;
        let malformed = |field: &str| {
            BackendError::Other(format!(
                "malformed Minos order response: missing or invalid {field}"
            ))
        };
        let response_unit = data["id_unit"].as_i64().ok_or_else(|| malformed("id_unit"))?;
        if response_unit != unit_id {
            return Err(BackendError::Other(format!(
                "malformed Minos order response: id_unit {response_unit} does not match path {unit_id}"
            )));
        }
        let assumed_time = data["assumed_time"]
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| malformed("assumed_time"))?
            .to_string();
        let latitude = data["latitude"]
            .as_f64()
            .ok_or_else(|| malformed("latitude"))?;
        let longitude = data["longitude"]
            .as_f64()
            .ok_or_else(|| malformed("longitude"))?;
        let applied_heading = data["heading_deg"]
            .as_f64()
            .ok_or_else(|| malformed("heading_deg"))?;
        let applied_speed = data["speed_kn"]
            .as_f64()
            .ok_or_else(|| malformed("speed_kn"))?;
        let clamped = data["clamped"]
            .as_bool()
            .ok_or_else(|| malformed("clamped"))?;
        let requested_speed = match data.get("requested_speed_kn") {
            None | Some(serde_json::Value::Null) => None,
            Some(value) => Some(
                value
                    .as_f64()
                    .ok_or_else(|| malformed("requested_speed_kn"))?,
            ),
        };
        if !applied_heading.is_finite()
            || !(0.0..360.0).contains(&applied_heading)
            || !applied_speed.is_finite()
            || applied_speed < 0.0
            || !latitude.is_finite()
            || !(-90.0..=90.0).contains(&latitude)
            || !longitude.is_finite()
            || !(-180.0..=180.0).contains(&longitude)
            || requested_speed.is_some_and(|requested| !requested.is_finite() || requested < 0.0)
            || clamped != requested_speed.is_some()
            || (clamped && requested_speed.is_some_and(|requested| requested <= applied_speed))
        {
            return Err(BackendError::Other(
                "malformed Minos order response: invalid authoritative GameFix values".into(),
            ));
        }
        Ok(GameFix {
            unit_id: response_unit,
            assumed_time,
            latitude,
            longitude,
            heading: applied_heading,
            speed: applied_speed,
            requested_speed,
            clamped,
            created_at: data["created_at"].as_str().map(str::to_string),
            created_by: data["created_by"].as_i64(),
        })
    }

    /// Where the fleet is at one assumed instant (C3): the plot read.
    /// Omit `at` for now (the client does not know scenario now); omit
    /// `from_unit` for a plain position read. Only hulls with a leg at
    /// the instant appear.
    pub fn positions(
        &self,
        token: &str,
        game_id: i64,
        at: Option<&str>,
        from_unit: Option<i64>,
    ) -> Result<PositionList, BackendError> {
        let mut qs = Vec::new();
        if let Some(at) = at {
            qs.push(format!("at={}", Self::enc(at)));
        }
        if let Some(f) = from_unit {
            qs.push(format!("from_unit={f}"));
        }
        let path = if qs.is_empty() {
            format!("/games/{game_id}/positions")
        } else {
            format!("/games/{game_id}/positions?{}", qs.join("&"))
        };
        let data = self.get(token, &path)?;
        let malformed = |field: &str| {
            BackendError::Other(format!(
                "malformed Minos positions response: missing or invalid {field}"
            ))
        };
        let assumed_time = data["assumed_time"]
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| malformed("assumed_time"))?
            .to_string();
        let rows = data["positions"]
            .as_array()
            .ok_or_else(|| malformed("positions"))?;
        let positions = rows
            .iter()
            .map(|p| {
                Ok(GameHullPos {
                    unit_id: p["id_unit"]
                        .as_i64()
                        .ok_or_else(|| malformed("id_unit"))?,
                    latitude: p["latitude"]
                        .as_f64()
                        .ok_or_else(|| malformed("latitude"))?,
                    longitude: p["longitude"]
                        .as_f64()
                        .ok_or_else(|| malformed("longitude"))?,
                    heading: p["heading_deg"]
                        .as_f64()
                        .ok_or_else(|| malformed("heading_deg"))?,
                    speed: p["speed_kn"]
                        .as_f64()
                        .ok_or_else(|| malformed("speed_kn"))?,
                    clamped: p["clamped"]
                        .as_bool()
                        .ok_or_else(|| malformed("clamped"))?,
                    assumed_time: p["assumed_time"]
                        .as_str()
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| malformed("assumed_time"))?
                        .to_string(),
                })
            })
            .collect::<Result<Vec<_>, BackendError>>()?;
        if positions.iter().any(|position| {
            !position.latitude.is_finite()
                || !(-90.0..=90.0).contains(&position.latitude)
                || !position.longitude.is_finite()
                || !(-180.0..=180.0).contains(&position.longitude)
                || !position.heading.is_finite()
                || !(0.0..360.0).contains(&position.heading)
                || !position.speed.is_finite()
                || position.speed < 0.0
        }) {
            return Err(malformed("position values"));
        }
        Ok(PositionList {
            assumed_time,
            positions,
        })
    }

    /// Stop the scenario clock and close the exercise to orders (H2).
    /// Pause appends a zero-rate segment and clears accepting — the
    /// chosen factor is kept for resume, so read that off the answer,
    /// never from memory. There is no clock GET: this write IS the
    /// read, and its answer is the GameClock to display.
    pub fn pause_game(&self, token: &str, game_id: i64) -> Result<GameClock, BackendError> {
        let data = self.post_empty(token, &format!("/games/{game_id}/pause"))?;
        Ok(Self::parse_clock(&data))
    }

    /// Restart the scenario clock at the chosen rate and reopen the
    /// exercise to orders (H2). Answers the GameClock.
    pub fn resume_game(&self, token: &str, game_id: i64) -> Result<GameClock, BackendError> {
        let data = self.post_empty(token, &format!("/games/{game_id}/resume"))?;
        Ok(Self::parse_clock(&data))
    }

    /// Change how fast the scenario clock runs (H2, must be > 0 — zero
    /// is not a rate, it is a pause through its own endpoint, leaving
    /// different evidence). Applies at once while running; while held
    /// only the chosen rate is stored. Answers the GameClock.
    pub fn set_time_factor(
        &self,
        token: &str,
        game_id: i64,
        factor: f64,
    ) -> Result<GameClock, BackendError> {
        let data = self.patch(
            token,
            &format!("/games/{game_id}/time-factor"),
            serde_json::json!({ "time_factor": factor }),
        )?;
        Ok(Self::parse_clock(&data))
    }

    /// The clock answer shared by pause, resume, and factor writes.
    /// `running` comes from the last segment by contract — while held
    /// it disagrees with `time_factor`, and that disagreement IS the
    /// paused state, not a parse problem.
    fn parse_clock(v: &serde_json::Value) -> GameClock {
        GameClock {
            state: v["state"].as_str().unwrap_or("").to_string(),
            actual_start: v["actual_start"].as_str().map(|s| s.to_string()),
            assumed_start: v["assumed_start"].as_str().map(|s| s.to_string()),
            assumed_now: v["assumed_now"].as_str().map(|s| s.to_string()),
            time_factor: v["time_factor"].as_f64().unwrap_or(0.0),
            accepting_actions: v["accepting_actions"].as_bool().unwrap_or(false),
            running: v["running"].as_bool().unwrap_or(false),
            segments: v["segments"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(|s| GameClockSegment {
                    factor: s["factor"].as_f64().unwrap_or(0.0),
                    effective_from: s["effective_from"].as_str().unwrap_or("").to_string(),
                    source: s["source"].as_str().unwrap_or("").to_string(),
                })
                .collect(),
        }
    }

    /// Send a message into a game (#99): telegram or administrative,
    /// marked TERBUKA/TERBATAS/RAHASIA, broadcast when neither to nor
    /// cc names anyone. The answer is the stored message as the caller
    /// may see it (the real author rides only where entitled).
    pub fn send_message(
        &self,
        token: &str,
        game_id: i64,
        draft: &MsgDraft,
    ) -> Result<InboxMsg, BackendError> {
        let mut body = serde_json::json!({
            "kind": draft.kind,
            "classification": draft.classification,
            "content": draft.content,
            "id_degree": draft.degree,
        });
        if !draft.to.is_empty() {
            body["to"] = serde_json::Value::Array(
                draft.to.iter().map(|u| serde_json::Value::from(*u)).collect(),
            );
        }
        if !draft.cc.is_empty() {
            body["cc"] = serde_json::Value::Array(
                draft.cc.iter().map(|u| serde_json::Value::from(*u)).collect(),
            );
        }
        if let Some(r) = draft.assumed_role {
            body["id_assumed_role"] = serde_json::Value::from(r);
        }
        if let Some(r) = draft.reply_to {
            body["id_reply_to"] = serde_json::Value::from(r);
        }
        if let Some(t) = draft.msg_type {
            body["id_type"] = serde_json::Value::from(t);
        }
        for (k, v) in [
            ("callsign", &draft.callsign),
            ("sending_note", &draft.sending_note),
            ("group_name", &draft.group_name),
            ("per", &draft.per),
            ("registration_number", &draft.registration_number),
        ] {
            if !v.trim().is_empty() {
                body[k] = serde_json::Value::String(v.clone());
            }
        }
        let data = self.post(token, &format!("/games/{game_id}/messages"), body)?;
        Self::parse_inbox(&data).ok_or_else(|| {
            BackendError::Other("send answer without a message".to_string())
        })
    }

    /// One inbox page (#99): the messages the caller may see, newest
    /// first by server order. Capped at 100 — a busier exercise pages
    /// (later slice), it does not silently truncate.
    pub fn inbox_list(
        &self,
        token: &str,
        game_id: i64,
        kind: Option<&str>,
        only_mine: bool,
    ) -> Result<Vec<InboxMsg>, BackendError> {
        Ok(self.inbox_page(token, game_id, kind, only_mine, 1, 100)?.messages)
    }

    /// One inbox page (messages ticket): kind + mine narrows ride
    /// along, offset paging binds the UI's prev/next. The leftover
    /// single-100 call above stays for callers with no pager.
    pub fn inbox_page(
        &self,
        token: &str,
        game_id: i64,
        kind: Option<&str>,
        only_mine: bool,
        page: u64,
        size: u64,
    ) -> Result<InboxPage, BackendError> {
        let mut path = format!("/games/{game_id}/messages?page_size={size}&page_number={page}");
        if let Some(k) = kind {
            path += &format!("&kind={}", Self::enc(k));
        }
        if only_mine {
            path += "&only_mine=true";
        }
        let resp = self
            .client
            .get(&format!("{}{}", self.base_url, path))
            .bearer_auth(token)
            .send()?;
        let (data, meta) = unwrap_envelope_page(resp)?;
        let messages = data
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|m| Self::parse_inbox(m))
            .collect();
        Ok(InboxPage {
            messages,
            total_records: meta["total_records"].as_u64().unwrap_or(0),
            total_pages: meta["total_pages"].as_u64().unwrap_or(1),
            current_page: meta["current_page"].as_u64().unwrap_or(page),
            has_next: meta["has_next_page"].as_bool().unwrap_or(false),
            has_prev: meta["has_previous_page"].as_bool().unwrap_or(false),
        })
    }

    /// One message as read back (messages ticket): the same
    /// projection the page renders, for the detail pane.
    pub fn get_message(
        &self,
        token: &str,
        game_id: i64,
        message_id: i64,
    ) -> Result<InboxMsg, BackendError> {
        let data = self.get(token, &format!("/games/{game_id}/messages/{message_id}"))?;
        Self::parse_inbox(&data).ok_or_else(|| {
            BackendError::Other(format!("message #{message_id} unreadable"))
        })
    }

    /// Delete one message (messages ticket): a sender's withdrawal
    /// clears it for everybody, a recipient's hides their own view —
    /// a broadcast a caller merely saw refuses loudly with the
    /// reason. No edit route exists, so none is modeled.
    pub fn delete_message(
        &self,
        token: &str,
        game_id: i64,
        message_id: i64,
    ) -> Result<InboxMsg, BackendError> {
        let data = self.delete(token, &format!("/games/{game_id}/messages/{message_id}"))?;
        Self::parse_inbox(&data).ok_or_else(|| {
            BackendError::Other(format!("message #{message_id} unreadable after delete"))
        })
    }

    /// Closure timeline page (assessment ticket): the merged event
    /// stream with its cursor block. Filters pass through verbatim —
    /// empty means unfiltered; the server owns window parsing (a bad
    /// timestamp is a loud 400 naming the field). Events arrive in
    /// assumed_at → source-rank → id order; the client never re-sorts.
    pub fn timeline_page(
        &self,
        token: &str,
        game_id: i64,
        source: Option<&str>,
        personnel: Option<i64>,
        unit: Option<i64>,
        from_assumed: Option<&str>,
        to_assumed: Option<&str>,
        cursor: Option<&str>,
        limit: u64,
    ) -> Result<TimelinePage, BackendError> {
        let mut path = format!("/games/{game_id}/timeline?limit={limit}");
        if let Some(s) = source {
            path += &format!("&source={}", Self::enc(s));
        }
        if let Some(p) = personnel {
            path += &format!("&personnel={p}");
        }
        if let Some(u) = unit {
            path += &format!("&unit={u}");
        }
        if let Some(f) = from_assumed {
            path += &format!("&from_assumed={}", Self::enc(f));
        }
        if let Some(t) = to_assumed {
            path += &format!("&to_assumed={}", Self::enc(t));
        }
        if let Some(c) = cursor {
            path += &format!("&cursor={}", Self::enc(c));
        }
        let resp = self
            .client
            .get(&format!("{}{}", self.base_url, path))
            .bearer_auth(token)
            .send()?;
        let (data, meta) = unwrap_envelope_page(resp)?;
        let events = data
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|e| {
                Some(TimelineEvent {
                    id: e["id"].as_str()?.to_string(),
                    etype: e["type"].as_str().unwrap_or("").to_string(),
                    assumed_at: e["assumed_at"].as_str().unwrap_or("").to_string(),
                    data: e["data"].clone(),
                })
            })
            .collect();
        Ok(TimelinePage {
            events,
            next_cursor: meta["next_cursor"].as_str().map(|s| s.to_string()),
            has_more: meta["has_more"].as_bool().unwrap_or(false),
        })
    }

    /// Closure reviews (assessment ticket): the exercise's documents,
    /// optionally narrowed to one subject or one author. A separate
    /// surface from the timeline by product rule, not by transport.
    pub fn reviews_list(
        &self,
        token: &str,
        game_id: i64,
        personnel: Option<i64>,
        author: Option<i64>,
    ) -> Result<Vec<Review>, BackendError> {
        let mut path = format!("/games/{game_id}/review?page_size=100&page_number=1");
        if let Some(p) = personnel {
            path += &format!("&id_personnel={p}");
        }
        if let Some(a) = author {
            path += &format!("&id_author={a}");
        }
        let data = self.get(token, &path)?;
        Ok(data
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|r| {
                Some(Review {
                    id: r["id"].as_i64()?,
                    personnel_id: r["personnel"]["id"].as_i64().unwrap_or(0),
                    personnel_name: r["personnel"]["name"].as_str().unwrap_or("").to_string(),
                    author_id: r["author"]["id"].as_i64().unwrap_or(0),
                    author_name: r["author"]["name"].as_str().unwrap_or("").to_string(),
                    body: r["body"].as_str().unwrap_or("").to_string(),
                    revised: r["revised"].as_bool().unwrap_or(false),
                    created_at: r["created_at"].as_str().unwrap_or("").to_string(),
                    updated_at: r["updated_at"].as_str().unwrap_or("").to_string(),
                })
            })
            .collect())
    }

    /// File one review: subject plus free-text body. The author is the
    /// caller (never sent). No phase gate backend-side — the workspace
    /// surface is the Closure home by placement, not by refusal.
    pub fn file_review(
        &self,
        token: &str,
        game_id: i64,
        personnel_id: i64,
        body: &str,
    ) -> Result<Review, BackendError> {
        let data = self.post(
            token,
            &format!("/games/{game_id}/review"),
            serde_json::json!({ "id_personnel": personnel_id, "body": body }),
        )?;
        Ok(Self::parse_review(&data, personnel_id))
    }

    /// Revise one review's words (subject is fixed at filing). The
    /// author alone may — anyone else gets an explicit 403, which the
    /// UI surfaces instead of pre-hiding with stale data.
    pub fn revise_review(
        &self,
        token: &str,
        game_id: i64,
        review_id: i64,
        body: &str,
    ) -> Result<Review, BackendError> {
        let data = self.patch(
            token,
            &format!("/games/{game_id}/review/{review_id}"),
            serde_json::json!({ "body": body }),
        )?;
        Ok(Self::parse_review(&data, 0))
    }

    /// One review answer, filed or revised: every field always
    /// present, `revised` derived server-side.
    fn parse_review(v: &serde_json::Value, personnel_fallback: i64) -> Review {
        Review {
            id: v["id"].as_i64().unwrap_or(0),
            personnel_id: v["personnel"]["id"].as_i64().unwrap_or(personnel_fallback),
            personnel_name: v["personnel"]["name"].as_str().unwrap_or("").to_string(),
            author_id: v["author"]["id"].as_i64().unwrap_or(0),
            author_name: v["author"]["name"].as_str().unwrap_or("").to_string(),
            body: v["body"].as_str().unwrap_or("").to_string(),
            revised: v["revised"].as_bool().unwrap_or(false),
            created_at: v["created_at"].as_str().unwrap_or("").to_string(),
            updated_at: v["updated_at"].as_str().unwrap_or("").to_string(),
        }
    }

    /// Hull pictures manifest (images ticket): which hulls have a
    /// picture and the content version. Manifest-first: the UI checks
    /// this before resolving any URL, so hulls without pictures cost
    /// no request. The version answers staleness without headers.
    pub fn unit_image_manifest(
        &self,
        token: &str,
    ) -> Result<ImageManifest, BackendError> {
        let data = self.get(token, "/assets/unit-images/manifest")?;
        Ok(Self::parse_image_manifest(&data))
    }

    /// The manifest read, split from the transport so it is testable
    /// against raw JSON. Pure projection: no defaults, no invention.
    pub fn parse_image_manifest(data: &serde_json::Value) -> ImageManifest {
        // The ETag is Minos' content identity: it covers the missing
        // count and every entry's path, type, size, update time, and
        // published physical measurements. `content_changed_at` is only
        // the newest included content clock and may stay fixed while the
        // set of pictures changes.
        let version = data["etag"].as_str().unwrap_or("").to_string();
        let entries = data["entries"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|e| {
                Some(UnitImageEntry {
                    asset_kind: e["asset_kind"].as_str().unwrap_or("").to_string(),
                    unit_id: e["id_unit"].as_i64()?,
                    name: e["name"].as_str().unwrap_or("").to_string(),
                    hull_number: e["hull_number"].as_str().map(|s| s.to_string()),
                    file_name: e["file_name"].as_str().unwrap_or("").to_string(),
                    content_type: e["content_type"].as_str().unwrap_or("").to_string(),
                    size_bytes: e["size_bytes"].as_i64().unwrap_or(0),
                    // Intrinsic dimensions ride the manifest so the
                    // renderer can size the asset before its bytes
                    // arrive. Validation policy belongs to the
                    // renderer, not to transport parsing.
                    width_px: e["width_px"].as_u64().map(|v| v as u32),
                    height_px: e["height_px"].as_u64().map(|v| v as u32),
                    // Physical measurements are part of the manifest
                    // contract when Minos publishes them. Keep null and
                    // malformed values as unknown; the map layer will
                    // refuse to project a partial or non-positive size.
                    loa_m: e["loa_m"].as_f64(),
                    beam_m: e["beam_m"].as_f64(),
                })
            })
            .collect();
        ImageManifest {
            version,
            entry_count: data["entry_count"].as_i64().unwrap_or(0) as usize,
            units_without_image: data["units_without_image_count"]
                .as_i64()
                .unwrap_or(0),
            entries,
        }
    }

    /// One hull's temporary picture URL (images ticket): presigned,
    /// time-limited, absent when the hull has none. Transient by
    /// contract — resolved at render, cached in session memory only,
    /// never mirrored.
    pub fn hull_image_url(
        &self,
        token: &str,
        unit_id: i64,
    ) -> Result<Option<String>, BackendError> {
        let data = self.get(token, &format!("/units/{unit_id}"))?;
        Ok(data["image_url"].as_str().map(|s| s.to_string()))
    }

    /// The game's task organisation (hierarchy ticket): a forest,
    /// highest echelon first, children nested. Staff-gated like the
    /// roster and pieces; the read stays open after execution for
    /// review. Writes (nodes, moves, piece assignment) are a later
    /// slice — this call draws the tree.
    pub fn game_hierarchy(
        &self,
        token: &str,
        game_id: i64,
    ) -> Result<Vec<HierarchyNode>, BackendError> {
        let data = self.get(token, &format!("/games/{game_id}/hierarchy"))?;
        Ok(data
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(Self::parse_hierarchy_node)
            .collect())
    }

    /// One forest node, children recursive. A missing id drops the
    /// node (unrenderable); everything else defaults — the tree
    /// draws what the server sent, never refuses a page for one row.
    fn parse_hierarchy_node(v: &serde_json::Value) -> Option<HierarchyNode> {
        Some(HierarchyNode {
            id: v["id"].as_i64()?,
            echelon_name: v["echelon_name"].as_str().unwrap_or("").to_string(),
            echelon_rank: v["echelon_rank"].as_i64().unwrap_or(0),
            parent_id: v["parent_id"].as_i64(),
            name: v["name"].as_str().unwrap_or("").to_string(),
            icon: v["icon"].as_str().unwrap_or("").to_string(),
            note: v["note"].as_str().unwrap_or("").to_string(),
            children: v["children"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .filter_map(Self::parse_hierarchy_node)
                .collect(),
        })
    }

    /// Closure judgements (assessment ticket): the exercise's marks,
    /// optionally narrowed to one person's debrief read. No fix-list
    /// route exists, so citation travels only inside answers.
    pub fn judgements_list(
        &self,
        token: &str,
        game_id: i64,
        personnel: Option<i64>,
    ) -> Result<Vec<Judgement>, BackendError> {
        let mut path = format!("/games/{game_id}/judgements?page_size=100&page_number=1");
        if let Some(p) = personnel {
            path += &format!("&id_personnel={p}");
        }
        let data = self.get(token, &path)?;
        Ok(data
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|j| {
                Some(Judgement {
                    id: j["id"].as_i64()?,
                    personnel_id: j["personnel"]["id"].as_i64().unwrap_or(0),
                    personnel_name: j["personnel"]["name"].as_str().unwrap_or("").to_string(),
                    judge_id: j["judge"]["id"].as_i64().unwrap_or(0),
                    judge_name: j["judge"]["name"].as_str().unwrap_or("").to_string(),
                    cited_unit: j["action"]["unit_name"].as_str().map(|s| s.to_string()),
                    assumed_at: j["assumed_at"].as_str().unwrap_or("").to_string(),
                    score: j["score"].as_str().unwrap_or("").to_string(),
                    created_at: j["created_at"].as_str().unwrap_or("").to_string(),
                })
            })
            .collect())
    }

    /// Record one judgement: subject plus free-text score. The judge
    /// is the caller (never sent); self-judging and non-judge callers
    /// fail loudly server-side. No id_fix — no public fix-list route
    /// exists to cite from (map fog, minos-side).
    pub fn record_judgement(
        &self,
        token: &str,
        game_id: i64,
        personnel_id: i64,
        score: &str,
    ) -> Result<Judgement, BackendError> {
        let body = serde_json::json!({
            "id_personnel": personnel_id,
            "score": score,
        });
        let data = self.post(
            token,
            &format!("/games/{game_id}/judgements"),
            body,
        )?;
        Ok(Judgement {
            id: data["id"].as_i64().unwrap_or(0),
            personnel_id: data["personnel"]["id"].as_i64().unwrap_or(personnel_id),
            personnel_name: data["personnel"]["name"].as_str().unwrap_or("").to_string(),
            judge_id: data["judge"]["id"].as_i64().unwrap_or(0),
            judge_name: data["judge"]["name"].as_str().unwrap_or("").to_string(),
            cited_unit: data["action"]["unit_name"].as_str().map(|s| s.to_string()),
            assumed_at: data["assumed_at"].as_str().unwrap_or("").to_string(),
            score: data["score"].as_str().unwrap_or("").to_string(),
            created_at: data["created_at"].as_str().unwrap_or("").to_string(),
        })
    }

    /// Record the caller's read receipt (#99): addressed messages only
    /// — broadcasts carry no per-recipient receipt, and the server
    /// refuses those loudly. Answers the message with the receipt on.
    pub fn mark_read(
        &self,
        token: &str,
        game_id: i64,
        message_id: i64,
    ) -> Result<InboxMsg, BackendError> {
        let data = self.post_empty(token, &format!("/games/{game_id}/messages/{message_id}/read"))?;
        Self::parse_inbox(&data).ok_or_else(|| {
            BackendError::Other("read answer without a message".to_string())
        })
    }

    /// Identities staff may send as (#99): per-game scenario roles.
    pub fn scenario_roles(
        &self,
        token: &str,
        game_id: i64,
    ) -> Result<Vec<ScenarioRole>, BackendError> {
        let data = self.get(token, &format!("/games/{game_id}/scenario-roles"))?;
        Ok(data
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|r| {
                Some(ScenarioRole {
                    id: r["id"].as_i64()?,
                    name: r["name"].as_str().unwrap_or("").to_string(),
                })
            })
            .collect())
    }

    /// Author a scenario role (#99): Game-Master authority, refused
    /// loudly otherwise. Answers the created role.
    pub fn create_scenario_role(
        &self,
        token: &str,
        game_id: i64,
        name: &str,
    ) -> Result<ScenarioRole, BackendError> {
        let data = self.post(
            token,
            &format!("/games/{game_id}/scenario-roles"),
            serde_json::json!({ "name": name }),
        )?;
        Ok(ScenarioRole {
            id: data["id"].as_i64().unwrap_or(0),
            name: data["name"].as_str().unwrap_or(name).to_string(),
        })
    }

    /// One message as the inbox draws it (#99): the drawable subset of
    /// the response. `sender` is the assumed identity where one was
    /// claimed, else the real author where entitled, else unknown —
    /// the server omits (never blanks) what the caller may not see, so
    /// the island renders assumed identity alone there by construction.
    fn parse_inbox(v: &serde_json::Value) -> Option<InboxMsg> {
        Some(InboxMsg {
            id: v["id"].as_i64()?,
            game_id: v["id_game"].as_i64().unwrap_or(0),
            kind: v["kind"].as_str().unwrap_or("").to_string(),
            class_label: v["classification"]["label"].as_str().unwrap_or("").to_string(),
            sender: v["sender"]["assumed_role"]["name"]
                .as_str()
                .map(|s| s.to_string())
                .or_else(|| {
                    v["sender"]["author"]["name"].as_str().map(|s| s.to_string())
                })
                .unwrap_or_else(|| "unknown sender".to_string()),
            broadcast: v["broadcast"].as_bool().unwrap_or(false),
            content: v["content"].as_str().unwrap_or("").to_string(),
            callsign: v["callsign"].as_str().unwrap_or("").to_string(),
            created_at: v["created_at"].as_str().unwrap_or("").to_string(),
            recipients: v["recipients"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .filter_map(|r| {
                    Some(MsgRecipient {
                        user_id: r["id_user"].as_i64()?,
                        kind: r["kind"].as_str().unwrap_or("").to_string(),
                        read_at: r["read_at"].as_str().map(|s| s.to_string()),
                    })
                })
                .collect(),
        })
    }

    /// One placement row of the setup view.
    fn parse_placements(v: &serde_json::Value) -> PlacementList {
        let placements: Vec<GamePlacement> = v["placements"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|p| {
                Some(GamePlacement {
                    unit_id: p["id_unit"].as_i64()?,
                    latitude: p["latitude"].as_f64().unwrap_or(0.0),
                    longitude: p["longitude"].as_f64().unwrap_or(0.0),
                })
            })
            .collect();
        PlacementList {
            placed: v["placed"].as_u64().map(|n| n as usize).unwrap_or(placements.len()),
            unplaced: v["unplaced"].as_i64().unwrap_or(0),
            ready: v["ready"].as_bool().unwrap_or(false),
            placements,
        }
    }

    /// The join/readiness answer pair: the game entered plus the
    /// caller's own row. A missing game is a decode error, never an
    /// empty default the UI could mistake for a room.
    fn parse_join(v: &serde_json::Value) -> Result<JoinResult, BackendError> {
        let game_id = v["game"]["id"]
            .as_i64()
            .ok_or_else(|| BackendError::Other("join answer without a game".to_string()))?;
        let user_id = v["participant"]["id_user"]
            .as_i64()
            .ok_or_else(|| BackendError::Other("join answer without a participant".to_string()))?;
        Ok(JoinResult {
            game_id,
            game_name: v["game"]["name"].as_str().unwrap_or("").to_string(),
            game_state: v["game"]["state"].as_str().unwrap_or("").to_string(),
            user_id,
            user_name: v["participant"]["user_name"].as_str().unwrap_or("").to_string(),
            role_id: v["participant"]["id_game_role"].as_i64().unwrap_or(0),
            role_name: v["participant"]["role_name"].as_str().unwrap_or("").to_string(),
            judge: v["participant"]["is_judge_side"].as_bool().unwrap_or(false),
            ready: v["participant"]["is_ready"].as_bool().unwrap_or(false),
            // The caller's own pieces (empty, never null by
            // contract): the only order authority a participant who
            // can never read the staff unit list will ever see.
            commanded_units: Self::parse_units(&v["commanded_units"]),
        })
    }
}

/// Backend user row for the session-users panel (live-read, no mirror).
/// Carries the personnel identity fields the picker shows — rank, unit,
/// position — so same-named accounts stay distinguishable, plus the
/// account status lookup row.
#[derive(Debug, Clone, PartialEq)]
pub struct BackendUser {
    pub id: i64,
    pub username: String,
    pub name: String,
    pub nrp: String,
    pub pangkat: String,
    pub satuan: String,
    pub jabatan: String,
    pub status_id: Option<i64>,
    pub status_name: String,
}

/// Game summary row for the session-users game picker.
#[derive(Debug, Clone, PartialEq)]
pub struct GameRow {
    pub id: i64,
    pub name: String,
    pub state: String,
}

/// Partial game edit: the six text fields the create form owns.
/// None means leave alone (absent on the wire, never null).
#[derive(Debug, Clone, Default)]
pub struct GameUpdate {
    pub name: Option<String>,
    pub description: Option<String>,
    pub purpose: Option<String>,
    pub target: Option<String>,
    pub area: Option<String>,
    pub map_tag: Option<String>,
}

/// One game's authoritative projection (H1): the detail read the held
/// game's local stage derives from. `state` is one of planning,
/// preparation, execution, closure; `mode` is always maneuver today.
/// The anchor + factor (H2) ride along: Minos stamps both on the
/// execution transition, so a client selecting mid-exercise learns
/// what the clock started from without ever being able to move it.
#[derive(Debug, Clone, PartialEq)]
pub struct GameDetail {
    pub id: i64,
    pub name: String,
    pub mode: String,
    pub state: String,
    pub actual_start: Option<String>,
    pub assumed_start: Option<String>,
    pub time_factor: f64,
    /// What personnel type to enter the room. Null until the game
    /// enters preparation — a planning game has no room to enter —
    /// and sent to every entitled caller, participants included,
    /// because withholding it from the group that must use it would
    /// make the feature impossible.
    pub room_key: Option<String>,
}

/// One hull's starting position on the map (C2): a Minos placement row.
/// No unit name by contract — the pieces list owns names.
#[derive(Debug, Clone, PartialEq)]
pub struct GamePlacement {
    pub unit_id: i64,
    pub latitude: f64,
    pub longitude: f64,
}

/// The setup view of an exercise's map (C2): placements plus the
/// gate's placement arithmetic, straight from the server.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlacementList {
    pub placements: Vec<GamePlacement>,
    pub placed: usize,
    pub unplaced: i64,
    pub ready: bool,
}

/// The join/readiness answer pair (C2): the game entered plus the
/// caller's own participant row plus the hulls they may steer.
/// Both write paths (join, readiness) answer through the same
/// writer, so both carry all three.
#[derive(Debug, Clone, PartialEq)]
pub struct JoinResult {
    pub game_id: i64,
    pub game_name: String,
    pub game_state: String,
    pub user_id: i64,
    pub user_name: String,
    pub role_id: i64,
    pub role_name: String,
    pub judge: bool,
    pub ready: bool,
    /// The caller's own pieces only — never the force. The order
    /// authority for callers who cannot read the staff unit list.
    pub commanded_units: Vec<GameUnit>,
}

/// One Closure timeline event: id `<source>:<row>`, the type
/// discriminator, its scenario instant, and the source-specific
/// payload (transition move, clock change, order + fix proof,
/// message, judgement). Payload fields read at render; unknown
/// shapes render their type + instant, never fail the page.
#[derive(Debug, Clone, PartialEq)]
pub struct TimelineEvent {
    pub id: String,
    pub etype: String,
    pub assumed_at: String,
    pub data: serde_json::Value,
}

/// One hull picture in the manifest (images ticket): identity for
/// humans and program, file location inside the bundle. The manifest
/// decides WHETHER a hull has a picture; the bytes always travel
/// through the hull's own temporary URL, never the mirror.
///
/// `width_px`/`height_px` are the intrinsic image dimensions Minos
/// measured when it validated the upload. `loa_m`/`beam_m` are the
/// optional physical measurements for the true-scale map renderer.
/// `None` means the backend did not publish the value — not that it is
/// zero.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct UnitImageEntry {
    /// Stable contract discriminator. The current manifest speaks
    /// `unit_image`; an unknown future kind is preserved here and
    /// rejected by the map layer, never guessed into a picture.
    pub asset_kind: String,
    pub unit_id: i64,
    pub name: String,
    pub hull_number: Option<String>,
    pub file_name: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub width_px: Option<u32>,
    pub height_px: Option<u32>,
    /// Physical dimensions published with the manifest entry, when the
    /// active Minos specification carries them. These are the map's
    /// true-scale inputs; image pixel dimensions are not a substitute.
    pub loa_m: Option<f64>,
    pub beam_m: Option<f64>,
}

/// The image manifest with its version block (images ticket):
/// `version` is Minos' ETag, the content identity used for cache
/// invalidation — NOT the per-unit presigned URL, which expires and is
/// never compared for change. `units_without_image` counts live hulls
/// still awaiting a picture, so the client can report coverage without
/// a second request.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ImageManifest {
    pub version: String,
    pub entry_count: usize,
    pub units_without_image: i64,
    pub entries: Vec<UnitImageEntry>,
}

/// One node of a game's task organisation (hierarchy ticket): the
/// tree the command centre draws, as a forest — a game may hold
/// several trees. Children always an array, rank orders the levels.
#[derive(Debug, Clone, PartialEq)]
pub struct HierarchyNode {
    pub id: i64,
    pub echelon_name: String,
    pub echelon_rank: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub icon: String,
    pub note: String,
    pub children: Vec<HierarchyNode>,
}

/// One review as read back (assessment ticket): who it is about,
/// who filed it, the body, and the derived revised flag. The author
/// alone may revise; nobody may delete — neither is modeled beyond
/// what the contract offers.
#[derive(Debug, Clone, PartialEq)]
pub struct Review {
    pub id: i64,
    pub personnel_id: i64,
    pub personnel_name: String,
    pub author_id: i64,
    pub author_name: String,
    pub body: String,
    pub revised: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// One judgement as read back (assessment ticket): who it is
/// about, who recorded it, the free-text score, and the cited order
/// when the judge cited one. Append-only by contract — no update or
/// delete exists, so none is modeled.
#[derive(Debug, Clone, PartialEq)]
pub struct Judgement {
    pub id: i64,
    pub personnel_id: i64,
    pub personnel_name: String,
    pub judge_id: i64,
    pub judge_name: String,
    pub cited_unit: Option<String>,
    pub assumed_at: String,
    pub score: String,
    pub created_at: String,
}

/// One timeline page with its cursor block: `next_cursor` absent on
/// the last page; `has_more` binds the load-more control.
#[derive(Debug, Clone, PartialEq)]
pub struct TimelinePage {
    pub events: Vec<TimelineEvent>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

/// One leg of a hull's track, as the API reports it (C3): the fix the
/// order closed at — where the hull HAD reached when the order landed,
/// derived from the previous leg and elapsed assumed time. The client
/// sends heading + speed only; position and time are the server's.
/// `requested_speed` is present only when the order was clamped.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct GameFix {
    #[serde(rename = "id_unit")]
    pub unit_id: i64,
    pub assumed_time: String,
    pub latitude: f64,
    pub longitude: f64,
    #[serde(rename = "heading_deg")]
    pub heading: f64,
    #[serde(rename = "speed_kn")]
    pub speed: f64,
    #[serde(rename = "requested_speed_kn")]
    pub requested_speed: Option<f64>,
    pub clamped: bool,
    /// Audit metadata emitted by MinOS' Go DTO but not required by the
    /// older OpenAPI schema. Keep it when present; never use it to fill a
    /// required movement field.
    pub created_at: Option<String>,
    pub created_by: Option<i64>,
}

/// One hull's computed position at the answered instant (C3): the leg
/// in force, so a client draws the arrow without a second call.
/// Measure fields (`from_unit` queries) are a later slice.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct GameHullPos {
    #[serde(rename = "id_unit")]
    pub unit_id: i64,
    pub latitude: f64,
    pub longitude: f64,
    #[serde(rename = "heading_deg")]
    pub heading: f64,
    #[serde(rename = "speed_kn")]
    pub speed: f64,
    /// Position clamp is separate from order-response speed clamping.
    pub clamped: bool,
    pub assumed_time: String,
}

/// Every hull's position at one assumed instant (C3): the command
/// centre's plot. The instant is echoed back — with `at` omitted the
/// client did not know what now was. Hulls with no leg at the instant
/// are absent, never zeroed: no position before the origin exists.
#[derive(Debug, Clone, PartialEq, Default, serde::Deserialize)]
pub struct PositionList {
    pub assumed_time: String,
    pub positions: Vec<GameHullPos>,
}

/// A game-position sample normalized across the REST plot and the
/// WebSocket publication. The REST contract always carries heading and
/// speed; the socket contract may omit either one, so both remain
/// optional here rather than being invented as zero.
#[derive(Debug, Clone, PartialEq)]
pub struct GamePositionUpdate {
    pub game_id: i64,
    pub assumed_time: String,
    pub positions: Vec<GamePositionFix>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GamePositionFix {
    pub unit_id: i64,
    pub latitude: f64,
    pub longitude: f64,
    pub heading_deg: Option<f64>,
    pub speed_kn: Option<f64>,
    pub assumed_time: String,
}

impl GamePositionUpdate {
    pub fn from_plot(game_id: i64, plot: PositionList) -> Self {
        let PositionList { assumed_time, positions: rows } = plot;
        let positions = rows
            .into_iter()
            .map(|p| GamePositionFix {
                unit_id: p.unit_id,
                latitude: p.latitude,
                longitude: p.longitude,
                heading_deg: Some(p.heading),
                speed_kn: Some(p.speed),
                assumed_time: p.assumed_time,
            })
            .collect();
        Self { game_id, assumed_time, positions }
    }

    pub fn from_fix(game_id: i64, fix: GameFix) -> Self {
        Self {
            game_id,
            assumed_time: fix.assumed_time.clone(),
            positions: vec![GamePositionFix {
                unit_id: fix.unit_id,
                latitude: fix.latitude,
                longitude: fix.longitude,
                heading_deg: Some(fix.heading),
                speed_kn: Some(fix.speed),
                assumed_time: fix.assumed_time,
            }],
        }
    }
}

/// The game identity is part of a position publication, not an
/// incidental envelope field. Keeping it here prevents a late result
/// from an old exercise entering the current Registry.
#[derive(Debug, Clone, PartialEq)]
pub struct GameOrderEvent {
    pub game_id: i64,
    pub fix: GameFix,
}

/// One entry in an exercise's rate history (H2): the factor in force
/// since `effective_from`, and why (`pause`, `resume`, `gm` …). Oldest
/// first — the order the assumed-time integral requires.
#[derive(Debug, Clone, PartialEq)]
pub struct GameClockSegment {
    pub factor: f64,
    pub effective_from: String,
    pub source: String,
}

/// An exercise's scenario clock (H2): the Game Master's chosen rate,
/// whether the clock advances, whether orders apply, and the instant
/// this response was built for. `time_factor` vs `running` vs
/// `accepting_actions` are three separate truths by contract: the
/// slider position, the last segment's rate, and the order gate — a
/// blackout runs with orders closed, and a pause holds the chosen
/// rate for resume. Never recompute assumed-now client-side: the
/// integral needs every segment plus the anchor, which is three
/// chances to disagree about what time it is in the exercise.
#[derive(Debug, Clone, PartialEq)]
pub struct GameClock {
    pub state: String,
    pub actual_start: Option<String>,
    pub assumed_start: Option<String>,
    pub assumed_now: Option<String>,
    pub time_factor: f64,
    pub accepting_actions: bool,
    pub running: bool,
    pub segments: Vec<GameClockSegment>,
}

/// One addressee with their own receipt state (#99).
#[derive(Debug, Clone, PartialEq)]
pub struct MsgRecipient {
    pub user_id: i64,
    pub kind: String,
    pub read_at: Option<String>,
}

/// One inbox message as the client draws it (#99).
#[derive(Debug, Clone, PartialEq)]
pub struct InboxMsg {
    pub id: i64,
    pub game_id: i64,
    pub kind: String,
    pub class_label: String,
    pub sender: String,
    pub broadcast: bool,
    pub content: String,
    pub callsign: String,
    pub created_at: String,
    pub recipients: Vec<MsgRecipient>,
}

/// One inbox page with its navigation block (messages ticket): the
/// server's offset paging, so the UI binds prev/next instead of a
/// silent first-100. Totals verify the thread, never re-sliced here.
#[derive(Debug, Clone, PartialEq)]
pub struct InboxPage {
    pub messages: Vec<InboxMsg>,
    pub total_records: u64,
    pub total_pages: u64,
    pub current_page: u64,
    pub has_next: bool,
    pub has_prev: bool,
}

/// A message being composed (#99): every send parameter. Empty
/// strings and empty audiences are omitted on the wire (no audience
/// is what makes a broadcast); `msg_type` stays None until the
/// vocabulary exists.
#[derive(Debug, Clone, Default)]
pub struct MsgDraft {
    pub kind: String,
    pub classification: String,
    pub content: String,
    pub to: Vec<i64>,
    pub cc: Vec<i64>,
    pub assumed_role: Option<i64>,
    pub reply_to: Option<i64>,
    pub degree: i64,
    pub msg_type: Option<i64>,
    pub callsign: String,
    pub sending_note: String,
    pub group_name: String,
    pub per: String,
    pub registration_number: String,
}

/// One scenario-role identity staff may send as (#99).
#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioRole {
    pub id: i64,
    pub name: String,
}

/// One game roster row: who holds which seat, and on which side.
#[derive(Debug, Clone, PartialEq)]
pub struct Participant {
    pub user_id: i64,
    pub user_name: String,
    pub role_id: i64,
    pub role_name: String,
    pub judge: bool,
    /// Readiness badge: cleared whenever a seat's role changes.
    pub ready: bool,
}

/// One game piece with its current commander, if any.
#[derive(Debug, Clone, PartialEq)]
pub struct GameUnit {
    pub unit_id: i64,
    pub unit_name: String,
    pub hull_number: String,
    pub commander_id: Option<i64>,
    pub commander_name: String,
    /// Task-organisation node the piece sits under, if any. Omitted
    /// for pieces in the exercise but outside the tree — absence is
    /// unassigned, never an error.
    pub hierarchy_node: Option<i64>,
}

fn b2i(b: bool) -> i64 {
    if b { 1 } else { 0 }
}

/// One hull's current figures (spec-sync ticket): the sim-driving
/// numbers for a register hull, carried by name so the picker can
/// match them to a runtime catalog class.
///
/// The physical measurements are the MAP's figures, not the sim's: a
/// hull's real size is what makes a thumbnail true-to-scale, and it
/// comes from Minos alone. Every one is `Option` and stays `None`
/// when the backend omits it — a missing `loa_m` means unknown, and
/// the client never substitutes a default that would draw a real-
/// world size nobody published.
#[derive(Debug, Clone, Default)]
pub struct HullSpec {
    pub unit_id: i64,
    pub version: i64,
    pub class_id: i64,
    pub class_name: String,
    pub speed_kn: Option<f64>,
    pub cruise_kn: Option<f64>,
    pub range_nm: Option<f64>,
    /// Length overall, metres — the thumbnail's long axis.
    pub loa_m: Option<f64>,
    /// Beam, metres — the thumbnail's short axis.
    pub beam_m: Option<f64>,
    /// Draft, metres. Carried for completeness; no first-slice
    /// rendering depends on it.
    pub draft_m: Option<f64>,
    pub displacement_standard_t: Option<f64>,
    pub displacement_full_t: Option<f64>,
    /// Maximum turn rate, degrees per second. The sim has its own
    /// figures; this is what the backend publishes, kept beside
    /// them rather than overriding anything.
    pub turn_rate_max_deg_s: Option<f64>,
}

impl MinosMaster {
    /// Current specification for one hull. Absent when the hull has no
    /// published version yet — the API invents no speed, neither do we.
    pub fn hull_spec(&self, token: &str, unit_id: i64) -> Result<HullSpec, BackendError> {
        let data = self.get(token, &format!("/units/{unit_id}"))?;
        Self::parse_hull_spec(&data, unit_id)
    }

    /// The spec read, split from the transport so it is testable
    /// against raw JSON. A hull with no published version is the
    /// NoSpec refusal — never a zero-filled stand-in, which would
    /// draw a 0 m ship.
    pub fn parse_hull_spec(
        data: &serde_json::Value,
        unit_id: i64,
    ) -> Result<HullSpec, BackendError> {
        let spec = data["current_specification"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        if spec.is_empty() {
            return Err(BackendError::NoSpec { unit_id });
        }
        let num = |key: &str| spec.get(key).and_then(|v| v.as_f64());
        Ok(HullSpec {
            unit_id,
            version: spec.get("version").and_then(|v| v.as_i64()).unwrap_or(0),
            class_id: data["unit_class"]["id"].as_i64().unwrap_or(0),
            class_name: data["unit_class"]["name"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            speed_kn: num("speed_max_surface_kn"),
            cruise_kn: num("speed_cruise_kn"),
            range_nm: num("range_nm"),
            loa_m: num("loa_m"),
            beam_m: num("beam_m"),
            draft_m: num("draft_m"),
            displacement_standard_t: num("displacement_standard_t"),
            displacement_full_t: num("displacement_full_t"),
            turn_rate_max_deg_s: num("turn_rate_max_deg_s"),
        })
    }

    /// Fetch one hull's current figures and store them. The caller
    /// checks spec_versions first when backfilling.
    pub fn sync_spec(
        &self,
        token: &str,
        conn: &rusqlite::Connection,
        unit_id: i64,
    ) -> Result<HullSpec, BackendError> {
        let spec = self.hull_spec(token, unit_id)?;
        // The mirror carries the MAP's figures alongside the sim's, so
        // a restored hull can be drawn true-to-scale without another
        // round-trip. Nulls are written as nulls, never dropped to a
        // default — an unpublished measurement stays unpublished.
        let body = serde_json::json!({
            "class_id": spec.class_id,
            "class_name": spec.class_name,
            "speed_kn": spec.speed_kn,
            "cruise_kn": spec.cruise_kn,
            "range_nm": spec.range_nm,
            "loa_m": spec.loa_m,
            "beam_m": spec.beam_m,
            "draft_m": spec.draft_m,
            "displacement_standard_t": spec.displacement_standard_t,
            "displacement_full_t": spec.displacement_full_t,
            "turn_rate_max_deg_s": spec.turn_rate_max_deg_s,
        })
        .to_string();
        crate::store::store_spec(conn, unit_id, spec.version, true, &body)
            .map_err(BackendError::Other)?;
        Ok(spec)
    }
}
