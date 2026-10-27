//! Device tokens, the scopes they carry, and the one-time pairing code that
//! mints them.
//!
//! The operator's own credential arrives in `TFG_CONTROL_TOKEN` and is never
//! written anywhere. Everything else is a *device*: a phone, a laptop running
//! `tfgctl`, a second screen. A device is paired once, gets its own token, and
//! can be revoked without disturbing the others — which is the whole reason
//! this file exists rather than a single shared secret.
//!
//! Three rules, and they are the reason the file is not smaller:
//!
//! - **The token is stored as a hash.** A copy of the devices file is not a
//!   set of credentials. SHA-256 is enough here because the token is 256 bits
//!   of randomness, not a password: there is nothing to guess.
//! - **A pairing code is short, single-use, and short-lived.** Eight Crockford
//!   characters is forty bits, which is only safe because it dies in two
//!   minutes and after five wrong guesses.
//! - **Revocation is persistence.** A revoked device is deleted from the file,
//!   so a restart cannot resurrect it.

use super::auth::{MIN_TOKEN_LEN, constant_time_eq};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// What a credential is allowed to do.
///
/// The order matters: a control credential also lets you watch, because a
/// client that can move the map and cannot see it is a client that cannot
/// tell whether it worked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Read the shared state and watch the event stream.
    Observe,
    /// Send camera commands.
    ControlView,
    /// Pair devices, list them, revoke them. The operator's own token has it;
    /// a paired device should not.
    Admin,
}

impl Scope {
    /// Every scope this one covers.
    pub fn implies(self, needed: Scope) -> bool {
        match self {
            Scope::Admin => true,
            Scope::ControlView => matches!(needed, Scope::Observe | Scope::ControlView),
            Scope::Observe => needed == Scope::Observe,
        }
    }

    /// The wire name, for error messages.
    pub fn name(self) -> &'static str {
        match self {
            Scope::Observe => "observe",
            Scope::ControlView => "control_view",
            Scope::Admin => "admin",
        }
    }
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Who is asking, and what they may do. Resolved once per request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Grant {
    /// `operator` for the `TFG_CONTROL_TOKEN` credential, `dev-…` otherwise.
    pub id: String,
    pub label: String,
    pub scopes: Vec<Scope>,
}

impl Grant {
    /// The operator's own credential: everything, including pairing.
    pub fn operator() -> Self {
        Self {
            id: "operator".to_string(),
            label: "operator (TFG_CONTROL_TOKEN)".to_string(),
            scopes: vec![Scope::Observe, Scope::ControlView, Scope::Admin],
        }
    }

    pub fn allows(&self, needed: Scope) -> bool {
        self.scopes.iter().any(|scope| scope.implies(needed))
    }
}

/// A paired device, as the devices file holds it.
///
/// `token_hash` never leaves this module: [`DeviceView`] is what the API and
/// the CLI see.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub label: String,
    pub scopes: Vec<Scope>,
    /// Unix seconds.
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<i64>,
    /// Hex SHA-256 of the bearer token. Never serialised to a client.
    pub token_hash: String,
}

/// A device as `GET /v1/devices` reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceView {
    pub id: String,
    pub label: String,
    pub scopes: Vec<Scope>,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<i64>,
}

impl From<&Device> for DeviceView {
    fn from(device: &Device) -> Self {
        Self {
            id: device.id.clone(),
            label: device.label.clone(),
            scopes: device.scopes.clone(),
            created_at: device.created_at,
            last_seen: device.last_seen,
        }
    }
}

/// The on-disk shape. Versioned so a later change can migrate rather than
/// guess.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct DeviceFile {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    devices: Vec<Device>,
}

fn default_version() -> u32 {
    1
}

/// An open pairing window: the code the operator is reading out loud.
#[derive(Debug, Clone)]
struct Pairing {
    code: String,
    expires_at: Instant,
    attempts: u32,
    scopes: Vec<Scope>,
}

