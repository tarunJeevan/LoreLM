//! Document ingestion crate stub for Phase 0.

/// Document ingestion error.
#[derive(Debug, thiserror::Error)]
pub enum DocIngestError {
    /// Ingestion is not implemented yet.
    #[error("document ingestion is not implemented yet")]
    NotImplemented,
}
