use std::path::{Path, PathBuf};

use directories::BaseDirs;

use crate::{Result, StorageError};

/// XDG-compliant LoreLM path resolver.
#[derive(Debug, Clone)]
pub struct AppPaths {
    config_dir: PathBuf,
    data_dir: PathBuf,
    cache_dir: PathBuf,
    state_dir: PathBuf,
}

impl AppPaths {
    /// Resolves project directories for LoreLM.
    pub fn resolve() -> Result<Self> {
        let dirs = BaseDirs::new().ok_or(StorageError::DirectoryResolution)?;
        let state_dir = dirs
            .state_dir()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| dirs.data_local_dir().join("state"))
            .join("LoreLM");
        Ok(Self {
            config_dir: dirs.config_dir().join("LoreLM"),
            data_dir: dirs.data_dir().join("LoreLM"),
            cache_dir: dirs.cache_dir().join("LoreLM"),
            state_dir,
        })
    }

    /// Builds paths from explicit roots. Useful for tests and embedded launches.
    pub fn from_roots(
        config_dir: impl Into<PathBuf>,
        data_dir: impl Into<PathBuf>,
        cache_dir: impl Into<PathBuf>,
        state_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            config_dir: config_dir.into(),
            data_dir: data_dir.into(),
            cache_dir: cache_dir.into(),
            state_dir: state_dir.into(),
        }
    }

    /// Ensures all top-level directories exist.
    pub fn ensure_all(&self) -> Result<()> {
        std::fs::create_dir_all(&self.config_dir)?;
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(&self.cache_dir)?;
        std::fs::create_dir_all(&self.state_dir)?;
        Ok(())
    }

    /// Returns the config directory.
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    /// Returns the data directory.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Returns the cache directory.
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// Returns the state directory.
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    /// Returns the global config file path.
    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    /// Returns the SQLite database file path.
    pub fn database_file(&self) -> PathBuf {
        self.data_dir.join("app.db")
    }
}
