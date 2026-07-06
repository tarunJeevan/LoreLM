//! llama.cpp backend implementation.
//!
//! This crate owns all `llama-cpp-2` types and exposes only the workspace's
//! backend-neutral `InferenceBackend` contract.

use std::{num::NonZeroU32, time::Instant};

use inference::{
    CancellationToken, GenerateRequest, GenerationEvent, GenerationSummary, InferenceBackend,
    InferenceError, ModelSpec, Result, RuntimeModelConfig, StopReason, TokenSink,
};
use llama_cpp_2::{
    context::params::LlamaContextParams,
    llama_backend::LlamaBackend as LlamaCppBackend,
    llama_batch::LlamaBatch,
    model::{AddBos, LlamaModel, params::LlamaModelParams},
    sampling::LlamaSampler,
};
use lorelm_core::ModelId;

/// llama.cpp backend implementation.
#[derive(Debug, Default)]
pub struct LlamaBackend {
    backend: Option<LlamaCppBackend>,
    model: Option<LlamaModel>,
    current_model: Option<ModelId>,
    runtime_config: Option<RuntimeModelConfig>,
}

impl LlamaBackend {
    /// Creates an unloaded backend.
    pub fn new() -> Self {
        Self::default()
    }
}

impl InferenceBackend for LlamaBackend {
    fn load_model(&mut self, spec: ModelSpec, config: RuntimeModelConfig) -> Result<()> {
        if !spec.path.exists() {
            return Err(InferenceError::ModelLoad(format!(
                "model path does not exist: {}",
                spec.path.display()
            )));
        }

        let mut backend = LlamaCppBackend::init()
            .map_err(|error| InferenceError::ModelLoad(error.to_string()))?;
        backend.void_logs();
        let model_params = LlamaModelParams::default()
            .with_use_mmap(config.use_mmap)
            .with_use_mlock(config.use_mlock);
        let model = LlamaModel::load_from_file(&backend, &spec.path, &model_params)
            .map_err(|error| InferenceError::ModelLoad(error.to_string()))?;

        self.backend = Some(backend);
        self.model = Some(model);
        self.current_model = Some(spec.id);
        self.runtime_config = Some(config);
        Ok(())
    }

    fn unload_model(&mut self) -> Result<()> {
        self.model = None;
        self.backend = None;
        self.current_model = None;
        self.runtime_config = None;
        Ok(())
    }

    fn generate_stream(
        &mut self,
        request: GenerateRequest,
        sink: TokenSink,
        cancel: CancellationToken,
    ) -> Result<GenerationSummary> {
        let model_id = self.current_model.ok_or(InferenceError::NoModelLoaded)?;
        let model = self.model.as_ref().ok_or(InferenceError::NoModelLoaded)?;
        let backend = self.backend.as_ref().ok_or(InferenceError::NoModelLoaded)?;
        let config = self
            .runtime_config
            .clone()
            .ok_or(InferenceError::NoModelLoaded)?;
        let started_at = Instant::now();

        if cancel.is_cancelled() {
            return Ok(cancelled_summary(model_id, 0, 0, started_at));
        }

        let prompt_tokens = model
            .str_to_token(&request.prompt, AddBos::Always)
            .map_err(|error| InferenceError::Generation(error.to_string()))?;
        if prompt_tokens.len() >= config.context_size {
            return Err(InferenceError::Generation(format!(
                "prompt uses {} tokens, exceeding configured context size {}",
                prompt_tokens.len(),
                config.context_size
            )));
        }

        let context_params = LlamaContextParams::default()
            .with_n_ctx(non_zero_u32(config.context_size, "context_size")?)
            .with_n_batch(to_u32(config.batch_size, "batch_size")?)
            .with_n_ubatch(to_u32(config.ubatch_size, "ubatch_size")?)
            .with_n_threads(to_i32(config.threads, "threads")?)
            .with_n_threads_batch(to_i32(config.threads, "threads")?);
        let mut context = model
            .new_context(backend, context_params)
            .map_err(|error| InferenceError::Generation(error.to_string()))?;

        let mut prompt_batch = LlamaBatch::new(prompt_tokens.len().max(1), 1);
        prompt_batch
            .add_sequence(&prompt_tokens, 0, false)
            .map_err(|error| InferenceError::Generation(error.to_string()))?;
        context
            .decode(&mut prompt_batch)
            .map_err(|error| InferenceError::Generation(error.to_string()))?;

        let mut sampler = sampler_for(&request);
        sampler.accept_many(&prompt_tokens);

        let mut generated_tokens = 0usize;
        let mut stop_reason = StopReason::MaxTokens;
        let context_tokens_used = prompt_tokens.len();
        let max_decode_tokens = request.generation.max_tokens.min(
            config
                .context_size
                .saturating_sub(context_tokens_used)
                .saturating_sub(1),
        );

        for generated_index in 0..max_decode_tokens {
            if cancel.is_cancelled() {
                return Ok(cancelled_summary(
                    model_id,
                    generated_tokens,
                    context_tokens_used,
                    started_at,
                ));
            }

            let token = sampler.sample(&context, -1);
            if model.is_eog_token(token) {
                stop_reason = StopReason::EndOfSequence;
                break;
            }

            sampler.accept(token);
            generated_tokens += 1;
            let delta = model
                .token_to_piece_bytes(token, 8, false, None)
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .map_err(|error| InferenceError::Generation(error.to_string()))?;
            if !delta.is_empty() {
                let _ = sink.send(GenerationEvent::TokenDelta(delta));
            }

            let position = context_tokens_used
                .checked_add(generated_index)
                .and_then(|value| i32::try_from(value).ok())
                .ok_or_else(|| InferenceError::Generation("token position overflow".to_owned()))?;
            let mut next_batch = LlamaBatch::new(1, 1);
            next_batch
                .add(token, position, &[0], true)
                .map_err(|error| InferenceError::Generation(error.to_string()))?;
            context
                .decode(&mut next_batch)
                .map_err(|error| InferenceError::Generation(error.to_string()))?;
        }

        Ok(summary(
            model_id,
            generated_tokens,
            context_tokens_used,
            stop_reason,
            started_at,
        ))
    }

