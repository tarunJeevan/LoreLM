//! Terminal UI and event loop.

use std::time::Duration;

use anyhow::Context;
use crossterm::{
    event::{
        self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
        supports_keyboard_enhancement,
    },
};
use lorelm_core::{AppEvent, AppState, Command, GenerationState, MessageRole, Panel};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Color, Modifier, Style},
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
    let mut input_state = InputState::default();

    loop {
        while let Ok(event) = event_rx.try_recv() {
            apply_event(&mut state, event);
        }

        terminal.draw(|frame| render(frame, &state, &prompt))?;

        if event::poll(Duration::from_millis(50)).context("failed to poll terminal events")?
            && let Event::Key(key) = event::read().context("failed to read terminal event")?
            && handle_key(key, &mut input_state, &mut prompt, &mut state, &command_tx).await?
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
    input_state: &mut InputState,
    prompt: &mut String,
    state: &mut AppState,
    command_tx: &mpsc::Sender<Command>,
) -> anyhow::Result<bool> {
    if key.kind != KeyEventKind::Press {
        return Ok(false);
    }

    let had_pending_escape = input_state.pending_escape;
    input_state.pending_escape = false;

    if is_quit_key(key) || (had_pending_escape && is_plain_char(key, 'q')) {
        let _ = command_tx.send(Command::Quit).await;
        return Ok(true);
    }

    let prompt_editable = state.ui.focused_panel == Panel::Prompt
        && matches!(
            state.generation,
            GenerationState::Idle | GenerationState::Failed(_)
        );

    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if matches!(
                state.generation,
                GenerationState::Preparing
                    | GenerationState::Retrieving
                    | GenerationState::Generating
            ) {
                state.generation = GenerationState::Cancelling;
                let _ = command_tx.send(Command::CancelGeneration).await;
            }
        }
        KeyCode::Enter if prompt_editable && key.modifiers.contains(KeyModifiers::SHIFT) => {
            prompt.push('\n');
        }
        KeyCode::Enter => {
            if prompt_editable
                && key.modifiers.is_empty()
                && let Some(conversation) = &state.conversation.active_conversation
            {
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
        KeyCode::Backspace if prompt_editable => {
            prompt.pop();
        }
        KeyCode::Char(character)
            if prompt_editable
                && (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT) =>
        {
            prompt.push(character);
        }
        KeyCode::Tab if key.modifiers.is_empty() => match state.ui.focused_panel {
            Panel::Prompt => {
                if prompt_editable {
                    prompt.push('\t')
                }
            }
            Panel::Sidebar | Panel::Transcript | Panel::Status => {
                state.ui.focused_panel = next_focus(state.ui.focused_panel)
            }
        },
        KeyCode::BackTab => match state.ui.focused_panel {
            Panel::Prompt => {
                if prompt_editable && prompt.ends_with('\t') {
                    prompt.pop();
                }
            }
            Panel::Sidebar | Panel::Status | Panel::Transcript => {
                state.ui.focused_panel = previous_focus(state.ui.focused_panel)
            }
        },
        KeyCode::Esc => {
            input_state.pending_escape = true;
            state.ui.focused_panel = match state.ui.focused_panel {
                Panel::Prompt => Panel::Sidebar,
                Panel::Transcript | Panel::Sidebar | Panel::Status => Panel::Prompt,
            };
        }
        KeyCode::PageUp if state.ui.focused_panel == Panel::Transcript => {
            state.conversation.scroll_offset = state.conversation.scroll_offset.saturating_add(3);
        }
        KeyCode::PageDown if state.ui.focused_panel == Panel::Transcript => {
            state.conversation.scroll_offset = state.conversation.scroll_offset.saturating_sub(3);
        }
        KeyCode::Up if state.ui.focused_panel == Panel::Transcript => {
            state.conversation.scroll_offset = state.conversation.scroll_offset.saturating_add(1);
        }
        KeyCode::Down if state.ui.focused_panel == Panel::Transcript => {
            state.conversation.scroll_offset = state.conversation.scroll_offset.saturating_sub(1);
        }
        _ => {}
    }

    Ok(false)
}

fn is_quit_key(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char(character) if character.eq_ignore_ascii_case(&'q'))
        && key.modifiers.contains(KeyModifiers::CONTROL)
}

fn is_plain_char(key: KeyEvent, expected: char) -> bool {
    matches!(key.code, KeyCode::Char(character) if character.eq_ignore_ascii_case(&expected))
        && key.modifiers.is_empty()
}

fn next_focus(panel: Panel) -> Panel {
    match panel {
        Panel::Sidebar => Panel::Transcript,
        Panel::Transcript => Panel::Prompt,
        Panel::Prompt | Panel::Status => Panel::Sidebar,
    }
}

