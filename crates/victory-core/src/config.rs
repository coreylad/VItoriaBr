//! Where the user's files live.
//!
//! Resolution order for each value: environment variable, then the config
//! file (`VICTORY_CONFIG`, else `./victory.toml`), then the default.
//! See `victory.example.toml` at the repository root.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::policy::ensure_outside_repo;

#[derive(Debug, Default, Clone)]
pub struct Config {
    /// The user's dump of the title.
    pub game_dir: Option<PathBuf>,
    /// Optional executable used by `run` when none is passed on the command line.
    pub game_exe: Option<PathBuf>,
    /// Root for all derived data. Always outside the repository.
    pub cache_dir: PathBuf,
    /// Translated shaders and host pipelines, defaulting to `<cache>/shaders`.
    pub shader_cache_dir: PathBuf,
    /// What titles write and must keep (persistent local storage). Never
    /// inside the cache, which may be deleted at any time.
    pub data_dir: PathBuf,
    /// Optional directory for an append-only `victory.log`.
    pub log_dir: Option<PathBuf>,
    /// Runtime defaults from the file; inherited environment variables win.
    pub environment: BTreeMap<String, String>,
    /// Other titles used to exercise platform-level code.
    pub test_titles: Vec<PathBuf>,
    /// The config file that was read, if any.
    pub source: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    #[serde(default)]
    paths: FilePaths,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FilePaths {
    game_dir: Option<PathBuf>,
    game_exe: Option<PathBuf>,
    cache_dir: Option<PathBuf>,
    shader_cache_dir: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    log_dir: Option<PathBuf>,
    #[serde(default)]
    test_titles: Vec<PathBuf>,
}

impl Config {
    /// Loads configuration from the environment and the config file.
    pub fn load() -> Result<Self> {
        let explicit = std::env::var_os("VICTORY_CONFIG").map(PathBuf::from);
        let path = explicit
            .clone()
            .unwrap_or_else(|| PathBuf::from("victory.toml"));
        let file = if path.is_file() {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            Some((path, text))
        } else if let Some(p) = explicit {
            anyhow::bail!(
                "VICTORY_CONFIG points at {}, which does not exist",
                p.display()
            );
        } else {
            None
        };
        let (source, parsed) = match file {
            Some((path, text)) => {
                let parsed: FileConfig =
                    toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
                (Some(path), parsed)
            }
            None => (None, FileConfig::default()),
        };
        Self::resolve(parsed.paths, parsed.environment, source, |k| {
            std::env::var_os(k)
        })
    }