    fn current_model(&self) -> Option<ModelId> {
        self.current_model
    }

    fn estimate_tokens(&self, text: &str) -> Result<usize> {
        if let Some(model) = &self.model {
            return model
                .str_to_token(text, AddBos::Always)
                .map(|tokens| tokens.len())
                .map_err(|error| InferenceError::Generation(error.to_string()));
        }
        Ok(text.split_whitespace().count())
    }
}

fn sampler_for(request: &GenerateRequest) -> LlamaSampler {
    if request.generation.temperature <= 0.0 {
        return LlamaSampler::greedy();
    }

    LlamaSampler::chain_simple([
        LlamaSampler::penalties(64, request.generation.repeat_penalty, 0.0, 0.0),
        LlamaSampler::top_p(request.generation.top_p, 1),
        LlamaSampler::temp(request.generation.temperature),
        LlamaSampler::dist(0),
    ])
}

fn non_zero_u32(value: usize, name: &str) -> Result<Option<NonZeroU32>> {
    let value = to_u32(value, name)?;
    NonZeroU32::new(value)
        .map(Some)
        .ok_or_else(|| InferenceError::Generation(format!("{name} must be greater than zero")))
}

fn to_u32(value: usize, name: &str) -> Result<u32> {
    u32::try_from(value)
        .map_err(|_| InferenceError::Generation(format!("{name} is too large: {value}")))
}

fn to_i32(value: usize, name: &str) -> Result<i32> {
    i32::try_from(value)
        .map_err(|_| InferenceError::Generation(format!("{name} is too large: {value}")))
}

fn cancelled_summary(
    model_id: ModelId,
    generated_tokens: usize,
    context_tokens_used: usize,
    started_at: Instant,
) -> GenerationSummary {
    summary(
        model_id,
        generated_tokens,
        context_tokens_used,
        StopReason::Cancelled,
        started_at,
    )
}

fn summary(
    model_id: ModelId,
    generated_tokens: usize,
    context_tokens_used: usize,
    stop_reason: StopReason,
    started_at: Instant,
) -> GenerationSummary {
    let duration = started_at.elapsed();
    let duration_ms = duration.as_millis().try_into().unwrap_or(u64::MAX);
    let tokens_per_second = if duration.as_secs_f32() > 0.0 {
        generated_tokens as f32 / duration.as_secs_f32()
    } else {
        0.0
    };

    GenerationSummary {
        model_id: Some(model_id),
        generated_tokens,
        context_tokens_used,
        tokens_per_second,
        stop_reason,
        duration_ms,
    }
}