/// What the operator / CLI is shown when a window opens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairingOffer {
    /// The short code, grouped for reading aloud: `K7QM-3XPD`.
    pub code: String,
    /// How long the code lives, in seconds.
    pub expires_in: u64,
    /// The URI a QR code should carry.
    pub uri: String,
    /// The same thing as JSON, for a client that would rather parse it.
    pub payload: String,
    pub scopes: Vec<Scope>,
}

/// The answer to a successful pairing: the device and its one and only
/// plaintext token.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Paired {
    pub device: DeviceView,
    pub token: String,
}

/// Why a pairing attempt failed. Each one is a distinct HTTP status, because a
/// client needs to tell "wrong code, try again" from "the window closed".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairError {
    /// No window is open at all.
    NoWindow,
    /// The window is open and the code is wrong.
    BadCode,
    /// The window's time ran out.
    Expired,
    /// Five wrong guesses: the window is closed.
    TooManyAttempts,
    /// The machine could not produce a token. Refused rather than issued from
    /// a weak source; the window is left open so the operator can try again.
    Unavailable,
}

impl PairError {
    pub fn code(self) -> &'static str {
        match self {
            PairError::NoWindow => "no_pairing_window",
            PairError::BadCode => "invalid_pairing_code",
            PairError::Expired => "pairing_expired",
            PairError::TooManyAttempts => "too_many_attempts",
            PairError::Unavailable => "server_error",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            PairError::NoWindow => "no pairing window is open on this desktop",
            PairError::BadCode => "that pairing code is not the one on screen",
            PairError::Expired => "the pairing code expired; open a new one",
            PairError::TooManyAttempts => "too many wrong codes; the pairing window is closed",
            PairError::Unavailable => {
                "this machine could not produce a device token (/dev/urandom)"
            }
        }
    }

    pub fn status(self) -> u16 {
        match self {
            PairError::NoWindow => 404,
            PairError::BadCode => 401,
            PairError::Expired => 410,
            PairError::TooManyAttempts => 429,
            PairError::Unavailable => 500,
        }
    }
}

/// How long a pairing code lives. Two minutes is long enough to read eight
/// characters off a screen and type them, and short enough that a photograph
/// of the screen is not a lasting credential.
pub const PAIRING_TTL: Duration = Duration::from_secs(120);

/// Wrong guesses before the window closes. A forty-bit code cannot be guessed
/// at this rate; the limit is there so a leaked screen recording is useless
/// after five tries.
pub const PAIRING_ATTEMPTS: u32 = 5;

/// How often a `last_seen` bump is written back to disk. Every request would
/// be a disk write per request; never is a file that lies about when a device
/// was last around.
const LAST_SEEN_FLUSH: Duration = Duration::from_secs(60);

struct Inner {
    file: DeviceFile,
    pairing: Option<Pairing>,
    /// When `last_seen` was last persisted, and whether it is dirty.
    flushed_at: Instant,
    dirty: bool,
}

/// The device table and the open pairing window, behind one lock.
pub struct Devices {
    path: Option<PathBuf>,
    inner: Mutex<Inner>,
}

impl Devices {
    /// Open the table at `path`. `None` keeps it in memory, which is what the
    /// tests and a `tfg-server --ephemeral` run want.
    ///
    /// A file that exists but does not parse is an ERROR rather than an empty
    /// table: starting empty would silently un-pair every device, and the
    /// operator would learn about it one refused request at a time.
    pub fn open(path: Option<PathBuf>) -> Result<Self, String> {
        let file = match &path {
            Some(path) => match std::fs::read_to_string(path) {
                Ok(text) if text.trim().is_empty() => DeviceFile {
                    version: 1,
                    devices: Vec::new(),
                },
                Ok(text) => serde_json::from_str::<DeviceFile>(&text).map_err(|error| {
                    format!("{} is not a devices file: {error}", path.display())
                })?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => DeviceFile {
                    version: 1,
                    devices: Vec::new(),
                },
                Err(error) => {
                    return Err(format!("reading {}: {error}", path.display()));
                }
            },
            None => DeviceFile {
                version: 1,
                devices: Vec::new(),
            },
        };
        Ok(Self {
            path,
            inner: Mutex::new(Inner {
                file,
                pairing: None,
                flushed_at: Instant::now(),
                dirty: false,
            }),
        })
    }

