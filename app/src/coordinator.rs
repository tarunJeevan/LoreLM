use anyhow::Context;
use lorelm_core::{AppError, AppEvent, Command, Message, MessageRole, MessageStatus};
use storage::Storage;
use tokio::sync::mpsc;

pub struct Coordinator {
    storage: Storage,
    command_rx: mpsc::Receiver<Command>,
    event_tx: mpsc::Sender<AppEvent>,
}

impl Coordinator {
    pub fn new(
        storage: Storage,
        command_rx: mpsc::Receiver<Command>,
        event_tx: mpsc::Sender<AppEvent>,
    ) -> Self {
        Self {
            storage,
            command_rx,
            event_tx,
        }
    }

    pub fn run(mut self) -> anyhow::Result<()> {
        while let Some(command) = self.command_rx.blocking_recv() {
            match command {
                Command::SubmitPrompt {
                    conversation_id,
                    content,
                } => {
                    if let Err(error) = self.persist_stub_exchange(&conversation_id, &content) {
                        let _ = self
                            .event_tx
                            .blocking_send(AppEvent::Error(AppError::new(error.to_string())));
                    }
                }
                Command::Quit => break,
                Command::CancelGeneration => {
                    let _ = self.event_tx.blocking_send(AppEvent::GenerationCancelled);
                }
                Command::DismissError
                | Command::ScrollTranscript { .. }
                | Command::SetFocus(_)
                | Command::OpenScreen(_) => {}
            }
        }

        Ok(())
    }

    fn persist_stub_exchange(
        &self,
        conversation_id: &lorelm_core::ConversationId,
        content: &str,
    ) -> anyhow::Result<()> {
        let next_sequence = self
            .storage
            .next_message_sequence(conversation_id)
            .context("failed to load next message sequence")?;

        let user_message = Message::new(
            *conversation_id,
            next_sequence,
            MessageRole::User,
            content.to_owned(),
            MessageStatus::Complete,
        );
        self.storage
            .insert_message(&user_message)
            .context("failed to persist user message")?;
        let _ = self
            .event_tx
            .blocking_send(AppEvent::MessageAppended(user_message));

        let assistant_message = Message::new(
            *conversation_id,
            next_sequence + 1,
            MessageRole::Assistant,
            format!("Stub response: {content}"),
            MessageStatus::Complete,
        );
        self.storage
            .insert_message(&assistant_message)
            .context("failed to persist assistant message")?;
        let _ = self
            .event_tx
            .blocking_send(AppEvent::MessageAppended(assistant_message));

        Ok(())
    }
}
