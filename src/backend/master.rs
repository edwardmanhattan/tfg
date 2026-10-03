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
        // The roster is WRAPPED. `GET /games/{id}/participants` answers
        // `{"data": {"participants": [...]}}`, so after the envelope is
        // unwrapped the value is an object, not an array — and `as_array()`
        // on it returned None, which turned a seated roster into an empty one
        // with no error anywhere.
        //
        // Found by driving: the seat was in the database (`seat.py` printed
        // it, and the API still returns it) while the island said "0 SEATED".
        // An empty roster is the one answer an operator cannot act on, so the
        // shape is read both ways rather than assumed.
        let rows = v
            .as_array()
            .cloned()
            .unwrap_or_else(|| v["participants"].as_array().cloned().unwrap_or_default());
        rows.iter()
            .filter_map(|p| {
                Some(Participant {
                    user_id: p["id_user"].as_i64()?,
                    user_name: p["user_name"].as_str().unwrap_or("").to_string(),
                    role_id: p["id_game_role"].as_i64().unwrap_or(0),
                    role_name: p["role_name"].as_str().unwrap_or("").to_string(),
                    judge: p["is_judge_side"].as_bool().unwrap_or(false),
                    ready: p["is_ready"].as_bool().unwrap_or(false),
                    joined_at: p["joined_at"]
                        .as_str()
                        .filter(|s| !s.trim().is_empty())
                        .map(|s| s.to_string()),
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
        // Three-state text is OMISSION, so it needs a different read from the
        // plain prose. `description`/`purpose`/`target` are always sent, so
        // absent means empty; `area`/`map_tag` are omitted both when unset
        // AND when withheld from a participant, and the two are
        // indistinguishable from here — see `GameDetail::area_is_withheld`.
        let absent_when_unset = |key: &str| {
            v[key]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
        };
        GameDetail {
            id: v["id"].as_i64().unwrap_or(fallback_id),
            name: v["name"].as_str().unwrap_or("").to_string(),
            mode: v["mode"].as_str().unwrap_or("").to_string(),
            state: v["state"].as_str().unwrap_or("").to_string(),
            description: v["description"].as_str().unwrap_or("").to_string(),
            purpose: v["purpose"].as_str().unwrap_or("").to_string(),
            target: v["target"].as_str().unwrap_or("").to_string(),
            area: absent_when_unset("area"),
            map_tag: absent_when_unset("map_tag"),
            overlay_text: absent_when_unset("overlay_text"),
            window: Self::parse_window(v),
            pace: v["pace"].as_str().and_then(GamePace::parse),
            time_factor: v["time_factor"].as_f64().unwrap_or(0.0),
            room_key: v["room_key"].as_str().map(|s| s.to_string()),
        }
    }

    /// The four clock fields read as one window.
    ///
    /// Absent is the wire's own word for unset — Minos omits rather than
    /// nulls — so a missing key and an explicit null read alike. That is the
    /// right answer for both: neither is a value, and neither is the empty
    /// string, which would be a window that ran for no time at all.
    fn parse_window(v: &serde_json::Value) -> TimeWindow {
        let t = |key: &str| v[key].as_str().map(|s| s.to_string());
        TimeWindow {
            actual_start: t("actual_start"),
            actual_end: t("actual_end"),
            assumed_start: t("assumed_start"),
            assumed_end: t("assumed_end"),
        }
    }

    /// Partial game edit (admin ticket): the six text fields the
    /// create form owns plus pace and the planned window — absent leaves
    /// alone, empty area/map_tag clears (nullable columns).
    /// Admin-granted, planning-only; the answer is the re-read detail,
    /// applied like any bundle detail.
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
        // Each clock key on its own, because the server merges them one at a
        // time: a PATCH that moves only `actual_end` is checked against the
        // start already ON THE GAME. Writing the pair together would be the
        // same request, and writing only the filled one is what lets an
        // operator fix one end without restating the other.
        text("actual_start", &upd.window.actual_start);
        text("actual_end", &upd.window.actual_end);
        text("assumed_start", &upd.window.assumed_start);
        text("assumed_end", &upd.window.assumed_end);
        if let Some(p) = upd.pace {
            body.insert(
                "pace".to_string(),
                serde_json::Value::String(p.wire().to_string()),
            );
        }
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

    // -----------------------------------------------------------------------
    // The scenario book.
    //
    // A game has exactly ONE book, and the book IS its ordered scenarios —
    // there is no books table and no cross-game template, so the composer
    // authors a game's own book rather than picking from a catalog.
    //
    /// What the server says about whether this game may leave preparation.
    ///
    /// `GET /games/{id}/readiness` — the same path the two readiness WRITES
    /// use, read rather than declared.
    pub fn game_readiness(
        &self,
        token: &str,
        game_id: i64,
    ) -> Result<GameReadinessView, BackendError> {
        let data = self.get(token, &format!("/games/{game_id}/readiness"))?;
        Ok(GameReadinessView {
            can_execute: data["can_execute"].as_bool().unwrap_or(false),
            blockers: data["blockers"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .filter_map(|b| b.as_str().map(|s| s.to_string()))
                .collect(),
            fast: data["fast"].as_bool().unwrap_or(false),
        })
    }

    // NOT the same thing as `scenario_roles` above. Those are the identities a
    // message is sent AS; these are the beats the Game Master authors. The
    // two names are one word apart and mean unrelated things, which the
    // backend says at its own router and which is why they are separate types
    // here rather than one `Scenario` with a flag.
    //
    // Steps arrive NESTED inside a scenario and nowhere else: there is no
    // step-list route and no single-step read. Every step write answers with
    // the whole scenario, because the ids are server-assigned and the
    // composer redraws from the answer rather than guessing one.
    // -----------------------------------------------------------------------

    /// The whole book. Answers either as a bare array or wrapped, so both are
    /// accepted rather than betting on the envelope.
    pub fn game_scenarios(
        &self,
        token: &str,
        game_id: i64,
    ) -> Result<Vec<GameScenario>, BackendError> {
        let data = self.get(token, &format!("/games/{game_id}/scenarios"))?;
        Ok(Self::parse_scenarios(&data, game_id))
    }

    /// One scenario with its steps.
    pub fn game_scenario(
        &self,
        token: &str,
        game_id: i64,
        scenario_id: i64,
    ) -> Result<GameScenario, BackendError> {
        let data = self.get(
            token,
            &format!("/games/{game_id}/scenarios/{scenario_id}"),
        )?;
        Self::parse_scenario(&data, game_id)
    }

    /// Add a scenario to the book's end.
    ///
    /// Append only: the caller does not choose a position, so two organizers
    /// adding at once cannot collide on the ordering index.
    pub fn create_game_scenario(
        &self,
        token: &str,
        game_id: i64,
        title: &str,
        description: &str,
    ) -> Result<GameScenario, BackendError> {
        let data = self.post(
            token,
            &format!("/games/{game_id}/scenarios"),
            serde_json::json!({ "title": title, "description": description }),
        )?;
        Self::parse_scenario(&data, game_id)
    }

    /// Rename a scenario or edit its description. Absent fields leave alone.
    pub fn update_game_scenario(
        &self,
        token: &str,
        game_id: i64,
        scenario_id: i64,
        title: Option<&str>,
        description: Option<&str>,
    ) -> Result<GameScenario, BackendError> {
        let mut body = serde_json::Map::new();
        if let Some(t) = title {
            body.insert("title".into(), serde_json::Value::String(t.into()));
        }
        if let Some(d) = description {
            body.insert(
                "description".into(),
                serde_json::Value::String(d.into()),
            );
        }
        let data = self.patch(
            token,
            &format!("/games/{game_id}/scenarios/{scenario_id}"),
            serde_json::Value::Object(body),
        )?;
        Self::parse_scenario(&data, game_id)
    }

    /// Put the book's scenarios in a new order: a whole-list PUT of every
    /// live id, checked server-side as a permutation.
    pub fn reorder_scenarios(
        &self,
        token: &str,
        game_id: i64,
        scenario_ids: &[i64],
    ) -> Result<Vec<GameScenario>, BackendError> {
        let data = self.put(
            token,
            &format!("/games/{game_id}/scenarios/order"),
            serde_json::json!({ "scenario_ids": scenario_ids }),
        )?;
        Ok(Self::parse_scenarios(&data, game_id))
    }

    /// Add a step to the end of a scenario.
    ///
    /// `window` is both ends or neither; the backend refuses a half-window
    /// and [`hhmm_ok`] refuses one before the request goes out.
    pub fn add_scenario_step(
        &self,
        token: &str,
        game_id: i64,
        scenario_id: i64,
        content: &str,
        window: Option<(&str, &str)>,
    ) -> Result<GameScenario, BackendError> {
        let mut body = serde_json::Map::new();
        body.insert("content".into(), serde_json::Value::String(content.into()));
        if let Some((start, end)) = window {
            body.insert("start_hour".into(), serde_json::Value::String(start.into()));
            body.insert("end_hour".into(), serde_json::Value::String(end.into()));
        }
        let data = self.post_created(
            token,
            &format!("/games/{game_id}/scenarios/{scenario_id}/steps"),
            serde_json::Value::Object(body),
        )?;
        Self::parse_scenario(&data, game_id)
    }

    /// Edit one step. Absent fields leave alone, so CLEARING a window means
    /// sending two empty strings rather than omitting them.
    pub fn update_scenario_step(
        &self,
        token: &str,
        game_id: i64,
        scenario_id: i64,
        step_id: i64,
        content: Option<&str>,
        window: Option<Option<(&str, &str)>>,
    ) -> Result<GameScenario, BackendError> {
        let mut body = serde_json::Map::new();
        if let Some(c) = content {
            body.insert("content".into(), serde_json::Value::String(c.into()));
        }
        if let Some(w) = window {
            match w {
                Some((start, end)) => {
                    body.insert("start_hour".into(), serde_json::Value::String(start.into()));
                    body.insert("end_hour".into(), serde_json::Value::String(end.into()));
                }
                None => {
                    body.insert("start_hour".into(), serde_json::Value::String(String::new()));
                    body.insert("end_hour".into(), serde_json::Value::String(String::new()));
                }
            }
        }
        let data = self.patch(
            token,
            &format!("/games/{game_id}/scenarios/{scenario_id}/steps/{step_id}"),
            serde_json::Value::Object(body),
        )?;
        Self::parse_scenario(&data, game_id)
    }

    /// Take a step out. Answers with the scenario that remains; positions are
    /// NOT renumbered, so a gap in the order is legal and expected.
    pub fn delete_scenario_step(
        &self,
        token: &str,
        game_id: i64,
        scenario_id: i64,
        step_id: i64,
    ) -> Result<GameScenario, BackendError> {
        let data = self.delete(
            token,
            &format!("/games/{game_id}/scenarios/{scenario_id}/steps/{step_id}"),
        )?;
        Self::parse_scenario(&data, game_id)
    }

    /// The book, from either envelope shape.
    fn parse_scenarios(
        v: &serde_json::Value,
        game_id: i64,
    ) -> Vec<GameScenario> {
        v.as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|s| Self::parse_scenario(s, game_id).ok())
            .collect()
    }

    /// One scenario, steps nested.
    ///
    /// A scenario without an id is a decode error rather than a dropped row:
    /// the composer's whole job is editing THIS scenario, and silently
    /// omitting it would leave the author looking at an empty list with no
    /// idea why.
    fn parse_scenario(
        v: &serde_json::Value,
        fallback_game: i64,
    ) -> Result<GameScenario, BackendError> {
        let id = v["id"]
            .as_i64()
            .ok_or_else(|| BackendError::Other("scenario answer without an id".to_string()))?;
        Ok(GameScenario {
            id,
            game_id: v["id_game"].as_i64().unwrap_or(fallback_game),
            position: v["position"].as_i64().unwrap_or(0),
            title: v["title"].as_str().unwrap_or("").to_string(),
            description: v["description"].as_str().unwrap_or("").to_string(),
            steps: v["steps"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .filter_map(|step| Self::parse_scenario_step(step, id))
                .collect(),
        })
    }

    /// One nested step.
    ///
    /// `scenario_id` comes from the PARENT, not from the step's own payload.
    /// The step is nested inside its scenario, so the parent already knows,
    /// and reading it from the child meant a step was silently dropped for
    /// omitting a field it did not need — which is how a response that is
    /// perfectly valid apart from that omission loses a row from the
    /// composer's list with nothing on screen to explain it.
    fn parse_scenario_step(
        v: &serde_json::Value,
        parent_scenario_id: i64,
    ) -> Option<GameScenarioStep> {
        Some(GameScenarioStep {
            id: v["id"].as_i64()?,
            scenario_id: parent_scenario_id,
            position: v["position"].as_i64().unwrap_or(0),
            // Content may legitimately be empty: a plan is authored while it
            // is being written, and the backend says so explicitly rather
            // than requiring a placeholder.
            content: v["content"].as_str().unwrap_or("").to_string(),
            // The window rides as authored `HHMM` strings. The derived
            // assumed-clock instants are recomputed by the server on every
            // clock write, so carrying them would show a stale reading.
            start_hour: v["start_hour"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.to_string()),
            end_hour: v["end_hour"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.to_string()),
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

/// A game's planned window: two clocks, each with a start and an end.
///
/// Incomplete is a real state and not a parse failure. The backend supplies
/// NO default for either end — the window is the Game Master's declaration of
/// when the exercise finishes — so a draft holding a start and no end is legal
/// all the way to the transition, which is where the gate refuses it. Four
/// independent options rather than two pairs is what lets an unfinished window
/// be written without pretending the finished half is whole.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TimeWindow {
    /// Real-world window, RFC3339 as Minos speaks it.
    pub actual_start: Option<String>,
    pub actual_end: Option<String>,
    /// Exercise clock. RFC3339 too, but only its TIME is meaningful; the date
    /// is a fixed base because nothing compares the two clocks.
    pub assumed_start: Option<String>,
    pub assumed_end: Option<String>,
}

impl TimeWindow {
    /// Whether the window says nothing at all. The edit form's "leave alone":
    /// six blank fields must not become a write that blanks a window somebody
    /// else authored.
    pub fn is_empty(&self) -> bool {
        self.actual_start.is_none()
            && self.actual_end.is_none()
            && self.assumed_start.is_none()
            && self.assumed_end.is_none()
    }
}

/// How much ceremony an exercise carries, which is a different question from
/// how fast its clock runs. Two values on the wire, and the second one buys
/// exactly one thing: the readiness declaration is waived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GamePace {
    Standard,
    Fast,
}

impl GamePace {
    /// Whether every non-judge participant must declare readiness before the
    /// exercise may start. The one condition `fast` waives, and the only thing
    /// this flag is allowed to switch off — the window, the pieces, the
    /// placements and the fighters all still apply under it.
    pub fn requires_readiness_declaration(self) -> bool {
        match self {
            GamePace::Standard => true,
            GamePace::Fast => false,
        }
    }

    /// The wire spelling. Read back by [`GamePace::parse`], so the two
    /// directions are one table rather than a match each way that can drift.
    pub fn wire(self) -> &'static str {
        match self {
            GamePace::Standard => "standard",
            GamePace::Fast => "fast",
        }
    }

    /// A pace off the wire, or `None` for a value this build has not heard of.
    /// `None` is not a third pace — see [`GameDetail::pace`].
    pub fn parse(s: &str) -> Option<GamePace> {
        match s {
            "standard" => Some(GamePace::Standard),
            "fast" => Some(GamePace::Fast),
            _ => None,
        }
    }
}

/// Partial game edit: the six text fields the create form owns, plus pace
/// and the planned window. None means leave alone (absent on the wire, never
/// null).
#[derive(Debug, Clone, Default)]
pub struct GameUpdate {
    pub name: Option<String>,
    pub description: Option<String>,
    pub purpose: Option<String>,
    pub target: Option<String>,
    pub area: Option<String>,
    pub map_tag: Option<String>,
    /// Absent leaves the pace alone. An operator who never touched the combo
    /// has not chosen a pace, and defaulting one here would silently re-pace
    /// a game every time somebody renamed it.
    pub pace: Option<GamePace>,
    pub window: TimeWindow,
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
    /// The Game Master's prose. Sent to EVERYONE entitled to the game, so
    /// unlike `area` there is no withholding rule and an empty string
    /// unambiguously means "not written".
    pub description: String,
    pub purpose: String,
    pub target: String,
    /// Absent means unset OR withheld — the server omits both with the same
    /// shape, so the client cannot tell them apart from the response alone.
    /// `GameDetail::area_is_withheld` gives the honest reading.
    pub area: Option<String>,
    pub map_tag: Option<String>,
    /// Written to be displayed above the map during planning and preparation.
    /// Never withheld; hidden at execution by a rule the CLIENT applies, from
    /// the state it already has. Absent means unset.
    pub overlay_text: Option<String>,
    /// The planned window, both clocks. Incomplete is the ordinary state of a
    /// draft, and it is the reason the execution transition is refused, so it
    /// is carried as read rather than filled in with a default nobody chose.
    pub window: TimeWindow,
    /// The exercise's pace, off the wire.
    ///
    /// `None` is an UNRECOGNISED value, and it means the ceremony applies. An
    /// unknown value is not a pace at all, so there is nothing to infer from
    /// it. What decides the direction is the cost of being wrong. Reading it
    /// as `fast` waives the readiness declaration on the strength of a version
    /// skew, and an exercise that starts while half the exercise side is still
    /// reading the brief is not a mistake you can undo. A gate that turns out
    /// to be inconvenient is only inconvenient.
    pub pace: Option<GamePace>,
    pub time_factor: f64,
    /// What personnel type to enter the room. Null until the game
    /// enters preparation — a planning game has no room to enter —
    /// and sent to every entitled caller, participants included,
    /// because withholding it from the group that must use it would
    /// make the feature impossible.
    pub room_key: Option<String>,
}

impl GameDetail {
    /// Whether an absent `area` means WITHHELD rather than unset.
    ///
    /// The two are the same shape on the wire, so this is a question about
    /// WHO IS ASKING rather than about the payload: a Game Master is never
    /// withheld, so for them absence means unset; a participant receives the
    /// area only from execution onward, so before that its absence IS the
    /// withholding.
    ///
    /// `(is_game_master, state)` rather than a bool because both halves are
    /// needed and computing them at each call site is how the two get mixed
    /// up — reading `state == "execution"` alone would show "withheld" to the
    /// Game Master, who is simply looking at an exercise with no area yet.
    pub fn area_is_withheld(&self, is_game_master: bool) -> bool {
        self.area.is_none() && !is_game_master && self.state != "execution"
    }
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

/// One scenario: a titled beat in a game's book, with ordered steps.
///
/// A game has exactly one book and the book is its ordered scenarios. There
/// is no cross-game catalog and no template, which is why the composer
/// authors rather than selects.
#[derive(Debug, Clone, PartialEq)]
pub struct GameScenario {
    pub id: i64,
    pub game_id: i64,
    /// Zero-based index in the book. Gaps are legal after a delete.
    pub position: i64,
    /// Required by the backend; may not be blank.
    pub title: String,
    /// May be empty — a plan is authored while it is being written.
    pub description: String,
    /// Nested, and the ONLY place steps arrive: there is no step-list route.
    pub steps: Vec<GameScenarioStep>,
}

/// One step: the free text that says what must happen, plus an optional
/// window on the exercise's assumed clock.
///
/// THERE IS NO TITLE. The backend decided the content *is* the label, so a
/// client that shows a separate title field is inventing a field the
/// contract does not have.
#[derive(Debug, Clone, PartialEq)]
pub struct GameScenarioStep {
    pub id: i64,
    pub scenario_id: i64,
    /// Zero-based within the scenario. Gaps are legal after a delete.
    pub position: i64,
    /// May be empty, deliberately: a step whose text is still to come is a
    /// legitimate authoring state, not an error.
    pub content: String,
    /// The window as authored, `HHMM`, four digits. Both ends or neither.
    pub start_hour: Option<String>,
    pub end_hour: Option<String>,
}

impl GameScenarioStep {
    /// The window, if both ends are present.
    ///
    /// One accessor rather than reading the pair at each call site, because a
    /// half-window is unrepresentable on the wire and a caller that checks
    /// one end and uses the other is a caller that will eventually send a
    /// 400.
    pub fn window(&self) -> Option<(&str, &str)> {
        Some((self.start_hour.as_deref()?, self.end_hour.as_deref()?))
    }

    /// The window as one list row, or a dash when there is none.
    pub fn window_label(&self) -> String {
        match self.window() {
            Some((s, e)) => format!("{s}-{e}"),
            None => "-".to_string(),
        }
    }
}

/// Whether `s` is a `HHMM` the backend will accept.
///
/// Strict on purpose, because the backend's own parser is: exactly four
/// characters, digits only, hour at most 23, minute at most 59. So `900`,
/// `2400`, `10:00` and `1060` are all refused, and a client that accepts any
/// of them turns an immediate local correction into a 400 from the field.
///
/// Checked on the way out so the author sees the problem while typing.
pub fn hhmm_ok(s: &str) -> bool {
    if s.len() != 4 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let h = s[0..2].parse::<u32>().unwrap_or(99);
    let m = s[2..4].parse::<u32>().unwrap_or(99);
    h <= 23 && m <= 59
}

/// Whether a pair of `HHMM` strings is a window the backend will accept.
///
/// Both ends or neither, and the end must be after the start. A window that
/// crosses midnight is two steps, by the backend's own rule.
pub fn hhmm_window_ok(start: &str, end: &str) -> bool {
    hhmm_ok(start) && hhmm_ok(end) && start < end
}

/// The date the exercise clock is stamped with, and it is NOT a real date.
///
/// RFC3339 has no time-only form, and `validateTimeBase` compares the assumed
/// clock ONLY with itself. Nothing in the contract compares the two clocks, so
/// the date half of the pair is free and has one job: be the SAME on both
/// ends. One constant does that, and `assumed_end > assumed_start` then holds
/// exactly when `hhmm_window_ok` says the two `HHMM`s run forwards, which is
/// the guarantee the operator was given when they typed them.
///
/// One consequence worth stating plainly. An assumed window that crosses
/// midnight, 2300 to 0100, is REFUSED, because on one date 01:00 is before
/// 23:00. `hhmm_window_ok` already forbids it for scenario steps, and
/// forbidding it here too keeps one rule for the whole console instead of a
/// client that accepts a window the server will reject. An overnight exercise
/// is two steps, which is what the scenario composer has always taught.
const ASSUMED_CLOCK_BASE_DATE: &str = "2000-01-01";

/// The exercise clock's `HHMM` as Minos speaks it. The date is a fixed base
/// because `validateTimeBase` compares the assumed clock only with itself.
///
/// Validated with [`hhmm_ok`] rather than a second, laxer parser, so the two
/// authoring surfaces in this console cannot disagree about what a military
/// time is.
pub fn assumed_hhmm_to_rfc3339(hhmm: &str) -> Option<String> {
    if !hhmm_ok(hhmm) {
        return None;
    }
    Some(format!(
        "{ASSUMED_CLOCK_BASE_DATE}T{}:{}:00Z",
        &hhmm[..2],
        &hhmm[2..]
    ))
}

/// A real-world date plus `HHMM` as Minos speaks it.
///
/// The date is typed, not guessed: the real clock is the one an operator has
/// to look up, and a default date would put the exercise on the wrong day
/// while every local check passed.
pub fn actual_to_rfc3339(date: &str, hhmm: &str) -> Option<String> {
    if !date_ok(date) || !hhmm_ok(hhmm) {
        return None;
    }
    Some(format!("{date}T{}:{}:00Z", &hhmm[..2], &hhmm[2..]))
}

/// Whether `s` is a calendar date the backend will accept: `YYYY-MM-DD`,
/// real day for real month.
///
/// Strict for the same reason [`hhmm_ok`] is. Go's `time.Time` refuses an
/// out-of-range day outright, so `2026-02-30` arrives as a 400 from the field
/// rather than as something the author sees while typing. Leap years are
/// counted because a check that waved 29 February through three years out of
/// four would be worse than no check at all — it would look like it worked.
fn date_ok(s: &str) -> bool {
    if s.len() != 10
        || !s
            .bytes()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
    {
        return false;
    }
    let part = |a: usize, b: usize| s[a..b].parse::<u32>().ok();
    let (Some(y), Some(m), Some(d)) = (part(0, 4), part(5, 7), part(8, 10)) else {
        return false;
    };
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&d)
}

/// What the server says about whether an exercise may start.
///
/// A PROJECTION of the gate, not the client's copy of it. Every value was
/// already readable through a route this client has; what the server publishes
/// here is the verdict, and the list of ingredients still missing in the
/// server's own words.
///
/// The client used to derive all of this from the roster and the placements,
/// and derived it wrong: five conditions, four of them implemented, and a
/// button disabled on a game the server would have accepted.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GameReadinessView {
    /// Whether `preparation` -> `execution` is permitted right now.
    pub can_execute: bool,
    /// Every missing ingredient, as the server words it.
    ///
    /// Empty when `can_execute`. Never rebuilt here: a client that reworded
    /// them would be one more copy of the rule, which is the thing this
    /// endpoint exists to end.
    pub blockers: Vec<String>,
    /// `fast` waives the readiness condition and nothing else.
    pub fast: bool,
}

impl GameReadinessView {
    /// Whether the outstanding list names the readiness condition.
    ///
    /// A question about WORDING, not about the gate — it decides whether the
    /// client may attach the participant names to that one sentence, and
    /// nothing else. Deliberately not a rule: the verdict is `can_execute`.
    pub fn names_readiness(&self) -> bool {
        self.blockers.iter().any(|b| b.contains("not declared themselves ready"))
    }
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
    /// When this person presented the room key, if they have.
    ///
    /// `None` is a real state and not a gap: assigned but not yet in the
    /// room. That distinction is the whole of the top zone's headcount, and
    /// the client used to throw this field away and then had nothing honest
    /// to show for "how many are online".
    pub joined_at: Option<String>,
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

#[cfg(test)]
mod scenario_tests {
    use super::{hhmm_ok, hhmm_window_ok, MinosMaster, GameScenarioStep};

    /// The client's `HHMM` check has to be exactly as strict as the server's
    /// or it defers a local correction to a 400 from the field. The refused
    /// set here is the server's parser's set, not a guess at it.
    #[test]
    fn hhmm_matches_the_servers_parser() {
        for good in ["0000", "0900", "1000", "1030", "2359"] {
            assert!(hhmm_ok(good), "{good} should be accepted");
        }
        for bad in ["", "9", "900", "2400", "10:00", "1060", "9999", "+900", "09 0", "abcd"] {
            assert!(!hhmm_ok(bad), "{bad:?} should be refused");
        }
    }

    /// A window is both ends or neither, and it runs forwards. A window that
    /// crosses midnight is two steps, which is the server's rule and not one
    /// this client gets to relax.
    #[test]
    fn a_window_needs_both_ends_and_must_run_forwards() {
        assert!(hhmm_window_ok("1000", "1030"));
        assert!(!hhmm_window_ok("1030", "1000"), "an end before its start");
        assert!(!hhmm_window_ok("1000", "1000"), "a zero-length window");
        assert!(!hhmm_window_ok("2300", "0100"), "crossing midnight");
        assert!(
            !hhmm_window_ok("0900", "1000 "),
            "a trailing space is not trimmed away silently"
        );
    }

    /// A half-window is unrepresentable on the wire, so reading one is not
    /// possible. Reading the pair through one accessor is what stops a
    /// caller checking one end and using the other.
    #[test]
    fn a_step_window_is_all_or_nothing() {
        let both = GameScenarioStep {
            id: 1,
            scenario_id: 1,
            position: 0,
            content: "sweep".into(),
            start_hour: Some("1000".into()),
            end_hour: Some("1030".into()),
        };
        assert_eq!(both.window(), Some(("1000", "1030")));
        assert_eq!(both.window_label(), "1000-1030");

        for (s, e) in [(Some("1000"), None), (None, Some("1030")), (None, None)] {
            let step = GameScenarioStep {
                start_hour: s.map(String::from),
                end_hour: e.map(String::from),
                ..both.clone()
            };
            assert_eq!(step.window(), None, "half a window must read as none");
            assert_eq!(step.window_label(), "-");
        }
    }

    /// Steps arrive nested inside the scenario and nowhere else, so the
    /// parse is the only place that has to get it right.
    #[test]
    fn a_scenario_parses_with_its_nested_steps() {
        let v: serde_json::Value = serde_json::json!({
            "id": 12,
            "id_game": 3,
            "position": 0,
            "title": "First light",
            "description": "",
            "steps": [
                { "id": 1, "id_scenario": 12, "position": 0, "content": "sweep north", "start_hour": "1000", "end_hour": "1030" },
                { "id": 2, "id_scenario": 12, "position": 1, "content": "", "start_hour": null, "end_hour": null }
            ]
        });
        let s = MinosMaster::parse_scenario(&v, 3).expect("a scenario with an id decodes");
        assert_eq!(s.id, 12);
        assert_eq!(s.title, "First light");
        assert_eq!(s.steps.len(), 2);
        assert_eq!(s.steps[0].window_label(), "1000-1030");
        assert_eq!(
            s.steps[1].content, "",
            "an empty step is a legitimate authoring state, not dropped"
        );
        assert_eq!(s.steps[1].window_label(), "-");
    }

    /// A scenario with no id is a decode error rather than a dropped row.
    /// The composer's whole job is editing this scenario; dropping it would
    /// leave the author staring at an empty list with no idea why.
    #[test]
    fn a_scenario_without_an_id_is_refused() {
        let v = serde_json::json!({ "id_game": 3, "title": "nameless" });
        assert!(MinosMaster::parse_scenario(&v, 3).is_err());
    }

    /// A step with no id is dropped rather than rendered: the composer's
    /// delete and edit verbs address steps by id, and a row that cannot be
    /// addressed is a row whose buttons all fail.
    ///
    /// The step that IS addressable carries NO `id_scenario` of its own, and
    /// is kept anyway, because the parent supplies it. Reading it from the
    /// child instead dropped this row too, which is what the first version
    /// of this test found.
    #[test]
    fn a_step_takes_its_scenario_from_the_parent() {
        let v: serde_json::Value = serde_json::json!({
            "id": 5, "id_game": 3, "position": 0, "title": "t",
            "steps": [
                { "position": 0, "content": "no id" },
                { "id": 2, "position": 1, "content": "has an id" }
            ]
        });
        let s = MinosMaster::parse_scenario(&v, 3).expect("the scenario itself decodes");
        assert_eq!(s.steps.len(), 1);
        assert_eq!(s.steps[0].content, "has an id");
    }
}

#[cfg(test)]
mod game_prose_tests {
    use super::{GameDetail, MinosMaster, TimeWindow};

    fn detail(v: serde_json::Value) -> GameDetail {
        MinosMaster::parse_game_detail(&v, 7)
    }

    /// The bug this fixes: `description` was sent by the create form and
    /// dropped by the parser, so it was write-only. Every prose field is
    /// asserted together, because the parser dropped six and fixing one would
    /// leave five more of the same shape.
    #[test]
    fn the_prose_fields_survive_the_read() {
        let d = detail(serde_json::json!({
            "id": 7,
            "name": "RIMPAC",
            "mode": "maneuver",
            "state": "planning",
            "description": "two-day exercise",
            "purpose": "train the staff",
            "target": "the exercise area"
        }));
        assert_eq!(d.description, "two-day exercise");
        assert_eq!(d.purpose, "train the staff");
        assert_eq!(d.target, "the exercise area");
    }

    /// Absent prose means "not written", which is an empty string rather than
    /// an absence — these two fields are always sent, so there is no
    /// withholding rule to confuse the two readings.
    #[test]
    fn unwritten_prose_is_empty_not_absent() {
        let d = detail(serde_json::json!({
            "id": 7, "name": "n", "mode": "maneuver", "state": "planning"
        }));
        assert_eq!(d.description, "");
        assert_eq!(d.purpose, "");
        assert_eq!(d.target, "");
    }

    /// The three-state fields omit BOTH "unset" and "withheld" the same way,
    /// so the read has to be an absence — and an empty string must not become
    /// a value, because "the area is the empty string" is not a thing.
    #[test]
    fn a_three_state_field_is_absent_until_it_has_a_value() {
        let d = detail(serde_json::json!({
            "id": 7, "name": "n", "mode": "maneuver", "state": "planning",
            "area": "Strait of Malacca",
            "map_tag": "",
            "overlay_text": "D+1"
        }));
        assert_eq!(d.area.as_deref(), Some("Strait of Malacca"));
        assert_eq!(d.map_tag, None, "an empty map tag is unset, not set-to-empty");
        assert_eq!(d.overlay_text.as_deref(), Some("D+1"));
    }

    /// The area's two absences are the same shape on the wire, so the
    /// difference is WHO IS ASKING. A Game Master is never withheld, so for
    /// them absence means unset — reading the state alone would tell the
    /// person who is editing the plan that their own area is hidden.
    #[test]
    fn an_absent_area_is_withheld_only_from_a_participant_before_execution() {
        let planning = detail(serde_json::json!({
            "id": 7, "name": "n", "mode": "maneuver", "state": "planning"
        }));
        assert!(
            !planning.area_is_withheld(true),
            "a Game Master looking at an unset area is not being withheld"
        );
        assert!(
            planning.area_is_withheld(false),
            "a participant before execution is being withheld"
        );

        let execution = detail(serde_json::json!({
            "id": 7, "name": "n", "mode": "maneuver", "state": "execution"
        }));
        assert!(
            !execution.area_is_withheld(false),
            "execution is when the area stops being withheld"
        );

        // A SET area is never withheld, whoever is asking.
        let set = detail(serde_json::json!({
            "id": 7, "name": "n", "mode": "maneuver", "state": "planning",
            "area": "somewhere"
        }));
        assert!(!set.area_is_withheld(false));
    }

    /// The ends ride the detail too, and comparing them with the realised
    /// finish is the whole point of closure — so the read has to carry all
    /// four, not just the two starts. They arrive as one `TimeWindow` rather
    /// than four loose fields because the gate asks one question of them and
    /// a question about half a window is not one anybody asked.
    #[test]
    fn the_ends_ride_alongside_the_starts() {
        let d = detail(serde_json::json!({
            "id": 7, "name": "n", "mode": "maneuver", "state": "execution",
            "actual_start": "2026-10-01T08:00:00Z",
            "assumed_start": "2026-10-01T0600",
            "actual_end": "2026-10-02T16:30:00Z",
            "assumed_end": "2026-10-02T1630"
        }));
        assert_eq!(
            d.window,
            TimeWindow {
                actual_start: Some("2026-10-01T08:00:00Z".into()),
                actual_end: Some("2026-10-02T16:30:00Z".into()),
                assumed_start: Some("2026-10-01T0600".into()),
                assumed_end: Some("2026-10-02T1630".into()),
            }
        );
    }

    /// A game with no planned window is the NORMAL state of a draft, and the
    /// client cannot invent the missing end — the backend supplies no default
    /// because the window is the Game Master's declaration of when the
    /// exercise finishes. So absence reads as an all-`None` window, rather
    /// than as an error or a guessed finish.
    ///
    /// No assertion about completeness, because the client no longer decides
    /// what complete means: `GET /games/{id}/readiness` answers that, and a
    /// local `is_complete` is the kind of second opinion that drifts.
    #[test]
    fn a_detail_with_no_window_reads_as_an_empty_one() {
        let d = detail(serde_json::json!({
            "id": 7, "name": "n", "mode": "maneuver", "state": "planning"
        }));
        assert_eq!(d.window, TimeWindow::default());
        assert!(d.window.is_empty());
    }
}

#[cfg(test)]
mod window_authoring_tests {
    use super::{
        GamePace, GameUpdate, MinosMaster, TimeWindow, actual_to_rfc3339,
        assumed_hhmm_to_rfc3339,
    };

    /// The exercise clock has no date to speak of, so the conversion supplies
    /// a base — and it must be the SAME base on both ends, because that is
    /// the only thing making `assumed_end > assumed_start` hold. One constant
    /// is the whole guarantee, which is why these two are asserted together.
    #[test]
    fn the_assumed_clock_uses_one_base_date_on_both_ends() {
        let start = assumed_hhmm_to_rfc3339("0600").expect("0600 is a military time");
        let end = assumed_hhmm_to_rfc3339("1630").expect("1630 is a military time");
        assert_eq!(start, "2000-01-01T06:00:00Z");
        assert_eq!(end, "2000-01-01T16:30:00Z");
        assert_eq!(
            start[..10],
            end[..10],
            "the base date has to match or the ordering the author was promised is a fiction"
        );
        assert!(
            end > start,
            "and the ordering survives the trip through RFC3339"
        );
    }

    /// What `hhmm_ok` refuses, this refuses — the two are one rule, so a
    /// value the scenario composer will not send is not one the exercise form
    /// sends either.
    #[test]
    fn the_assumed_clock_reuses_the_scenarios_military_time_rule() {
        for bad in ["", "900", "2400", "10:00", "1060"] {
            assert_eq!(
                assumed_hhmm_to_rfc3339(bad),
                None,
                "{bad:?} is not a military time"
            );
        }
    }

    /// The real window is a date and two times, and BOTH halves are checked:
    /// a well-formed time on a nonsense date is still a request the field
    /// rejects, which is the failure this check exists to catch.
    #[test]
    fn the_real_window_refuses_a_bad_date_or_a_bad_time() {
        assert_eq!(
            actual_to_rfc3339("2026-11-01", "0800").as_deref(),
            Some("2026-11-01T08:00:00Z")
        );
        for bad_date in [
            "",
            "2026-11",
            "01/11/2026",
            "2026-13-01",
            "2026-00-10",
            "2026-02-30",
            "1900-02-29",
            "2026-11-31",
        ] {
            assert_eq!(
                actual_to_rfc3339(bad_date, "0800"),
                None,
                "{bad_date:?} is not a calendar date"
            );
        }
        // And the leap rule is real in both directions, because a check that
        // only ever says no is indistinguishable from no check.
        assert!(actual_to_rfc3339("2024-02-29", "0800").is_some());
        assert!(actual_to_rfc3339("2000-02-29", "0800").is_some());
        assert!(actual_to_rfc3339("2100-02-29", "0800").is_none());

        for bad_time in ["", "800", "2400", "08:00"] {
            assert_eq!(
                actual_to_rfc3339("2026-11-01", bad_time),
                None,
                "{bad_time:?} is not a military time"
            );
        }
    }

    /// Both wire values parse, and nothing else does. The refusal is the point
    /// of the test: an unrecognised pace is the case that decides whether the
    /// readiness declaration applies, so it has to be a distinct answer rather
    /// than a silently mapped default.
    #[test]
    fn a_pace_parses_the_two_wire_values_and_nothing_else() {
        assert_eq!(GamePace::parse("standard"), Some(GamePace::Standard));
        assert_eq!(GamePace::parse("fast"), Some(GamePace::Fast));
        for bad in ["", "FAST", "Standard", "quick", "standard ", "1"] {
            assert_eq!(GamePace::parse(bad), None, "{bad:?} is not a pace");
        }
    }

    /// The direction that matters: an unknown pace must land on the value
    /// that REQUIRES the declaration. Guessing the other way would waive a
    /// ceremony gate on a version skew, and the failure is an exercise
    /// starting while half the exercise side is still reading the brief.
    #[test]
    fn an_unknown_pace_fails_closed_onto_the_ceremony() {
        let unknown = GamePace::parse("turbo");
        assert!(unknown.is_none(), "an unknown pace is not a pace at all");
        assert!(
            GamePace::Standard.requires_readiness_declaration(),
            "standard is the ceremony-applying value, so an unknown one falls here"
        );
        assert!(!GamePace::Fast.requires_readiness_declaration());
    }

    /// Wire spelling round-trips through the parser, because the two are the
    /// only place the vocabulary lives. A `wire` that drifts from `parse` is a
    /// write the server rejects with a 422.
    #[test]
    fn a_written_pace_reads_back_as_itself() {
        for p in [GamePace::Standard, GamePace::Fast] {
            assert_eq!(GamePace::parse(p.wire()), Some(p));
        }
    }

    /// The roster is wrapped on the wire, and the wrapper is the whole bug.
    ///
    /// `as_array()` on `{"participants": [...]}` is None, so a seated game
    /// rendered as an empty one and nothing said so. Asserted against the
    /// server's real envelope AND the bare array, because the write paths
    /// (`add_participant` and friends) answer with the same wrapper and both
    /// shapes have been seen on this client.
    #[test]
    fn a_wrapped_roster_reads_as_its_rows() {
        let row = serde_json::json!({
            "id_user": 4,
            "user_name": "Super User",
            "id_game_role": 18,
            "role_name": "Game Master",
            "is_judge_side": false,
            "is_ready": false,
            "joined_at": "2026-10-03T23:20:00+07:00"
        });
        let wrapped = serde_json::json!({ "participants": [row.clone()] });
        let bare = serde_json::json!([row]);

        for shape in [wrapped, bare] {
            let roster = MinosMaster::parse_roster(&shape);
            assert_eq!(roster.len(), 1, "shape {shape}");
            assert_eq!(roster[0].user_id, 4);
            assert_eq!(roster[0].role_name, "Game Master");
            assert!(roster[0].joined_at.is_some(), "a joined_at is a fact");
        }
    }

    /// An envelope that is neither shape must not invent rows, and must not
    /// panic on a missing key either.
    #[test]
    fn an_unrecognised_roster_envelope_is_empty_rather_than_wrong() {
        assert!(MinosMaster::parse_roster(&serde_json::json!({})).is_empty());
        assert!(MinosMaster::parse_roster(&serde_json::json!(null)).is_empty());
    }

}
