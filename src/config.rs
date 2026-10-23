//! Persisted settings, as JSON in `%APPDATA%\Porta\config.json` on Windows and
//! `~/Library/Application Support/Porta/config.json` on macOS.

use crate::log::lerror;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    /// List the OS's own listeners too. Off by default: there are dozens on
    /// Windows, nobody started them, and stopping one is never the fix.
    pub show_system: bool,
    /// How often to look again. Listing sockets is cheap; this is about not
    /// rebuilding a menu nobody is looking at more often than useful.
    pub refresh_seconds: u64,
    /// macOS: how many of your servers are up, beside the door in the menu
    /// bar. Nothing is shown while there are none.
    pub show_count: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            show_system: false,
            refresh_seconds: 5,
            show_count: true,
        }
    }
}

impl Config {
    #[cfg(windows)]
    pub fn dir() -> Option<PathBuf> {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("Porta"))
    }

    #[cfg(target_os = "macos")]
    pub fn dir() -> Option<PathBuf> {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/Porta"))
    }

    pub fn path() -> Option<PathBuf> {
        Self::dir().map(|d| d.join("config.json"))
    }

    pub fn modified() -> Option<std::time::SystemTime> {
        std::fs::metadata(Self::path()?).ok()?.modified().ok()
    }

    /// `None` for missing or unparsable, so an editor caught mid-save changes
    /// nothing.
    pub fn reload() -> Option<Self> {
        Self::parse(&std::fs::read_to_string(Self::path()?).ok()?).ok()
    }

    fn parse(text: &str) -> Result<Self, serde_json::Error> {
        // Notepad and PowerShell both like to write a BOM; serde_json does not.
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        serde_json::from_str::<Config>(text).map(Config::sanitised)
    }

    /// Never fails: a missing or corrupt file yields defaults.
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        Self::parse(&text).unwrap_or_else(|e| {
            lerror!(
                "config at {} is unreadable ({e}); using defaults",
                path.display()
            );
            Self::default()
        })
    }

    /// Temp file + rename, so an interrupted save never leaves half a file.
    pub fn save(&self) -> std::io::Result<()> {
        let (Some(dir), Some(path)) = (Self::dir(), Self::path()) else {
            return Err(std::io::Error::other("no config directory in environment"));
        };
        std::fs::create_dir_all(&dir)?;
        let json = serde_json::to_string_pretty(self)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &path)
    }

    pub fn sanitised(mut self) -> Self {
        self.refresh_seconds = self.refresh_seconds.clamp(1, 300);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tolerates_bom_missing_fields_and_nonsense() {
        let c = Config::parse("\u{feff}{\"showSystem\": true, \"refreshSeconds\": 0}").unwrap();
        assert!(c.show_system);
        assert_eq!(c.refresh_seconds, 1);
        assert_eq!(Config::parse("{}").unwrap(), Config::default());
    }
}