    /// The default location: `$XDG_DATA_HOME/tfg/control-devices.json`, or
    /// `~/.local/share/tfg/control-devices.json`.
    pub fn default_path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })?;
        Some(base.join("tfg").join("control-devices.json"))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Resolve a bearer token to a grant, and remember that the device was
    /// here. The operator's token is checked by [`Credentials`], not here.
    pub fn authorise(&self, token: &str) -> Option<Grant> {
        if token.len() < MIN_TOKEN_LEN {
            return None;
        }
        let hash = hash_token(token);
        let now = unix_now();
        {
            let mut inner = self.lock();
            let (grant, refreshed) = {
                let device = inner.file.devices.iter_mut().find(|device| {
                    constant_time_eq(device.token_hash.as_bytes(), hash.as_bytes())
                })?;
                let refreshed = device.last_seen.is_none_or(|seen| now - seen >= 60);
                if refreshed {
                    device.last_seen = Some(now);
                }
                let grant = Grant {
                    id: device.id.clone(),
                    label: device.label.clone(),
                    scopes: device.scopes.clone(),
                };
                (grant, refreshed)
            };
            if refreshed {
                inner.dirty = true;
            }
            if inner.dirty && inner.flushed_at.elapsed() >= LAST_SEEN_FLUSH {
                self.persist(&mut inner);
            }
            Some(grant)
        }
    }

    /// Every paired device, oldest first.
    pub fn list(&self) -> Vec<DeviceView> {
        self.lock()
            .file
            .devices
            .iter()
            .map(DeviceView::from)
            .collect()
    }

    /// Whether anything has ever been paired. The CLI uses it to tell
    /// "no devices yet" from "the list is empty because they were revoked".
    pub fn is_empty(&self) -> bool {
        self.lock().file.devices.is_empty()
    }

    /// Open a pairing window, replacing any window already open. Returning the
    /// offer rather than printing it keeps the caller free to put it on a
    /// screen, in a terminal, or in a QR code.
    pub fn open_pairing(
        &self,
        scopes: Vec<Scope>,
        ttl: Duration,
        url: &str,
    ) -> Result<PairingOffer, String> {
        let code = random_code()?;
        let grouped = group_code(&code);
        let mut inner = self.lock();
        inner.pairing = Some(Pairing {
            code: code.clone(),
            expires_at: Instant::now() + ttl,
            attempts: 0,
            scopes: scopes.clone(),
        });
        let uri = format!(
            "tfg-control://pair?url={}&code={}",
            urlencoding(url),
            grouped
        );
        let payload = serde_json::json!({
            "protocol": "tfg-control",
            "version": 1,
            "url": url,
            "pairing_code": grouped,
        })
        .to_string();
        Ok(PairingOffer {
            code: grouped,
            expires_in: ttl.as_secs(),
            uri,
            payload,
            scopes,
        })
    }

    /// Close the window without pairing anything.
    pub fn close_pairing(&self) {
        self.lock().pairing = None;
    }

    /// Exchange a code for a device and its token.
    ///
    /// The code is compared in constant time and is spent on success: a
    /// window pairs exactly one device.
    pub fn pair(&self, code: &str, label: &str) -> Result<Paired, PairError> {
        let normalised = normalise_code(code);
        let token = random_token().map_err(|_| PairError::Unavailable)?;
        let device_id = format!("dev-{}", random_hex(6).map_err(|_| PairError::Unavailable)?);
        let mut inner = self.lock();
        let Some(pairing) = inner.pairing.clone() else {
            return Err(PairError::NoWindow);
        };
        if Instant::now() >= pairing.expires_at {
            inner.pairing = None;
            return Err(PairError::Expired);
        }
        if !constant_time_eq(normalised.as_bytes(), pairing.code.as_bytes()) {
            let attempts = pairing.attempts + 1;
            if attempts >= PAIRING_ATTEMPTS {
                inner.pairing = None;
                return Err(PairError::TooManyAttempts);
            }
            if let Some(open) = inner.pairing.as_mut() {
                open.attempts = attempts;
            }
            return Err(PairError::BadCode);
        }
        // The code was right: spend it, then mint the device.
        inner.pairing = None;
        let device = Device {
            id: device_id,
            label: bounded_label(label),
            scopes: pairing.scopes.clone(),
            created_at: unix_now(),
            last_seen: Some(unix_now()),
            token_hash: hash_token(&token),
        };
        let view = DeviceView::from(&device);
        inner.file.devices.push(device);
        self.persist(&mut inner);
        Ok(Paired {
            device: view,
            token,
        })
    }

    /// Whether a pairing window is open and still live.
    pub fn pairing_open(&self) -> bool {
        let mut inner = self.lock();
        match &inner.pairing {
            Some(pairing) if Instant::now() < pairing.expires_at => true,
            Some(_) => {
                inner.pairing = None;
                false
            }
            None => false,
        }
    }

    /// Forget a device. Its token stops working on the next request because
    /// the lookup is by hash against this table.
    pub fn revoke(&self, id: &str) -> bool {
        let mut inner = self.lock();
        let before = inner.file.devices.len();
        inner.file.devices.retain(|device| device.id != id);
        let removed = inner.file.devices.len() != before;
        if removed {
            self.persist(&mut inner);
        }
        removed
    }

    /// Replace a device's token, keeping its id, label and scopes. Returns the
    /// new plaintext token; the old one stops working immediately.
    pub fn rotate(&self, id: &str) -> Option<String> {
        let token = random_token().ok()?;
        let mut inner = self.lock();
        let device = inner
            .file
            .devices
            .iter_mut()
            .find(|device| device.id == id)?;
        device.token_hash = hash_token(&token);
        self.persist(&mut inner);
        Some(token)
    }

    /// Write the table back. Best effort, and reported: a full disk must not
    /// look like a successful pairing that will not survive a restart.
    fn persist(&self, inner: &mut Inner) {
        let Some(path) = &self.path else {
            inner.dirty = false;
            inner.flushed_at = Instant::now();
            return;
        };
        if let Err(error) = write_atomic(path, &inner.file) {
            eprintln!("control devices: {error}");
            // Leave it dirty so the next attempt tries again.
            return;
        }
        inner.dirty = false;
        inner.flushed_at = Instant::now();
    }
}

