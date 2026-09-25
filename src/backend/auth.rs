//! Auth against Minos (backend split, step 2; typed errors).
//!
//! - [`TokenPair`]: short-lived access token plus the desktop refresh path.
//! - [`MinosAuth`]: identifier login, rotation refresh, password-change
//!   gate probe. Blocking client like the other backends.
//! - `keyring_*`: refresh-token store, one OS keyring entry per identifier.
//!
//! All failures are [`BackendError`] (matched by the UI, never
//! substring-matched). The envelope unwrap lives in the parent until it
//! moves into `error` with the next layers.

use std::path::Path;

use super::{BackendError, unwrap_envelope};

/// Minos token pair (docs/minos-api.yaml): short-lived access token plus
/// the desktop refresh path. `refresh_token` is present only when the
/// login asked for it (`return_refresh_token`), which native clients must.
#[derive(Debug, Clone)]
pub struct TokenPair {
    pub access_token: String,
    /// Always `Bearer` by contract — enforced, not defaulted (M2).
    pub token_type: String,
    pub expires_in: u64,
    pub refresh_token: Option<String>,
    /// Server courtesy flag (M2): route straight to change-password
    /// instead of discovering it from a refused request.
    pub must_change_password: bool,
}

/// The authenticated account's stable identity and application-role ids.
/// The role ids let the desktop client keep session-management controls
/// hidden for participants without guessing from display names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedUser {
    pub id: i64,
    pub app_role_ids: Vec<i64>,
}

/// Strict pair decode (M2): the access token must be non-empty, the
/// type present and Bearer, the lifetime present. Missing fields are a
/// decode error — never an empty token that authenticates nothing or
/// a zero lifetime that refresh-loops. `refresh_token` stays optional
/// (absent unless requested); `must_change_password` defaults false
/// when absent (the flag's absence is its negative).
fn parse_pair(data: &serde_json::Value) -> Result<TokenPair, BackendError> {
    let access = data["access_token"].as_str().unwrap_or("");
    if access.is_empty() {
        return Err(BackendError::Other("auth answer without an access token".to_string()));
    }
    let scheme = data["token_type"].as_str().unwrap_or("");
    if scheme != "Bearer" {
        return Err(BackendError::Other(format!(
            "auth answer with token_type {scheme:?}, want \"Bearer\""
        )));
    }
    let expires = data["expires_in"]
        .as_i64()
        .and_then(|e| u64::try_from(e).ok())
        .ok_or_else(|| BackendError::Other("auth answer without expires_in".to_string()))?;
    Ok(TokenPair {
        access_token: access.to_string(),
        token_type: scheme.to_string(),
        expires_in: expires,
        refresh_token: data["refresh_token"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string()),
        must_change_password: data["must_change_password"].as_bool().unwrap_or(false),
    })
}

/// Minos auth (auth grill resolution): identifier login, rotation refresh,
/// password-change gate probe. Blocking client like the other backends.
/// Tokens live with the caller — access in memory, refresh in the keyring.
pub struct MinosAuth {
    base_url: String,
    client: reqwest::blocking::Client,
}

