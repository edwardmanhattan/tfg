//! The console's own settings, remembered across launches.
//!
//! ONE FILE, and it belongs to this machine: the operator's choices are
//! about how the console reads in this room, not about the exercise, and
//! nothing here is ever sent to Minos.
//!
//! WHY A FILE AND NOT A FLAG. The text size is the one setting an
//! operator sets ONCE — in the room, before the exercise, because the
//! console was too small to read from the back of it — and then never
//! touches again. A setting that resets on every launch is not a setting;
//! it is a chore with a control on it.
//!
//! WHY IT IS WRITTEN BY HAND. The console is air-gapped and the settings
//! file is four lines of JSON, so it is written the four lines are worth.
//! A settings framework would be a dependency, a migration story and a
//! schema for one number.
//!
//! AND WHY A CORRUPT FILE IS NOT FATAL. A settings file that cannot be
//! read must not stop the console from opening: the operator's size is a
//! preference, and a console that will not open because of one is a
//! console that is now broken by a file it did not need. A bad file is
//! ignored and left alone, and the console boots at its default.

use std::path::Path;

/// What the console remembers between launches.
///
/// Every field is optional, and a missing field means "the default" — so a
/// file written by an older console still opens, and a file written by a
/// newer one still reads (the fields this build does not know are ignored
/// rather than refused).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    /// The text scale the operator chose.
    ///
    /// The NUMBER, not the step: the ladder can gain a rung between two
    /// launches, and a step remembered as an index would then mean a
    /// different size. A scale remembered as a scale means what it said.
    /// [`crate::view::set_text_scale`] snaps it to the nearest rung, so a
    /// scale this build no longer offers lands on the closest one it does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_scale: Option<f32>,
}

impl Settings {
    /// Read the settings, or the defaults when there are none to read.
    ///
    /// Every failure — no file, an unreadable file, a file that is not
    /// JSON, JSON that is not this shape — answers the same way: the
    /// defaults. A settings file is a convenience, and a convenience that
    /// can refuse to open the console has become a dependency.
    pub fn load(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    /// Write the settings, and say whether they were written.
    ///
    /// The write goes to a TEMP FILE and is then renamed over the real
    /// one, because a console that is killed mid-write must not leave a
    /// half-file where the settings were. A rename is atomic on every
    /// filesystem this console runs on.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| format!("the settings could not be written: {e}"))?;
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, text)
            .map_err(|e| format!("the settings could not be written: {e}"))?;
        std::fs::rename(&temp, path).map_err(|e| {
            let _ = std::fs::remove_file(&temp);
            format!("the settings could not be saved: {e}")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("tfg-settings-tests");
        std::fs::create_dir_all(&dir).expect("a temp dir");
        dir.join(name)
    }

    #[test]
    fn a_settings_file_that_is_not_there_reads_as_the_defaults() {
        let path = temp_path("absent.json");
        let _ = std::fs::remove_file(&path);
        assert_eq!(Settings::load(&path), Settings::default());
    }

    #[test]
    fn a_settings_file_that_is_rubbish_reads_as_the_defaults() {
        // A corrupt file must not stop the console from opening: the size
        // is a preference and the console is not.
        let path = temp_path("rubbish.json");
        std::fs::write(&path, "{ this is not json").expect("a file");
        assert_eq!(Settings::load(&path), Settings::default());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_size_survives_a_launch() {
        let path = temp_path("size.json");
        let settings = Settings { text_scale: Some(1.3) };
        settings.save(&path).expect("the settings are written");
        assert_eq!(Settings::load(&path), settings);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_file_without_the_size_reads_as_no_size() {
        // An older console's file, or a newer one's field this build does
        // not know: the field it does know is still read.
        let path = temp_path("partial.json");
        std::fs::write(&path, r#"{"something_else": 7}"#).expect("a file");
        assert_eq!(Settings::load(&path), Settings::default());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_write_leaves_no_half_file_behind() {
        let path = temp_path("atomic.json");
        let settings = Settings { text_scale: Some(1.15) };
        settings.save(&path).expect("the settings are written");
        // The temp file is GONE — it was renamed over the real one.
        assert!(
            !path.with_extension("json.tmp").exists(),
            "the temp file was left behind"
        );
        assert!(path.exists(), "and the real one is there");
        let _ = std::fs::remove_file(&path);
    }
}
