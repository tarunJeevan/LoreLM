//! Backend-neutral inference API stub for Phase 0.

/// Inference error.
#[derive(Debug, thiserror::Error)]
pub enum InferenceError {
    /// Inference is not implemented yet.
    #[error("inference is not implemented yet")]
    NotImplemented,
}
