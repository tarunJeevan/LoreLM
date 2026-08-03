use std::{path::PathBuf, sync::mpsc};

use inference::{
    CancellationToken, GenerateRequest, GenerationEvent, InferenceBackend, ModelSpec,
    RuntimeModelConfig, StopReason,
};
use lorelm_core::{ConversationId, GenerationConfig, ModeId, ModelId};

#[test]
#[ignore = "requires a local GGUF model at $LORELM_TEST_MODEL_PATH"]
fn load_model_generate_streams_tokens_and_unloads() {
    let path = PathBuf::from(
        std::env::var("LORELM_TEST_MODEL_PATH")
            .expect("LORELM_TEST_MODEL_PATH must point to a GGUF model"),
    );
    let mut backend = llama_backend::LlamaBackend::new();
    let model_id = ModelId::new();
    backend
        .load_model(
            ModelSpec {
                id: model_id,
                display_name: "test model".to_owned(),
                size_bytes: path.metadata().ok().map(|metadata| metadata.len()),
                path,
            },
            RuntimeModelConfig {
                context_size: 512,
                threads: 2,
                batch_size: 128,
                ubatch_size: 64,
                use_mmap: true,
                use_mlock: false,
            },
        )
        .expect("model loads");

    let (tx, rx) = mpsc::channel();
    let summary = backend
        .generate_stream(
            GenerateRequest {
                conversation_id: ConversationId::new(),
                prompt: "Write one short sentence about local-first software.".to_owned(),
                mode_id: ModeId::named("freeform"),
                generation: GenerationConfig {
                    temperature: 0.0,
                    top_p: 1.0,
                    repeat_penalty: 1.0,
                    max_tokens: 8,
                },
            },
            tx,
            CancellationToken::new(),
        )
        .expect("generation succeeds");

    let deltas = rx
        .try_iter()
        .filter(|event| matches!(event, GenerationEvent::TokenDelta(_)))
        .count();
    assert!(deltas > 0, "expected at least one streamed token delta");
    assert_eq!(summary.model_id, Some(model_id));
    assert_ne!(summary.stop_reason, StopReason::Cancelled);

    backend.unload_model().expect("model unloads");
    assert_eq!(backend.current_model(), None);
}