fn previous_focus(panel: Panel) -> Panel {
    match panel {
        Panel::Sidebar => Panel::Prompt,
        Panel::Transcript => Panel::Sidebar,
        Panel::Prompt | Panel::Status => Panel::Transcript,
    }
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
            .block(panel_block(
                "Sidebar",
                Panel::Sidebar,
                state.ui.focused_panel,
            ))
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
            .block(panel_block(
                "Chat",
                Panel::Transcript,
                state.ui.focused_panel,
            ))
            .wrap(Wrap { trim: false }),
        body[1],
    );

    let prompt_title = match state.generation {
        GenerationState::Idle | GenerationState::Failed(_) => "Prompt",
        _ => "Prompt (locked)",
    };
    frame.render_widget(
        Paragraph::new(prompt)
            .block(panel_block(
                prompt_title,
                Panel::Prompt,
                state.ui.focused_panel,
            ))
            .wrap(Wrap { trim: false }),
        root[2],
    );
    if state.ui.focused_panel == Panel::Prompt
        && matches!(
            state.generation,
            GenerationState::Idle | GenerationState::Failed(_)
        )
        && let Some(position) = prompt_cursor_position(prompt, root[2])
    {
        frame.set_cursor_position(position);
    }

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

fn panel_block(title: impl Into<String>, panel: Panel, focused_panel: Panel) -> Block<'static> {
    let focused = panel == focused_panel;
    let title = if focused {
        format!("> {}", title.into())
    } else {
        title.into()
    };
    let style = if focused {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    Block::default()
        .title(title)
        .title_style(style)
        .border_style(style)
        .borders(Borders::ALL)
}

fn prompt_cursor_position(prompt: &str, area: Rect) -> Option<Position> {
    let width = usize::from(area.width.saturating_sub(2));
    let height = usize::from(area.height.saturating_sub(2));
    if width == 0 || height == 0 {
        return None;
    }

    let mut visual_line = 0_usize;
    let mut visual_column = 0_usize;
    for line in prompt.split('\n') {
        let line_width = line.chars().count();
        visual_column = line_width % width;
        visual_line = visual_line.saturating_add(line_width / width);
    }
    visual_line = visual_line.saturating_add(prompt.matches('\n').count());

    Some(Position {
        x: area.x + 1 + visual_column.min(width.saturating_sub(1)) as u16,
        y: area.y + 1 + visual_line.min(height.saturating_sub(1)) as u16,
    })
}

#[derive(Default)]
struct InputState {
    pending_escape: bool,
}

struct TerminalSession {
    terminal: Terminal<CrosstermBackend<std::io::Stdout>>,
    keyboard_enhancement_enabled: bool,
}

