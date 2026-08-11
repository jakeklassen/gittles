//! The on-disk store: a config file and a cached copy of your stars.
//!
//! Paths are injectable ([`Store::new`]) so tests never touch a real home
//! directory; [`Store::discover`] is the production constructor.

use anyhow::Result;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::github::Star;

/// The CLI stores under `~/.config/gittles`. The desktop app deliberately does
/// *not* share that directory: the two serialise `Star` differently (camelCase
/// vs snake_case), so a shared cache would leave each build treating the
/// other's file as corrupt and re-syncing every switch.
const APP_DIR: &str = "gittles-desktop";

/// `#[serde(default)]` is load-bearing. The three fields below are always
/// written, but any field added later will be missing from configs written by
/// an older build — and without `default` that would fail the parse and log the
/// user out. Add new fields as `Option<_>`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub token: String,
    pub username: String,
    pub last_synced_at: String,
}

#[derive(Debug, Clone)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Store { dir: dir.into() }
    }

    /// The real store, under the platform config directory.
    pub fn discover() -> Result<Self> {
        let dirs = ProjectDirs::from("", "", APP_DIR)
            .ok_or_else(|| anyhow::anyhow!("could not locate a config directory"))?;
        Ok(Store::new(dirs.config_dir()))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn config_path(&self) -> PathBuf {
        self.dir.join("config.json")
    }

    pub fn stars_path(&self) -> PathBuf {
        self.dir.join("stars.json")
    }

    /// A config the current shape cannot describe is treated as absent. Better
    /// to re-authenticate than to crash on every launch.
    pub fn load_config(&self) -> Config {
        read_json(&self.config_path()).unwrap_or_default()
    }

    pub fn save_config(&self, config: &Config) -> Result<()> {
        fs::create_dir_all(&self.dir)?;
        fs::write(self.config_path(), serde_json::to_string(config)?)?;
        Ok(())
    }

    /// Forget the token but keep everything else — a logout should not also
    /// throw away the sync bookkeeping.
    pub fn clear_auth(&self) -> Result<()> {
        let config = self.load_config();
        self.save_config(&Config {
            token: String::new(),
            username: String::new(),
            ..config
        })
    }

    pub fn is_authenticated(&self) -> bool {
        !self.load_config().token.is_empty()
    }

    /// A stored shape that no longer matches `Star` is what a schema change
    /// looks like. Treat it as "no cache" rather than a crash; the caller
    /// re-syncs.
    pub fn load_stars(&self) -> Vec<Star> {
        read_json(&self.stars_path()).unwrap_or_default()
    }

    pub fn save_stars(&self, stars: &[Star]) -> Result<()> {
        fs::create_dir_all(&self.dir)?;
        fs::write(self.stars_path(), serde_json::to_string(stars)?)?;
        Ok(())
    }

    /// `now` is supplied by the caller so this stays free of a clock — and
    /// testable.
    pub fn mark_synced(&self, now: impl Into<String>) -> Result<()> {
        let config = self.load_config();
        self.save_config(&Config {
            last_synced_at: now.into(),
            ..config
        })
    }
}

/// Missing file or unparseable contents both read as `None`.
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path());
        (dir, store)
    }

    fn sample_star() -> Star {
        Star {
            id: 1,
            name: "zed".into(),
            full_name: "zed-industries/zed".into(),
            description: "Code at the speed of thought".into(),
            url: "https://github.com/zed-industries/zed".into(),
            language: "Rust".into(),
            stargazers_count: 50000,
            forks_count: 3000,
            open_issues_count: 2000,
            pushed_at: "2024-05-01T12:00:00Z".into(),
            starred_at: "2024-03-01T10:00:00Z".into(),
        }
    }

    #[test]
    fn missing_files_read_as_empty() {
        let (_tmp, store) = store();
        assert_eq!(store.load_config(), Config::default());
        assert!(store.load_stars().is_empty());
        assert!(!store.is_authenticated());
    }

    #[test]
    fn config_round_trips() {
        let (_tmp, store) = store();
        let config = Config {
            token: "gho_secret".into(),
            username: "jakeklassen".into(),
            last_synced_at: "2024-05-01T12:00:00Z".into(),
        };

        store.save_config(&config).unwrap();

        assert_eq!(store.load_config(), config);
        assert!(store.is_authenticated());
    }

    #[test]
    fn save_creates_the_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().join("nested").join("deeper"));

        store.save_config(&Config::default()).unwrap();

        assert!(store.config_path().exists());
    }

    #[test]
    fn unparseable_config_reads_as_empty_rather_than_crashing() {
        let (_tmp, store) = store();
        fs::create_dir_all(store.dir()).unwrap();
        fs::write(store.config_path(), "{ this is not json").unwrap();

        assert_eq!(store.load_config(), Config::default());
    }

    #[test]
    fn config_from_an_older_build_still_loads() {
        // Written before `last_synced_at` existed. It must not log the user out.
        let (_tmp, store) = store();
        fs::create_dir_all(store.dir()).unwrap();
        fs::write(
            store.config_path(),
            r#"{"token":"gho_secret","username":"jakeklassen"}"#,
        )
        .unwrap();

        let config = store.load_config();
        assert_eq!(config.token, "gho_secret");
        assert_eq!(config.last_synced_at, "");
        assert!(store.is_authenticated());
    }

    #[test]
    fn stars_round_trip() {
        let (_tmp, store) = store();
        let stars = vec![sample_star()];

        store.save_stars(&stars).unwrap();

        assert_eq!(store.load_stars(), stars);
    }

    #[test]
    fn stars_of_the_wrong_shape_read_as_no_cache() {
        let (_tmp, store) = store();
        fs::create_dir_all(store.dir()).unwrap();
        // Valid JSON, wrong schema — a `Star` needs far more than this.
        fs::write(store.stars_path(), r#"[{"id":1}]"#).unwrap();

        assert!(store.load_stars().is_empty());
    }

    #[test]
    fn clear_auth_keeps_the_sync_bookkeeping() {
        let (_tmp, store) = store();
        store
            .save_config(&Config {
                token: "gho_secret".into(),
                username: "jakeklassen".into(),
                last_synced_at: "2024-05-01T12:00:00Z".into(),
            })
            .unwrap();

        store.clear_auth().unwrap();

        let config = store.load_config();
        assert_eq!(config.token, "");
        assert_eq!(config.username, "");
        assert_eq!(config.last_synced_at, "2024-05-01T12:00:00Z");
        assert!(!store.is_authenticated());
    }

    #[test]
    fn mark_synced_keeps_the_token() {
        let (_tmp, store) = store();
        store
            .save_config(&Config {
                token: "gho_secret".into(),
                username: "jakeklassen".into(),
                last_synced_at: String::new(),
            })
            .unwrap();

        store.mark_synced("2024-05-02T09:00:00Z").unwrap();

        let config = store.load_config();
        assert_eq!(config.last_synced_at, "2024-05-02T09:00:00Z");
        assert_eq!(config.token, "gho_secret");
    }
}
