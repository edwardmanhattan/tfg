//! Runtime paths for installed and portable builds.
//!
//! Read-only assets are embedded in the executable. Everything writable
//! lives under one discovered data directory, never beside the source or
//! relative to the process working directory.

use std::path::{Path, PathBuf};

/// One process's read-only and writable runtime locations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    pub data_dir: PathBuf,
    pub local_db: PathBuf,
    pub last_user: PathBuf,
    pub map_cache: PathBuf,
    pub log_dir: PathBuf,
    pub initial_log: PathBuf,
}

impl AppPaths {
    /// Resolve the data directory and create its small directory tree.
    ///
    /// `TFG_DATA_DIR` wins for managed/offline deployments. `TFG_PORTABLE=1`
    /// keeps all mutable state under `<executable>/data`; otherwise the
    /// platform's per-user data location is used.
    pub fn discover() -> Result<Self, String> {
        Self::from_data_dir(resolve_data_dir()?)
    }

    /// Construct paths below an explicit data directory. This is also the
    /// test seam: no process-global environment mutation is required.
    pub fn from_data_dir(data_dir: impl Into<PathBuf>) -> Result<Self, String> {
        let data_dir = data_dir.into();
        let log_dir = data_dir.join("logs");
        let map_dir = data_dir.join("maps");
        std::fs::create_dir_all(&log_dir)
            .map_err(|e| format!("log directory unavailable ({}): {e}", log_dir.display()))?;
        std::fs::create_dir_all(&map_dir)
            .map_err(|e| format!("map directory unavailable ({}): {e}", map_dir.display()))?;
        Ok(Self {
            local_db: data_dir.join("tfg-local.db"),
            last_user: data_dir.join("tfg-last-user"),
            map_cache: map_dir.join("tfg-tiles-cache.sqlite"),
            initial_log: log_dir.join("tfg-session-log.jsonl"),
            log_dir,
            data_dir,
        })
    }
}

fn resolve_data_dir() -> Result<PathBuf, String> {
    if let Some(path) = non_empty_env("TFG_DATA_DIR") {
        return absolute(path);
    }
    if env_flag("TFG_PORTABLE") {
        let exe =
            std::env::current_exe().map_err(|e| format!("executable path unavailable: {e}"))?;
        let executable_dir = exe
            .parent()
            .ok_or_else(|| "executable has no parent directory".to_string())?;
        #[cfg(target_os = "macos")]
        let dir = portable_macos_dir(executable_dir);
        #[cfg(not(target_os = "macos"))]
        let dir = executable_dir.join("data");
        return absolute(dir);
    }
    platform_data_dir()
}

fn non_empty_env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn absolute(path: PathBuf) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Ok(path);
    }
    Ok(std::env::current_dir()
        .map_err(|e| format!("current directory unavailable: {e}"))?
        .join(path))
}

#[cfg(target_os = "windows")]
fn platform_data_dir() -> Result<PathBuf, String> {
    non_empty_env("LOCALAPPDATA")
        .map(|base| base.join("tfg"))
        .ok_or_else(|| "LOCALAPPDATA is not set".to_string())
}

#[cfg(target_os = "macos")]
fn portable_macos_dir(executable_dir: &Path) -> PathBuf {
    let is_app_binary = executable_dir
        .file_name()
        .is_some_and(|name| name == std::ffi::OsStr::new("MacOS"))
        && executable_dir
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name == std::ffi::OsStr::new("Contents"));
    if is_app_binary {
        if let Some(bundle_dir) = executable_dir.parent().and_then(Path::parent) {
            if let Some(package_dir) = bundle_dir.parent() {
                return package_dir.join("data");
            }
        }
    }
    executable_dir.join("data")
}

#[cfg(target_os = "macos")]
fn platform_data_dir() -> Result<PathBuf, String> {
    non_empty_env("HOME")
        .map(|home| home.join("Library").join("Application Support").join("tfg"))
        .ok_or_else(|| "HOME is not set".to_string())
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn platform_data_dir() -> Result<PathBuf, String> {
    if let Some(base) = non_empty_env("XDG_DATA_HOME").filter(|path| path.is_absolute()) {
        return Ok(base.join("tfg"));
    }
    if let Some(home) = non_empty_env("HOME") {
        return Ok(home.join(".local").join("share").join("tfg"));
    }
    Err("neither XDG_DATA_HOME nor HOME is set".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_data_dir_owns_every_mutable_path() {
        let root = std::env::temp_dir().join(format!(
            "tfg-paths-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let paths = AppPaths::from_data_dir(&root).expect("paths");
        assert_eq!(paths.local_db, root.join("tfg-local.db"));
        assert_eq!(paths.last_user, root.join("tfg-last-user"));
        assert_eq!(paths.map_cache, root.join("maps/tfg-tiles-cache.sqlite"));
        assert_eq!(paths.initial_log, root.join("logs/tfg-session-log.jsonl"));
        assert!(paths.log_dir.is_dir());
        assert!(paths.map_cache.parent().is_some_and(Path::is_dir));
        std::fs::remove_dir_all(root).ok();
    }
}
