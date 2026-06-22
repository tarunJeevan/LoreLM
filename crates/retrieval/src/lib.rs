//! Retrieval crate stub for Phase 0.

/// Retrieval error.
#[derive(Debug, thiserror::Error)]
pub enum RetrievalError {
    /// Retrieval is not implemented yet.
    #[error("retrieval is not implemented yet")]
    NotImplemented,
}