fn write_atomic(path: &Path, file: &DeviceFile) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("creating {}: {error}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(file)
        .map_err(|error| format!("encoding the devices file: {error}"))?;
    // Written beside the target and renamed: a crash mid-write leaves the old
    // table, never half of a new one.
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, text)
        .map_err(|error| format!("writing {}: {error}", temporary.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&temporary, path)
        .map_err(|error| format!("renaming into {}: {error}", path.display()))
}

/// The operator's token and the device table, together: one call site decides
/// who a request is, in one order.
pub struct Credentials {
    operator: String,
    devices: Devices,
}

impl Credentials {
    pub fn new(operator: String, devices: Devices) -> Self {
        Self { operator, devices }
    }

    /// The operator's own token, for the CLI's messages. Never serialised.
    pub fn operator_token(&self) -> &str {
        &self.operator
    }

    pub fn devices(&self) -> &Devices {
        &self.devices
    }

    /// Resolve an `Authorization: Bearer …` header to a grant.
    ///
    /// The operator token is tried first: it is the one credential that is not
    /// in the file, and it is the one that must never be locked out by a
    /// corrupt table.
    pub fn authorise(&self, headers: &hyper::HeaderMap) -> Option<Grant> {
        let presented = super::auth::bearer(headers)?;
        if !self.operator.is_empty()
            && constant_time_eq(presented.as_bytes(), self.operator.as_bytes())
        {
            return Some(Grant::operator());
        }
        self.devices.authorise(presented)
    }
}

fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

/// Crockford's alphabet: no `I`, `L`, `O` or `U`, so a code read off a screen
/// cannot be mistyped as a different valid code, and no code spells a word.
const ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

