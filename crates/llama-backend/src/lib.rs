//! llama.cpp backend boundary.
//!
//! The real `llama-cpp-2` integration will live entirely in this crate. Phase 1
//! wiring can already exercise model loading and worker ownership without
//! leaking llama-specific types into the rest of the workspace.

use inference::{
    CancellationToken, GenerateRequest, GenerationSummary, InferenceBackend, InferenceError,
    ModelSpec, Result, RuntimeModelConfig, StopReason, TokenSink,
};
use lorelm_core::ModelId;

/// llama.cpp backend implementation.
#[derive(Debug, Default)]
pub struct LlamaBackend {
    current_model: Option<ModelId>,
}

impl LlamaBackend {
    /// Creates an unloaded backend.
    pub fn new() -> Self {
        Self::default()
    }
}

impl InferenceBackend for LlamaBackend {
    fn load_model(&mut self, spec: ModelSpec, _config: RuntimeModelConfig) -> Result<()> {
        if !spec.path.exists() {
            return Err(InferenceError::ModelLoad(format!(
                "model path does not exist: {}",
                spec.path.display()
            )));
        }
        self.current_model = Some(spec.id);
        Ok(())
    }

    fn unload_model(&mut self) -> Result<()> {
        self.current_model = None;
        Ok(())
    }

    fn generate_stream(
        &mut self,
        request: GenerateRequest,
        _sink: TokenSink,
        cancel: CancellationToken,
    ) -> Result<GenerationSummary> {
        let model_id = self.current_model.ok_or(InferenceError::NoModelLoaded)?;
        if cancel.is_cancelled() {
            return Ok(GenerationSummary {
                model_id: Some(model_id),
                generated_tokens: 0,
                context_tokens_used: self.estimate_tokens(&request.prompt)?,
                tokens_per_second: 0.0,
                stop_reason: StopReason::Cancelled,
                duration_ms: 0,
            });
        }

        Err(InferenceError::Generation(
            "llama.cpp generation is not implemented yet".to_owned(),
        ))
    }

    fn current_model(&self) -> Option<ModelId> {
        self.current_model
    }

    fn estimate_tokens(&self, text: &str) -> Result<usize> {
        Ok(text.split_whitespace().count())
    }
}