impl TerminalSession {
    fn start() -> anyhow::Result<Self> {
        enable_raw_mode().context("failed to enable raw mode")?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen).context("failed to enter alternate screen")?;
        let keyboard_enhancement_enabled = supports_keyboard_enhancement().unwrap_or(false)
            && execute!(
                stdout,
                PushKeyboardEnhancementFlags(
                    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                        | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                )
            )
            .is_ok();
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend).context("failed to initialize terminal")?;
        Ok(Self {
            terminal,
            keyboard_enhancement_enabled,
        })
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
        if self.keyboard_enhancement_enabled {
            let _ = execute!(self.terminal.backend_mut(), PopKeyboardEnhancementFlags);
        }
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
        let _ = self.terminal.show_cursor();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lorelm_core::{
        Conversation, ConversationState, DocumentPanelState, ModeDefinition, ModelPanelState,
        UiState, Workspace, WorkspaceId, WorkspaceState,
    };

    #[test]
    fn forward_focus_cycles_through_interactive_panels() {
        assert_eq!(next_focus(Panel::Sidebar), Panel::Transcript);
        assert_eq!(next_focus(Panel::Transcript), Panel::Prompt);
        assert_eq!(next_focus(Panel::Prompt), Panel::Sidebar);
        assert_eq!(next_focus(Panel::Status), Panel::Sidebar);
    }

    #[test]
    fn reverse_focus_cycles_through_interactive_panels() {
        assert_eq!(previous_focus(Panel::Sidebar), Panel::Prompt);
        assert_eq!(previous_focus(Panel::Transcript), Panel::Sidebar);
        assert_eq!(previous_focus(Panel::Prompt), Panel::Transcript);
        assert_eq!(previous_focus(Panel::Status), Panel::Transcript);
    }

    #[tokio::test]
    async fn tab_in_prompt_cycles_focus() {
        let mut state = test_state();
        let (command_tx, _command_rx) = mpsc::channel(1);
        let mut prompt = String::new();
        let mut input_state = InputState::default();

        handle_key(
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            &mut input_state,
            &mut prompt,
            &mut state,
            &command_tx,
        )
        .await
        .expect("key handling succeeds");

        assert_eq!(prompt, "");
        assert_eq!(state.ui.focused_panel, Panel::Sidebar);
    }

    #[tokio::test]
    async fn backtab_in_prompt_cycles_focus_backwards() {
        let mut state = test_state();
        let (command_tx, _command_rx) = mpsc::channel(1);
        let mut prompt = String::new();
        let mut input_state = InputState::default();

        handle_key(
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
            &mut input_state,
            &mut prompt,
            &mut state,
            &command_tx,
        )
        .await
        .expect("key handling succeeds");

        assert_eq!(state.ui.focused_panel, Panel::Transcript);
    }

    #[tokio::test]
    async fn typing_is_ignored_when_prompt_is_not_focused() {
        let mut state = test_state();
        state.ui.focused_panel = Panel::Transcript;
        let (command_tx, _command_rx) = mpsc::channel(1);
        let mut prompt = String::new();
        let mut input_state = InputState::default();

        handle_key(
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
            &mut input_state,
            &mut prompt,
            &mut state,
            &command_tx,
        )
        .await
        .expect("key handling succeeds");

        assert_eq!(prompt, "");
    }

    #[tokio::test]
    async fn ctrl_q_variants_quit() {
        for key in [
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::CONTROL),
            KeyEvent::new(
                KeyCode::Char('q'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
        ] {
            let mut state = test_state();
            let (command_tx, mut command_rx) = mpsc::channel(1);
            let mut prompt = String::new();
            let mut input_state = InputState::default();

            let should_quit =
                handle_key(key, &mut input_state, &mut prompt, &mut state, &command_tx)
                    .await
                    .expect("key handling succeeds");

            assert!(should_quit);
            assert_eq!(command_rx.try_recv(), Ok(Command::Quit));
        }
    }

    #[tokio::test]
    async fn esc_then_q_quits_as_fallback() {
        let mut state = test_state();
        let (command_tx, mut command_rx) = mpsc::channel(1);
        let mut prompt = String::new();
        let mut input_state = InputState::default();

        let should_quit = handle_key(
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            &mut input_state,
            &mut prompt,
            &mut state,
            &command_tx,
        )
        .await
        .expect("key handling succeeds");
        assert!(!should_quit);

        let should_quit = handle_key(
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
            &mut input_state,
            &mut prompt,
            &mut state,
            &command_tx,
        )
        .await
        .expect("key handling succeeds");

        assert!(should_quit);
        assert_eq!(command_rx.try_recv(), Ok(Command::Quit));
    }

    #[tokio::test]
    async fn alt_tab_is_ignored_by_lorelm() {
        let mut state = test_state();
        let (command_tx, _command_rx) = mpsc::channel(1);
        let mut prompt = String::new();
        let mut input_state = InputState::default();

        let should_quit = handle_key(
            KeyEvent::new(KeyCode::Tab, KeyModifiers::ALT),
            &mut input_state,
            &mut prompt,
            &mut state,
            &command_tx,
        )
        .await
        .expect("key handling succeeds");

        assert!(!should_quit);
        assert_eq!(prompt, "");
        assert_eq!(state.ui.focused_panel, Panel::Prompt);
    }

    #[tokio::test]
    async fn alt_enter_inserts_prompt_newline() {
        let mut state = test_state();
        let (command_tx, _command_rx) = mpsc::channel(1);
        let mut prompt = String::new();
        let mut input_state = InputState::default();

        handle_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT),
            &mut input_state,
            &mut prompt,
            &mut state,
            &command_tx,
        )
        .await
        .expect("key handling succeeds");

        assert_eq!(prompt, "\n");
    }

    fn test_state() -> AppState {
        let workspace_id = WorkspaceId::new();
        AppState {
            workspace: WorkspaceState {
                active_workspace: Some(Workspace {
                    id: workspace_id,
                    name: "default".to_owned(),
                    root_path: None,
                    created_at: String::new(),
                    updated_at: String::new(),
                }),
            },
            conversation: ConversationState {
                active_conversation: Some(Conversation {
                    id: lorelm_core::ConversationId::new(),
                    workspace_id,
                    parent_conversation_id: None,
                    title: Some("main".to_owned()),
                    active_mode_id: lorelm_core::ModeId::named("freeform"),
                    created_at: String::new(),
                    updated_at: String::new(),
                }),
                messages: Vec::new(),
                streaming_response: None,
                scroll_offset: 0,
            },
            documents: DocumentPanelState::default(),
            models: ModelPanelState::default(),
            generation: GenerationState::Idle,
            active_mode: ModeDefinition::freeform(),
            ui: UiState::default(),
            available_ram_bytes: 0,
        }
    }
}