    fn resolve(
        file: FilePaths,
        environment: BTreeMap<String, String>,
        source: Option<PathBuf>,
        env: impl Fn(&str) -> Option<std::ffi::OsString>,
    ) -> Result<Self> {
        validate_environment(&environment)?;
        let value = |k: &str| env(k).or_else(|| environment.get(k).map(OsString::from));
        let env_path = |k: &str| value(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        let test_titles = match value("VICTORY_TEST_TITLES") {
            Some(v) if !v.is_empty() => std::env::split_paths(&v)
                .filter(|p| !p.as_os_str().is_empty())
                .collect(),
            _ => file.test_titles,
        };
        let cache_dir = env_path("VICTORY_CACHE_DIR")
            .or(file.cache_dir)
            .map_or_else(|| default_dir("cache"), Ok)?;
        let data_dir = env_path("VICTORY_DATA_DIR")
            .or(file.data_dir)
            .map_or_else(|| default_dir("data"), Ok)?;
        let shader_cache_dir = env_path("VICTORY_SHADER_CACHE_DIR")
            .or(file.shader_cache_dir)
            .unwrap_or_else(|| cache_dir.join("shaders"));
        Ok(Config {
            game_dir: env_path("VICTORY_GAME_DIR").or(file.game_dir),
            game_exe: env_path("VICTORY_GAME_EXE").or(file.game_exe),
            cache_dir,
            shader_cache_dir,
            data_dir,
            log_dir: env_path("VICTORY_LOG_DIR").or(file.log_dir),
            environment,
            test_titles,
            source,
        })
    }

    /// Installs file defaults without overwriting inherited environment variables.
    ///
    /// # Safety
    /// Call only during single-threaded startup, before logging, runtime threads,
    /// or any code that may access the process environment concurrently.
    pub unsafe fn apply_environment(&self) -> Result<()> {
        validate_environment(&self.environment)?;
        for (key, value) in &self.environment {
            if std::env::var_os(key).is_none() {
                // SAFETY: the caller guarantees single-threaded startup.
                unsafe { std::env::set_var(key, value) };
            }
        }
        Ok(())
    }

    /// The configured executable, when `run` has no positional path.
    pub fn require_game_exe(&self) -> Result<&Path> {
        self.game_exe.as_deref().context(
            "no game executable configured: pass it to run, set VICTORY_GAME_EXE, \
             or set [paths].game_exe in victory.toml",
        )
    }

    /// The game directory, or a helpful error if it is not configured.
    pub fn require_game_dir(&self) -> Result<&Path> {
        self.game_dir.as_deref().context(
            "no game directory configured: set VICTORY_GAME_DIR or [paths].game_dir in \
             victory.toml (see victory.example.toml)",
        )
    }

    /// Returns `<cache>/<sub>`, creating it, after checking the asset policy.
    pub fn cache_subdir(&self, sub: &str) -> Result<PathBuf> {
        subdir(&self.cache_dir, sub)
    }

    /// Creates the configured shader cache after checking the asset policy.
    pub fn shader_cache(&self) -> Result<PathBuf> {
        subdir(&self.shader_cache_dir, "")
    }

    /// Returns `<data>/<sub>`, creating it, after checking the asset policy.
    pub fn data_subdir(&self, sub: &str) -> Result<PathBuf> {
        subdir(&self.data_dir, sub)
    }
}

fn validate_environment(environment: &BTreeMap<String, String>) -> Result<()> {
    for (key, value) in environment {
        anyhow::ensure!(
            key.strip_prefix("VICTORY_").is_some_and(|suffix| {
                !suffix.is_empty()
                    && suffix
                        .bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
            }) && key != "VICTORY_CONFIG",
            "invalid [environment] key {key:?}: use uppercase VICTORY_* names other than VICTORY_CONFIG"
        );
        anyhow::ensure!(
            !value.contains('\0'),
            "[environment].{key} contains a NUL character"
        );
    }
    Ok(())
}

fn subdir(root: &Path, sub: &str) -> Result<PathBuf> {
    let dir = root.join(sub);
    ensure_outside_repo(&dir)?;
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    Ok(dir)
}

/// `%LOCALAPPDATA%/victory/<name>` (elsewhere the XDG or home cache).
fn default_dir(name: &str) -> Result<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("XDG_CACHE_HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .context("cannot determine a user directory; set VICTORY_CACHE_DIR and VICTORY_DATA_DIR")?;
    Ok(base.join("victory").join(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_overrides_file() {
        let file = FilePaths {
            game_dir: Some("from-file".into()),
            cache_dir: Some("cache-from-file".into()),
            ..Default::default()
        };
        let env = |k: &str| (k == "VICTORY_GAME_DIR").then(|| OsString::from("from-env"));
        let cfg = Config::resolve(file, BTreeMap::new(), None, env).unwrap();
        assert_eq!(cfg.game_dir.unwrap(), PathBuf::from("from-env"));
        assert_eq!(cfg.cache_dir, PathBuf::from("cache-from-file"));
    }

    #[test]
    fn empty_env_values_are_ignored() {
        let file = FilePaths {
            game_dir: Some("from-file".into()),
            ..Default::default()
        };
        let env = |k: &str| (k == "VICTORY_GAME_DIR").then(OsString::new);
        let cfg = Config::resolve(file, BTreeMap::new(), None, env).unwrap();
        assert_eq!(cfg.game_dir.unwrap(), PathBuf::from("from-file"));
    }

    #[test]
    fn log_directory_respects_environment_and_empty_values() {
        let file = FilePaths {
            log_dir: Some("logs-from-file".into()),
            ..Default::default()
        };
        let env = |k: &str| (k == "VICTORY_LOG_DIR").then(|| OsString::from("logs-from-env"));
        let cfg = Config::resolve(file, BTreeMap::new(), None, env).unwrap();
        assert_eq!(cfg.log_dir.unwrap(), PathBuf::from("logs-from-env"));

        let file = FilePaths {
            log_dir: Some("logs-from-file".into()),
            ..Default::default()
        };
        let env = |k: &str| (k == "VICTORY_LOG_DIR").then(OsString::new);
        let cfg = Config::resolve(file, BTreeMap::new(), None, env).unwrap();
        assert_eq!(cfg.log_dir.unwrap(), PathBuf::from("logs-from-file"));
        assert!(
            Config::resolve(FilePaths::default(), BTreeMap::new(), None, |_| None)
                .unwrap()
                .log_dir
                .is_none()
        );
    }

    #[test]
    fn runtime_environment_defaults_resolve_paths_below_inherited_values() {
        let parsed: FileConfig = toml::from_str(
            r#"
            [paths]
            game_exe = "file-game.exe"
            shader_cache_dir = "file-shaders"
            log_dir = "file-logs"
            [environment]
            VICTORY_GAME_EXE = "table-game.exe"
            VICTORY_SHADER_CACHE_DIR = "table-shaders"
            VICTORY_LOG_DIR = "table-logs"
            VICTORY_RELAY = "2"
            "#,
        )
        .unwrap();
        let env = |key: &str| (key == "VICTORY_LOG_DIR").then(|| OsString::from("inherited-logs"));
        let cfg = Config::resolve(parsed.paths, parsed.environment, None, env).unwrap();
        assert_eq!(cfg.require_game_exe().unwrap(), Path::new("table-game.exe"));
        assert_eq!(cfg.shader_cache_dir, Path::new("table-shaders"));
        assert_eq!(cfg.log_dir.unwrap(), Path::new("inherited-logs"));
        assert_eq!(cfg.environment["VICTORY_RELAY"], "2");
    }

    #[test]
    fn shader_cache_defaults_to_cache_root_and_rejects_source_tree() {
        let file = FilePaths {
            cache_dir: Some("custom-cache".into()),
            ..Default::default()
        };
        let cfg = Config::resolve(file, BTreeMap::new(), None, |_| None).unwrap();
        assert_eq!(
            cfg.shader_cache_dir,
            Path::new("custom-cache").join("shaders")
        );

        let cfg = Config {
            shader_cache_dir: Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders"),
            ..Default::default()
        };
        assert!(cfg.shader_cache().is_err());
    }

    #[test]
    fn invalid_runtime_environment_is_rejected_before_application() {
        for (key, value) in [
            ("PATH", "other"),
            ("VICTORY_CONFIG", "other.toml"),
            ("VICTORY_BAD=KEY", "1"),
            ("VICTORY_RELAY", "2\0"),
        ] {
            let environment = BTreeMap::from([(key.to_owned(), value.to_owned())]);
            assert!(
                Config::resolve(FilePaths::default(), environment, None, |_| None).is_err(),
                "{key:?} should be rejected"
            );
        }
        assert!(toml::from_str::<FileConfig>("[environment]\nVICTORY_RELAY = 2").is_err());
    }

    #[test]
    fn empty_test_titles_env_is_ignored() {
        let file = FilePaths {
            test_titles: vec!["from-file".into()],
            ..Default::default()
        };
        let env = |k: &str| (k == "VICTORY_TEST_TITLES").then(OsString::new);
        let cfg = Config::resolve(file, BTreeMap::new(), None, env).unwrap();
        assert_eq!(cfg.test_titles, vec![PathBuf::from("from-file")]);
    }

    #[test]
    fn test_titles_env_splits_paths() {
        let value = std::env::join_paths(["first", "second"]).unwrap();
        let file = FilePaths::default();
        let env = |k: &str| (k == "VICTORY_TEST_TITLES").then(|| value.clone());
        let cfg = Config::resolve(file, BTreeMap::new(), None, env).unwrap();
        assert_eq!(
            cfg.test_titles,
            vec![PathBuf::from("first"), PathBuf::from("second")]
        );
    }

    #[test]
    fn example_config_parses() {
        let text = include_str!("../../../victory.example.toml");
        let parsed: FileConfig = toml::from_str(text).unwrap();
        assert!(parsed.paths.game_dir.is_some());
    }

    #[test]
    fn cache_inside_repo_is_refused() {
        let inside = Path::new(env!("CARGO_MANIFEST_DIR")).join("cache");
        let cfg = Config {
            cache_dir: inside,
            ..Default::default()
        };
        assert!(cfg.cache_subdir("x").is_err());
    }

    #[test]
    fn data_inside_repo_is_refused() {
        let inside = Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
        let cfg = Config {
            data_dir: inside,
            ..Default::default()
        };
        assert!(cfg.data_subdir("x").is_err());
    }
}
