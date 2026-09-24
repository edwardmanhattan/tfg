//! Typed backend errors (error-handling pass, step 1).
//!
//! Failures used to be `String`s, matched by substring (`e.contains("403")`)
//! — brittle, because any reworded message silently breaks the match.
//! Now each failure is a variant the UI matches on and the compiler checks:
//! `Forbidden` from the gate probe means must-change-password, the same
//! `Forbidden` from the user directory means roster-fallback.

use thiserror::Error;

/// Every way a backend call can fail. Match on these; never substring-match
/// the rendered message.
#[derive(Debug, Error)]
pub enum BackendError {
    /// The server was unreachable or the connection broke mid-call.
    #[error("unreachable: {0}")]
    Transport(#[from] reqwest::Error),

    /// The server answered, but the body wasn't the shape we parse.
    #[error("bad data: {0}")]
    Decode(#[from] serde_json::Error),

    /// 401: no token, or an expired/invalid one. Sign in again.
    #[error("unauthorized: {message}")]
    Unauthorized { message: String, detail: Option<String> },

    /// 403: authenticated but refused. Meaning depends on the call site:
    /// the gate probe reads it as must-change-password / not Active,
    /// the user directory reads it as roster-fallback.
    #[error("forbidden: {message}")]
    Forbidden { message: String, detail: Option<String> },

    /// Any other non-2xx, with its status attached. `detail` is the
    /// server's `data.errors` flattened (M1) — e.g. the execution gate
    /// naming every missing ingredient at once — folded into the
    /// message so every status line shows it, and kept structured for
    /// callers that render it apart.
    #[error("HTTP {status}: {message}")]
    Api { status: u16, message: String, detail: Option<String> },

    /// The hull has no published specification version — the API invents
    /// no speed, neither do we. Matched, never substring-searched.
    #[error("unit {unit_id}: no published specification")]
    NoSpec { unit_id: i64 },

    /// The OS keyring failed (entry present, operation refused).
    #[error("keyring: {0}")]
    Keyring(String),

    /// Anything else with nothing to match on. Shrinks with each pass —
    /// a new variant beats a new `Other` every time.
    #[error("{0}")]
    Other(String),
}

impl BackendError {
    /// Classify an HTTP failure by status code with the server's
    /// `data.errors` attached (M1): status and top-level message stay,
    /// the detail rides both structured and folded into the message
    /// every status line renders.
    pub(crate) fn status_detailed(
        status: u16,
        message: String,
        detail: Option<String>,
    ) -> Self {
        let message = match &detail {
            Some(d) if !d.is_empty() => format!("{message} — {d}"),
            _ => message,
        };
        match status {
            401 => Self::Unauthorized { message, detail },
            403 => Self::Forbidden { message, detail },
            _ => Self::Api { status, message, detail },
        }
    }

    /// Build the error for a non-2xx response from its body: top-level
    /// message plus flattened `data.errors` detail.
    pub(crate) fn http_error(status: u16, body: &serde_json::Value) -> Self {
        let message = body["message"]
            .as_str()
            .unwrap_or("request failed")
            .to_string();
        Self::status_detailed(status, message, error_detail(body))
    }

    /// The structured detail, for callers that render it apart from
    /// the message it is already folded into.
    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::Unauthorized { detail, .. }
            | Self::Forbidden { detail, .. }
            | Self::Api { detail, .. } => detail.as_deref(),
            _ => None,
        }
    }
}

/// Flatten Minos `data.errors` (a field→message map, `_request` bare)
/// to one display string. Unknown shapes yield None rather than a
/// fabricated detail — no errors key means nothing to add.
pub(crate) fn error_detail(body: &serde_json::Value) -> Option<String> {
    let obj = body.get("data")?.get("errors")?.as_object()?;
    let mut parts = Vec::new();
    // Sorted keys: deterministic text for tests and logs.
    let mut keys: Vec<&String> = obj.keys().collect();
    keys.sort();
    for k in keys {
        let msgs: Vec<String> = match &obj[k] {
            serde_json::Value::String(s) => vec![s.clone()],
            serde_json::Value::Array(arr) => arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
            _ => Vec::new(),
        };
        for msg in msgs {
            if k == "_request" {
                parts.push(msg);
            } else {
                parts.push(format!("{k}: {msg}"));
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("; "))
    }
}

/// Bridge while layers migrate: untouched `Result<T, String>` boundaries
/// keep compiling through `?`. New code matches on variants instead.
impl From<BackendError> for String {
    fn from(e: BackendError) -> String {
        e.to_string()
    }
}
