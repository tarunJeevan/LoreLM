//! Terminal UI and event loop.

use std::time::Duration;

use anyhow::Context;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use lorelm_core::{AppEvent, AppState, Command, GenerationState, MessageRole, Panel};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use tokio::sync::mpsc;

/// Runs the terminal UI until the user quits.
pub async fn run(
    initial_state: AppState,
    command_tx: mpsc::Sender<Command>,
    mut event_rx: mpsc::Receiver<AppEvent>,
) -> anyhow::Result<()> {
    let mut terminal = TerminalSession::start()?;
    let mut state = initial_state;
    let mut prompt = String::new();

    loop {
        while let Ok(event) = event_rx.try_recv() {
            apply_event(&mut state, event);
        }

        terminal.draw(|frame| render(frame, &state, &prompt))?;

        if event::poll(Duration::from_millis(50)).context("failed to poll terminal events")?
            && let Event::Key(key) = event::read().context("failed to read terminal event")?
            && handle_key(key, &mut prompt, &mut state, &command_tx).await?
        {
            break;
        }
    }

    Ok(())
}

fn apply_event(state: &mut AppState, event: AppEvent) {
    match event {
        AppEvent::MessageAppended(message) => {
            state.conversation.messages.push(message);
        }
        AppEvent::GenerationCancelled => {
            state.conversation.streaming_response = None;
            state.generation = GenerationState::Idle;
        }
        AppEvent::Error(error) => {
            state.conversation.streaming_response = None;
            state.generation = GenerationState::Failed(error.message.clone());
            state.ui.error = Some(error);
        }
        AppEvent::TokenDelta(delta) => {
            let response = state
                .conversation
                .streaming_response
                .get_or_insert_with(String::new);
            response.push_str(&delta);
            state.generation = GenerationState::Generating;
        }
        AppEvent::GenerationFinished(_) => {
            state.conversation.streaming_response = None;
            state.generation = GenerationState::Idle;
        }
        AppEvent::Tick
        | AppEvent::DocumentImportProgress(_)
        | AppEvent::ModelDownloadProgress(_)
        | AppEvent::ModelLoaded(_)
        | AppEvent::RetrievalFinished(_) => {}
    }
}

async fn handle_key(
    key: KeyEvent,
    prompt: &mut String,
    state: &mut AppState,
    command_tx: &mpsc::Sender<Command>,
) -> anyhow::Result<bool> {
    let prompt_editable = matches!(
        state.generation,
        GenerationState::Idle | GenerationState::Failed(_)
    );

    match (key.code, key.modifiers) {
        (KeyCode::Char('q'), KeyModifiers::CONTROL) => {
            let _ = command_tx.send(Command::Quit).await;
            return Ok(true);
        }
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            if matches!(state.generation, GenerationState::Generating) {
                state.generation = GenerationState::Cancelling;
                let _ = command_tx.send(Command::CancelGeneration).await;
            }
        }
        (KeyCode::Enter, KeyModifiers::SHIFT) if prompt_editable => {
            prompt.push('\n');
        }
        (KeyCode::Enter, _) => {
            if prompt_editable && let Some(conversation) = &state.conversation.active_conversation {
                let content = prompt.trim().to_owned();
                if !content.is_empty() {
                    state.generation = GenerationState::Preparing;
                    prompt.clear();
                    command_tx
                        .send(Command::SubmitPrompt {
                            conversation_id: conversation.id,
                            content,
                        })
                        .await
                        .context("failed to send prompt command")?;
                }
            }
        }
        (KeyCode::Backspace, _) if prompt_editable => {
            prompt.pop();
        }
        (KeyCode::Char(character), KeyModifiers::NONE | KeyModifiers::SHIFT) if prompt_editable => {
            prompt.push(character);
        }
        (KeyCode::Tab, KeyModifiers::NONE) if prompt_editable => {
            prompt.push('\t');
        }
        (KeyCode::BackTab, _) => {
            state.ui.focused_panel = match state.ui.focused_panel {
                Panel::Sidebar => Panel::Prompt,
                Panel::Transcript => Panel::Sidebar,
                Panel::Prompt => Panel::Transcript,
                Panel::Status => Panel::Prompt,
            };
        }
        (KeyCode::Esc, _) => {
            state.ui.focused_panel = match state.ui.focused_panel {
                Panel::Prompt => Panel::Transcript,
                Panel::Transcript | Panel::Sidebar | Panel::Status => Panel::Prompt,
            };
        }
        (KeyCode::Tab, _) => {
            state.ui.focused_panel = match state.ui.focused_panel {
                Panel::Sidebar => Panel::Transcript,
                Panel::Transcript => Panel::Prompt,
                Panel::Prompt => Panel::Sidebar,
                Panel::Status => Panel::Prompt,
            };
        }
        (KeyCode::PageUp, _) => {
            state.conversation.scroll_offset = state.conversation.scroll_offset.saturating_add(3);
        }
        (KeyCode::PageDown, _) => {
            state.conversation.scroll_offset = state.conversation.scroll_offset.saturating_sub(3);
        }
        (KeyCode::Up, _) => {
            state.conversation.scroll_offset = state.conversation.scroll_offset.saturating_add(1);
        }
        (KeyCode::Down, _) => {
            state.conversation.scroll_offset = state.conversation.scroll_offset.saturating_sub(1);
        }
        _ => {}
    }

    Ok(false)
}

