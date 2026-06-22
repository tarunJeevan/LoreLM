//! llama.cpp backend crate stub for Phase 0.

/// llama backend error.
#[derive(Debug, thiserror::Error)]
pub enum LlamaBackendError {
    /// llama backend is not implemented yet.
    #[error("llama backend is not implemented yet")]
    NotImplemented,
}