fn random_bytes(count: usize) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut buffer = vec![0u8; count];
    // The system's own entropy. There is no fallback on purpose: a pairing
    // code from a weak source is worse than no pairing at all, so a machine
    // that cannot produce one refuses to open a window and says why.
    let mut source = std::fs::File::open("/dev/urandom")
        .map_err(|error| format!("no /dev/urandom for a pairing code: {error}"))?;
    source
        .read_exact(&mut buffer)
        .map_err(|error| format!("reading /dev/urandom: {error}"))?;
    Ok(buffer)
}

fn random_code() -> Result<String, String> {
    // Eight Crockford characters, forty bits. The modulo bias of `% 32` over a
    // uniform byte is zero, because 32 divides 256.
    let bytes = random_bytes(8)?;
    Ok(bytes
        .iter()
        .map(|byte| ALPHABET[usize::from(byte % 32)] as char)
        .collect())
}

fn random_hex(bytes: usize) -> Result<String, String> {
    Ok(random_bytes(bytes)?
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn random_token() -> Result<String, String> {
    // 32 bytes: 256 bits, which is what the operator token's minimum length is
    // standing in for.
    random_hex(32)
}

/// `K7QM3XPD` becomes `K7QM-3XPD`, which is easier to read aloud and to type.
fn group_code(code: &str) -> String {
    if code.len() == 8 {
        format!("{}-{}", &code[..4], &code[4..])
    } else {
        code.to_string()
    }
}

/// Accept a code however it was typed: lower case, with or without the dash,
/// and with the characters Crockford folds (`I`/`L` for `1`, `O` for `0`).
fn normalise_code(raw: &str) -> String {
    raw.chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '-')
        .map(|ch| match ch.to_ascii_uppercase() {
            'I' | 'L' => '1',
            'O' => '0',
            other => other,
        })
        .collect()
}

/// A device name a person will recognise in a list, bounded so a log line
/// cannot be forged across lines.
fn bounded_label(raw: &str) -> String {
    let cleaned: String = raw
        .trim()
        .chars()
        .filter(|ch| !ch.is_control())
        .take(48)
        .collect();
    if cleaned.is_empty() {
        "unnamed device".to_string()
    } else {
        cleaned
    }
}

/// Percent-encode the few characters that matter in the pairing URI. A full
/// encoder would be a dependency for a string that is a host, a port and a
/// path.
fn urlencoding(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'.'
            | b'_'
            | b'~'
            | b':'
            | b'/'
            | b'['
            | b']' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn devices() -> Devices {
        Devices::open(None).expect("in-memory devices")
    }

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tfg-devices-{}-{name}-{}",
            std::process::id(),
            random_hex(4).expect("random")
        ));
        dir.join("control-devices.json")
    }

    fn pair_one(devices: &Devices, label: &str) -> Paired {
        let offer = devices
            .open_pairing(
                vec![Scope::Observe, Scope::ControlView],
                PAIRING_TTL,
                "http://x",
            )
            .expect("window");
        devices.pair(&offer.code, label).expect("paired")
    }

    #[test]
    fn a_paired_device_can_authorise_and_carries_its_scopes() {
        let devices = devices();
        let paired = pair_one(&devices, "phone");
        let grant = devices.authorise(&paired.token).expect("authorised");
        assert_eq!(grant.id, paired.device.id);
        assert_eq!(grant.label, "phone");
        assert!(grant.allows(Scope::ControlView));
        assert!(grant.allows(Scope::Observe));
        assert!(!grant.allows(Scope::Admin));
    }

    #[test]
    fn the_operator_implicitly_holds_every_scope() {
        let grant = Grant::operator();
        assert!(grant.allows(Scope::Observe));
        assert!(grant.allows(Scope::ControlView));
        assert!(grant.allows(Scope::Admin));
    }

    #[test]
    fn a_control_credential_can_observe_and_an_observe_credential_cannot_control() {
        assert!(Scope::ControlView.implies(Scope::Observe));
        assert!(!Scope::Observe.implies(Scope::ControlView));
        assert!(Scope::Admin.implies(Scope::ControlView));
        assert!(!Scope::ControlView.implies(Scope::Admin));
    }

    #[test]
    fn a_token_is_not_stored_in_the_clear() {
        let devices = devices();
        let paired = pair_one(&devices, "phone");
        let stored = devices.lock().file.devices[0].token_hash.clone();
        assert_ne!(stored, paired.token);
        assert_eq!(stored, hash_token(&paired.token));
        assert_eq!(stored.len(), 64);
    }

    #[test]
    fn a_wrong_token_authorises_nobody() {
        let devices = devices();
        let _ = pair_one(&devices, "phone");
        assert!(
            devices
                .authorise("not-a-token-that-was-ever-issued")
                .is_none()
        );
        assert!(devices.authorise("").is_none());
    }

    #[test]
    fn a_pairing_code_is_spent_by_the_device_that_used_it() {
        let devices = devices();
        let offer = devices
            .open_pairing(vec![Scope::Observe], PAIRING_TTL, "http://x")
            .expect("window");
        assert!(devices.pair(&offer.code, "phone").is_ok());
        assert_eq!(
            devices.pair(&offer.code, "laptop"),
            Err(PairError::NoWindow)
        );
        assert_eq!(devices.list().len(), 1);
    }

    #[test]
    fn a_code_is_readable_and_forgiving_of_how_it_was_typed() {
        let devices = devices();
        let offer = devices
            .open_pairing(vec![Scope::Observe], PAIRING_TTL, "http://x")
            .expect("window");
        assert_eq!(offer.code.len(), 9, "grouped as XXXX-XXXX");
        let typed = offer.code.to_lowercase().replace('-', " ");
        assert!(devices.pair(&typed, "phone").is_ok());
    }

    #[test]
    fn crockford_folds_the_characters_that_look_alike() {
        assert_eq!(normalise_code("k7qm-3xpd"), "K7QM3XPD");
        assert_eq!(normalise_code("IL0O"), "1100");
    }

    #[test]
    fn five_wrong_codes_close_the_window() {
        let devices = devices();
        let offer = devices
            .open_pairing(vec![Scope::Observe], PAIRING_TTL, "http://x")
            .expect("window");
        for _ in 0..(PAIRING_ATTEMPTS - 1) {
            assert_eq!(devices.pair("WRONGCOD", "x"), Err(PairError::BadCode));
        }
        assert_eq!(
            devices.pair("WRONGCOD", "x"),
            Err(PairError::TooManyAttempts)
        );
        // Even the right code is no use now.
        assert_eq!(devices.pair(&offer.code, "x"), Err(PairError::NoWindow));
    }

    #[test]
    fn an_expired_window_refuses_the_right_code() {
        let devices = devices();
        let offer = devices
            .open_pairing(vec![Scope::Observe], Duration::ZERO, "http://x")
            .expect("window");
        assert_eq!(devices.pair(&offer.code, "x"), Err(PairError::Expired));
        assert!(!devices.pairing_open());
    }

    #[test]
    fn a_revoked_device_loses_its_token_immediately() {
        let devices = devices();
        let paired = pair_one(&devices, "phone");
        assert!(devices.authorise(&paired.token).is_some());
        assert!(devices.revoke(&paired.device.id));
        assert!(devices.authorise(&paired.token).is_none());
        assert!(
            !devices.revoke(&paired.device.id),
            "revoking twice is a no-op"
        );
    }

    #[test]
    fn rotation_retires_the_old_token_and_keeps_the_device() {
        let devices = devices();
        let paired = pair_one(&devices, "phone");
        let fresh = devices.rotate(&paired.device.id).expect("rotated");
        assert_ne!(fresh, paired.token);
        assert!(devices.authorise(&paired.token).is_none());
        assert_eq!(
            devices.authorise(&fresh).expect("the new token works").id,
            paired.device.id
        );
        assert_eq!(devices.list().len(), 1);
        assert!(devices.rotate("dev-nothing").is_none());
    }

    #[test]
    fn the_table_survives_a_restart_and_so_does_revocation() {
        let path = temp_path("restart");
        let token;
        let gone;
        {
            let devices = Devices::open(Some(path.clone())).expect("open");
            let kept = pair_one(&devices, "phone");
            let dropped = pair_one(&devices, "old laptop");
            token = kept.token;
            gone = dropped.token;
            assert!(devices.revoke(&dropped.device.id));
        }
        let reopened = Devices::open(Some(path.clone())).expect("reopen");
        assert!(reopened.authorise(&token).is_some());
        assert!(
            reopened.authorise(&gone).is_none(),
            "a revoked device came back after a restart"
        );
        assert_eq!(reopened.list().len(), 1);
        let _ = std::fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn a_devices_file_that_does_not_parse_is_an_error_not_an_empty_table() {
        let path = temp_path("corrupt");
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, "{ not json").expect("write");
        let error = match Devices::open(Some(path.clone())) {
            Ok(_) => panic!("a corrupt devices file must be refused"),
            Err(error) => error,
        };
        assert!(error.contains("not a devices file"), "{error}");
        let _ = std::fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn a_pairing_offer_carries_the_uri_and_payload_a_qr_code_would_hold() {
        let devices = devices();
        let offer = devices
            .open_pairing(
                vec![Scope::Observe, Scope::ControlView],
                PAIRING_TTL,
                "http://192.168.1.20:8787",
            )
            .expect("window");
        assert!(offer.uri.starts_with("tfg-control://pair?"));
        assert!(offer.uri.contains("192.168.1.20"));
        let payload: serde_json::Value = serde_json::from_str(&offer.payload).expect("json");
        assert_eq!(payload["protocol"], "tfg-control");
        assert_eq!(payload["version"], 1);
        assert_eq!(payload["url"], "http://192.168.1.20:8787");
        assert_eq!(payload["pairing_code"], offer.code);
    }

    #[test]
    fn every_scope_in_the_code_is_a_scope_in_the_schema() {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../schemas/control-v1.json"))
                .expect("schemas/control-v1.json is valid JSON");
        let documented: Vec<&str> = schema["$defs"]["scope"]["enum"]
            .as_array()
            .expect("the schema lists its scopes")
            .iter()
            .map(|value| value.as_str().expect("a string"))
            .collect();
        for scope in [Scope::Observe, Scope::ControlView, Scope::Admin] {
            assert!(
                documented.contains(&scope.name()),
                "`{}` is a scope in the code and not in the schema",
                scope.name()
            );
        }
        assert_eq!(documented.len(), 3, "a scope the code does not have");
    }

    #[test]
    fn a_label_is_bounded_and_never_empty() {
        assert_eq!(bounded_label("   "), "unnamed device");
        assert_eq!(bounded_label("phone\nsecond line"), "phonesecond line");
        assert_eq!(bounded_label(&"x".repeat(200)).len(), 48);
    }

    #[test]
    fn an_empty_operator_token_authorises_nobody() {
        // Not reachable through `spawn_server`, which refuses an empty token,
        // but a credential store that treated "" as a match would be a very
        // quiet catastrophe.
        let credentials = Credentials::new(String::new(), devices());
        let mut headers = hyper::HeaderMap::new();
        headers.insert(
            hyper::header::AUTHORIZATION,
            hyper::header::HeaderValue::from_static("Bearer anything-at-all"),
        );
        assert!(credentials.authorise(&headers).is_none());
    }

    #[test]
    fn the_operator_token_is_tried_before_the_device_table() {
        let devices = devices();
        let credentials = Credentials::new("o".repeat(64), devices);
        let mut headers = hyper::HeaderMap::new();
        headers.insert(
            hyper::header::AUTHORIZATION,
            hyper::header::HeaderValue::from_str(&format!("Bearer {}", "o".repeat(64)))
                .expect("header"),
        );
        let grant = credentials.authorise(&headers).expect("operator");
        assert_eq!(grant.id, "operator");
        assert!(grant.allows(Scope::Admin));

        headers.insert(
            hyper::header::AUTHORIZATION,
            hyper::header::HeaderValue::from_static("Bearer nope"),
        );
        assert!(credentials.authorise(&headers).is_none());
    }
}