impl MinosAuth {
    pub fn new(base_url: &str) -> Result<Self, BackendError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client,
        })
    }

    /// Sign in. Desktop path: `return_refresh_token` true (no cookie jar).
    /// Unknown user and wrong password are an identical 401 by design —
    /// the caller matches `Unauthorized`, never the message text.
    pub fn login(&self, identifier: &str, password: &str) -> Result<TokenPair, BackendError> {
        let data = unwrap_envelope(
            self.client
                .post(&format!("{}/auth/login", self.base_url))
                .json(&serde_json::json!({
                    "identifier": identifier,
                    "password": password,
                    "return_refresh_token": true,
                }))
                .send()?,
        )?;
        parse_pair(&data)
    }

    /// Rotate: presents the refresh token, returns a new pair. The old
    /// refresh token is invalidated server-side (replay is rejected).
    pub fn refresh(&self, refresh_token: &str) -> Result<TokenPair, BackendError> {
        let data = unwrap_envelope(
            self.client
                .post(&format!("{}/auth/refresh", self.base_url))
                .json(&serde_json::json!({ "refresh_token": refresh_token }))
                .send()?,
        )?;
        parse_pair(&data)
    }

    /// Change the account password (the must_change_password gate). 204 on
    /// success; every other refresh token for the account is invalidated,
    /// so the caller re-logins afterwards. Passwords go byte for byte.
    pub fn change_password(
        &self,
        access_token: &str,
        current_password: &str,
        new_password: &str,
    ) -> Result<(), BackendError> {
        let resp = self
            .client
            .put(&format!("{}/users/me/password", self.base_url))
            .bearer_auth(access_token)
            .json(&serde_json::json!({
                "current_password": current_password,
                "new_password": new_password,
            }))
            .send()?;
        let status = resp.status();
        if !status.is_success() {
            // M1: refusal bodies carry field detail — surface it.
            let body: serde_json::Value = resp.json().unwrap_or(serde_json::Value::Null);
            return Err(BackendError::http_error(status.as_u16(), &body));
        }
        Ok(())
    }

    /// End the refresh session (H9): revokes the presented refresh
    /// token family server-side. Answers 204 whatever happens —
    /// including no session at all, so it cannot probe tokens — which
    /// means no envelope to unwrap: status-first, no body parse (a 204
    /// carries none, and parsing it would fail the happy path). Best
    /// effort by design: local sign-out proceeds regardless; only the
    /// access token's remaining lifetime is beyond this call (it is
    /// stateless by backend design — see B2 on the map).
    pub fn logout(&self, refresh_token: Option<&str>) -> Result<(), BackendError> {
        let mut body = serde_json::json!({});
        if let Some(rt) = refresh_token {
            body["refresh_token"] = serde_json::Value::String(rt.to_string());
        }
        let resp = self
            .client
            .post(&format!("{}/auth/logout", self.base_url))
            .json(&body)
            .send()?;
        let status = resp.status();
        if !status.is_success() {
            // Logout answers 204 whatever happens, so a failure here is
            // transport-level — still run it through the detail reader
            // in case the backend ever explains itself.
            let body: serde_json::Value = resp.json().unwrap_or(serde_json::Value::Null);
            return Err(BackendError::http_error(status.as_u16(), &body));
        }
        Ok(())
    }

    /// Gate probe: who am I with this token. Ok carries the caller's
    /// user id and application-role ids (C2: readiness matching needs
    /// the former; the setup screen needs the latter to separate an
    /// organizer from a room-key participant). Ok means the token works
    /// and no gate stands in the way; `Forbidden` with valid credentials
    /// means the account is not Active (or must_change_password still
    /// shuts doors) — matched as a variant by the login island. A
    /// missing id is a decode error, never zero.
    pub fn me(&self, access_token: &str) -> Result<AuthenticatedUser, BackendError> {
        // Status-first like the old probe (never unwrap-then-check):
        // the gate match on `Forbidden` must survive even a body that
        // is not the envelope shape.
        let resp = self
            .client
            .get(&format!("{}/users/me", self.base_url))
            .bearer_auth(access_token)
            .send()?;
        let status = resp.status();
        let body: serde_json::Value =
            resp.json().unwrap_or(serde_json::Value::Null);
        if !status.is_success() {
            return Err(BackendError::http_error(status.as_u16(), &body));
        }
        let data = &body["data"];
        let id = data["id"]
            .as_i64()
            .ok_or_else(|| BackendError::Other("me answer without an id".to_string()))?;
        let app_role_ids = data["roles"]
            .as_array()
            .map(|roles| roles.iter().filter_map(|role| role["id"].as_i64()).collect())
            .unwrap_or_default();
        Ok(AuthenticatedUser { id, app_role_ids })
    }
}

/// Refresh-token store (keyring resolution): OS keyring entry per
/// identifier, single writer on the UI thread. `None` on load means no
/// entry (fresh login); any other store failure degrades to memory with
/// a visible degraded status — never silently.
const KEYRING_SERVICE: &str = "tfg-minos";

/// Save (overwrite) the refresh token for this identifier.
pub fn keyring_save(user: &str, token: &str) -> Result<(), BackendError> {
    keyring::Entry::new(KEYRING_SERVICE, user)
        .map_err(|e| BackendError::Keyring(e.to_string()))?
        .set_password(token)
        .map_err(|e| BackendError::Keyring(e.to_string()))
}

/// Load the stored refresh token, if any.
pub fn keyring_load(user: &str) -> Result<Option<String>, BackendError> {
    let entry =
        keyring::Entry::new(KEYRING_SERVICE, user).map_err(|e| BackendError::Keyring(e.to_string()))?;
    match entry.get_password() {
        Ok(pw) => Ok(Some(pw)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(BackendError::Keyring(e.to_string())),
    }
}

/// Forget the stored refresh token (sign out). Missing entry is fine.
pub fn keyring_clear(user: &str) -> Result<(), BackendError> {
    let entry =
        keyring::Entry::new(KEYRING_SERVICE, user).map_err(|e| BackendError::Keyring(e.to_string()))?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(BackendError::Keyring(e.to_string())),
    }
}

/// Remember who signed in (best effort — a missed write only costs
/// the next launch a login form).
pub fn last_user_save(path: &Path, user: &str) {
    let _ = std::fs::write(path, user.trim());
}

/// Who signed in last, if the record survived.
pub fn last_user_load(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Forget the last identifier (sign out).
pub fn last_user_clear(path: &Path) {
    let _ = std::fs::remove_file(path);
}
