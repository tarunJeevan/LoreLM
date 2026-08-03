//! Model management and Phase 1 resource planning.

use std::{fs, path::Path};

use lorelm_core::{ModelSpec, RuntimeModelConfig};

/// Convenient result type for model management operations.
pub type Result<T> = std::result::Result<T, ModelManagerError>;

/// Model management error.
#[derive(Debug, thiserror::Error)]
pub enum ModelManagerError {
    /// The operating system did not expose a usable memory estimate.
    #[error("could not estimate available memory")]
    AvailableMemoryUnavailable,
    /// Filesystem operation failed.
    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),
}

/// Plans runtime model settings from model metadata and available memory.
#[derive(Debug, Clone, Copy, Default)]
pub struct ResourcePlanner;

impl ResourcePlanner {
    /// Creates a new planner.
    pub fn new() -> Self {
        Self
    }

    /// Estimates currently available RAM in bytes.
    pub fn estimate_available_ram(&self) -> Result<u64> {
        estimate_available_ram()
    }

    /// Produces a conservative runtime config for a model load.
    pub fn plan(&self, model_spec: &ModelSpec, available_ram_bytes: u64) -> RuntimeModelConfig {
        let likely_tight = model_spec
            .size_bytes
            .is_some_and(|size_bytes| size_bytes.saturating_mul(2) > available_ram_bytes);

        RuntimeModelConfig {
            context_size: if likely_tight { 4096 } else { 8192 },
            threads: std::thread::available_parallelism().map_or(4, usize::from),
            batch_size: if likely_tight { 256 } else { 512 },
            ubatch_size: 128,
            use_mmap: true,
            use_mlock: false,
        }
    }
}

fn estimate_available_ram() -> Result<u64> {
    available_ram_from_proc_meminfo(Path::new("/proc/meminfo"))
        .map_err(ModelManagerError::from)?
        .ok_or(ModelManagerError::AvailableMemoryUnavailable)
}

fn available_ram_from_proc_meminfo(path: &Path) -> std::io::Result<Option<u64>> {
    let content = fs::read_to_string(path)?;
    Ok(parse_mem_available_kib(&content).map(|kib| kib.saturating_mul(1024)))
}

fn parse_mem_available_kib(content: &str) -> Option<u64> {
    content.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        if key != "MemAvailable" {
            return None;
        }
        value.split_whitespace().next()?.parse().ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lorelm_core::ModelId;
    use std::path::PathBuf;

    #[test]
    fn parses_mem_available_from_proc_meminfo() {
        let content = "MemTotal:       1000 kB\nMemAvailable:    42 kB\n";

        assert_eq!(parse_mem_available_kib(content), Some(42));
    }

    #[test]
    fn tight_models_get_smaller_context_and_batch() {
        let planner = ResourcePlanner::new();
        let spec = ModelSpec {
            id: ModelId::new(),
            display_name: "model.gguf".to_owned(),
            path: PathBuf::from("model.gguf"),
            size_bytes: Some(9_000_000_000),
        };

        let plan = planner.plan(&spec, 10_000_000_000);

        assert_eq!(plan.context_size, 4096);
        assert_eq!(plan.batch_size, 256);
    }
}