fn render(frame: &mut ratatui::Frame<'_>, state: &AppState, prompt: &str) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(8),
            Constraint::Length(5),
            Constraint::Length(1),
        ])
        .split(frame.area());

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(26), Constraint::Min(20)])
        .split(root[1]);

    let workspace = state
        .workspace
        .active_workspace
        .as_ref()
        .map(|workspace| workspace.name.as_str())
        .unwrap_or("none");
    let model = state
        .models
        .active_model
        .as_ref()
        .map(|model| model.display_name.as_str())
        .unwrap_or("none");
    let header = Paragraph::new(format!(
        "Workspace: {workspace} | Mode: {} | Model: {model}",
        state.active_mode.name
    ));
    frame.render_widget(header, root[0]);

    let sidebar_lines = vec![
        Line::from(Span::styled(
            "Documents",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from("No documents imported"),
        Line::from(""),
        Line::from(Span::styled(
            "Sessions",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from("> main"),
    ];
    frame.render_widget(
        Paragraph::new(sidebar_lines)
            .block(Block::default().borders(Borders::RIGHT))
            .wrap(Wrap { trim: true }),
        body[0],
    );

    let mut transcript = Vec::new();
    for message in &state.conversation.messages {
        let role = match message.role {
            MessageRole::System => "System",
            MessageRole::User => "User",
            MessageRole::Assistant => "Assistant",
        };
        transcript.push(Line::from(Span::styled(
            format!("{role}:"),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        transcript.push(Line::from(message.content.as_str()));
        transcript.push(Line::from(""));
    }
    if let Some(streaming_response) = &state.conversation.streaming_response {
        transcript.push(Line::from(Span::styled(
            "Assistant:",
            Style::default().add_modifier(Modifier::BOLD),
        )));
        transcript.push(Line::from(streaming_response.as_str()));
        transcript.push(Line::from(""));
    }
    if transcript.is_empty() {
        transcript.push(Line::from("Type a prompt and press Enter."));
    }
    frame.render_widget(
        Paragraph::new(transcript)
            .block(Block::default().title("Chat").borders(Borders::LEFT))
            .wrap(Wrap { trim: false }),
        body[1],
    );

    let prompt_title = match state.generation {
        GenerationState::Idle | GenerationState::Failed(_) => "Prompt",
        _ => "Prompt (locked)",
    };
    frame.render_widget(
        Paragraph::new(prompt)
            .block(Block::default().title(prompt_title).borders(Borders::ALL))
            .wrap(Wrap { trim: false }),
        root[2],
    );

    let status = match &state.generation {
        GenerationState::Idle => "Idle".to_owned(),
        GenerationState::Preparing => "Preparing".to_owned(),
        GenerationState::Retrieving => "Retrieving".to_owned(),
        GenerationState::Generating => "Generating".to_owned(),
        GenerationState::Cancelling => "Cancelling".to_owned(),
        GenerationState::Failed(message) => format!("Error: {message}"),
    };
    frame.render_widget(Paragraph::new(format!("{status} | Ctrl+Q quit")), root[3]);
}

struct TerminalSession {
    terminal: Terminal<CrosstermBackend<std::io::Stdout>>,
}

impl TerminalSession {
    fn start() -> anyhow::Result<Self> {
        enable_raw_mode().context("failed to enable raw mode")?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen).context("failed to enter alternate screen")?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend).context("failed to initialize terminal")?;
        Ok(Self { terminal })
    }

    fn draw<F>(&mut self, render: F) -> anyhow::Result<()>
    where
        F: FnOnce(&mut ratatui::Frame<'_>),
    {
        self.terminal.draw(render)?;
        Ok(())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}
