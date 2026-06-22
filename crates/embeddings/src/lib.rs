//! Embedding crate stub for Phase 0.

/// Embedding error.
#[derive(Debug, thiserror::Error)]
pub enum EmbeddingError {
    /// Embeddings are not implemented yet.
    #[error("embeddings are not implemented yet")]
    NotImplemented,
}
