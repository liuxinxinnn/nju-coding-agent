mod input;
mod layout;
mod model;
mod overlay;
mod terminal;

use std::path::PathBuf;
use std::sync::{Arc, mpsc as std_mpsc};
use std::time::Duration;

use crossterm::event::{
    self, Event as TerminalEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};
use ratatui::Frame;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use tokio::sync::mpsc;

use crate::agent::{AgentEvent, AgentPhase};
use crate::tools::{ApprovalFn, default_registry_with_approval};
use crate::{Agent, Config, HttpLanguageModel, Result};

use input::{History, InputBuffer};
use layout::split;
use model::{
    ChatMessage, EventEntry, EventKind, MessageRole, summarize_arguments, summarize_result,
};
use terminal::TerminalGuard;

const POLL_INTERVAL: Duration = Duration::from_millis(16);
const MAX_MESSAGES: usize = 240;
const MAX_EVENTS: usize = 500;

enum WorkerCommand {
    Chat(String),
    Shutdown,
}

enum WorkerEvent {
    Ready {
        tools: Vec<String>,
    },
    Started,
    Agent(AgentEvent),
    Done(std::result::Result<String, String>),
    Confirm {
        prompt: String,
        response: std_mpsc::Sender<bool>,
    },
    Fatal(String),
}

struct PendingConfirmation {
    prompt: String,
    response: std_mpsc::Sender<bool>,
}

struct App {
    workspace: PathBuf,
    model: String,
    input: InputBuffer,
    history: History,
    messages: Vec<ChatMessage>,
    events: Vec<EventEntry>,
    tools: Vec<String>,
    busy: bool,
    status: String,
    phase: AgentPhase,
    workspace_revision: u64,
    last_verified_revision: Option<u64>,
    project_kind: String,
    verification_command: Option<String>,
    should_quit: bool,
    show_help: bool,
    events_collapsed: bool,
    chat_scroll: u16,
    follow_chat: bool,
    chat_height: u16,
    pending_confirmation: Option<PendingConfirmation>,
}

impl App {
    fn new(workspace: PathBuf, model: String) -> Self {
        Self {
            workspace,
            model,
            input: InputBuffer::new(),
            history: History::default(),
            messages: vec![ChatMessage::new(
                MessageRole::System,
                "Coding Agent 已启动。输入任务后按 Enter 发送，F1 查看帮助。",
            )],
            events: vec![EventEntry::new(EventKind::Info, "正在初始化 Agent")],
            tools: Vec::new(),
            busy: false,
            status: "初始化".to_owned(),
            phase: AgentPhase::Done,
            workspace_revision: 0,
            last_verified_revision: None,
            project_kind: "待检测".to_owned(),
            verification_command: None,
            should_quit: false,
            show_help: false,
            events_collapsed: false,
            chat_scroll: 0,
            follow_chat: true,
            chat_height: 10,
            pending_confirmation: None,
        }
    }

    fn push_message(&mut self, role: MessageRole, content: impl Into<String>) {
        self.messages.push(ChatMessage::new(role, content));
        if self.messages.len() > MAX_MESSAGES {
            self.messages.remove(0);
        }
        self.follow_chat = true;
    }

    fn push_event(&mut self, kind: EventKind, text: impl Into<String>) {
        self.events.push(EventEntry::new(kind, text));
        if self.events.len() > MAX_EVENTS {
            self.events.remove(0);
        }
    }

    fn apply_worker_event(&mut self, event: WorkerEvent) {
        match event {
            WorkerEvent::Ready { tools } => {
                self.tools = tools;
                self.status = "就绪".to_owned();
                self.push_event(EventKind::Info, "Agent 初始化完成");
            }
            WorkerEvent::Started => {
                self.busy = true;
                self.status = "执行中".to_owned();
            }
            WorkerEvent::Agent(event) => match event {
                AgentEvent::ProjectDetected {
                    kind,
                    evidence,
                    verification_command,
                } => {
                    self.project_kind = kind.clone();
                    self.verification_command = verification_command.clone();
                    self.push_event(
                        EventKind::Detect,
                        format!(
                            "{kind} · {} · {}",
                            if evidence.is_empty() {
                                "无标志文件".to_owned()
                            } else {
                                evidence.join(", ")
                            },
                            verification_command.unwrap_or_else(|| "由模型选择验证".to_owned())
                        ),
                    );
                }
                AgentEvent::PhaseChanged { phase } => {
                    self.phase = phase;
                    self.status = phase_status(phase).to_owned();
                    self.push_event(
                        phase_event_kind(phase),
                        format!("进入 {} 阶段", phase.label()),
                    );
                }
                AgentEvent::PlanCreated { plan } => {
                    self.push_message(MessageRole::Plan, plan.clone());
                    self.push_event(
                        EventKind::Plan,
                        format!("计划 · {}", summarize_result(&plan)),
                    );
                }
                AgentEvent::Thinking { step, phase } => {
                    self.phase = phase;
                    self.status = format!("{} · step {step}", phase.label());
                    self.push_event(phase_event_kind(phase), format!("#{step} 模型思考"));
                }
                AgentEvent::ToolCall {
                    step,
                    phase,
                    name,
                    arguments,
                } => {
                    self.phase = phase;
                    self.status = format!("{} · {name}", phase.label());
                    self.push_event(
                        phase_event_kind(phase),
                        format!("#{step} {name} · {}", summarize_arguments(&arguments)),
                    );
                }
                AgentEvent::ToolResult { name, result } => {
                    self.push_event(
                        EventKind::Result,
                        format!("{name} · {}", summarize_result(&result)),
                    );
                }
                AgentEvent::ContextCompressed {
                    covered_messages,
                    before_tokens,
                    after_tokens,
                } => self.push_event(
                    EventKind::Context,
                    format!(
                        "压缩 {covered_messages} 条消息 · {before_tokens} → {after_tokens} tokens"
                    ),
                ),
                AgentEvent::WorkspaceChanged {
                    revision,
                    tool_name,
                } => {
                    self.workspace_revision = revision;
                    self.push_event(
                        EventKind::Execute,
                        format!("workspace rev {revision} · {tool_name}"),
                    );
                }
                AgentEvent::VerificationFinished {
                    revision,
                    command,
                    passed,
                } => {
                    if passed {
                        self.last_verified_revision = Some(revision);
                    } else if self.last_verified_revision == Some(revision) {
                        self.last_verified_revision = None;
                    }
                    self.push_event(
                        EventKind::Verify,
                        format!(
                            "{} rev {revision} · {}",
                            if passed { "PASS" } else { "FAIL" },
                            summarize_result(&command)
                        ),
                    );
                }
                AgentEvent::FinishBlocked {
                    workspace_revision,
                    last_verified_revision,
                } => {
                    self.workspace_revision = workspace_revision;
                    self.last_verified_revision = last_verified_revision;
                    self.push_event(
                        EventKind::Verify,
                        format!("DONE 被阻止 · rev {workspace_revision} 未验证"),
                    );
                }
            },
            WorkerEvent::Done(result) => {
                self.busy = false;
                match result {
                    Ok(answer) => {
                        self.status = "就绪".to_owned();
                        self.push_message(MessageRole::Agent, answer);
                        self.push_event(EventKind::State, "任务完成");
                    }
                    Err(error) => {
                        self.status = "失败".to_owned();
                        self.push_message(MessageRole::Error, error.clone());
                        self.push_event(EventKind::Error, error);
                    }
                }
            }
            WorkerEvent::Confirm { prompt, response } => {
                self.pending_confirmation = Some(PendingConfirmation { prompt, response });
            }
            WorkerEvent::Fatal(error) => {
                self.busy = false;
                self.status = "初始化失败".to_owned();
                self.push_message(MessageRole::Error, error.clone());
                self.push_event(EventKind::Error, error);
            }
        }
    }

    fn submit(&mut self, command_tx: &mpsc::UnboundedSender<WorkerCommand>) {
        let text = self.input.text();
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        if trimmed.starts_with('/') {
            self.handle_command(trimmed);
            self.input.clear();
            return;
        }
        if self.busy {
            self.push_event(EventKind::Info, "当前任务仍在执行，请稍候");
            return;
        }
        self.history.push(text.clone());
        self.input.clear();
        self.push_message(MessageRole::User, text.clone());
        if command_tx.send(WorkerCommand::Chat(text)).is_err() {
            self.push_message(MessageRole::Error, "Agent worker 已停止");
        }
    }

    fn handle_command(&mut self, command: &str) {
        match command {
            "/exit" | "/quit" => self.should_quit = true,
            "/help" => self.show_help = !self.show_help,
            "/clear" => {
                self.messages.clear();
                self.events.clear();
                self.push_message(
                    MessageRole::System,
                    "界面记录已清空；Agent 对话历史仍保留。",
                )
            }
            "/status" => self.push_message(
                MessageRole::System,
                format!(
                    "状态：{}\n阶段：{}\n版本：rev {} / {}\n项目：{}\n验证：{}\n模型：{}\nWorkspace：{}",
                    self.status,
                    self.phase.label(),
                    self.workspace_revision,
                    verification_label(self.workspace_revision, self.last_verified_revision),
                    self.project_kind,
                    self.verification_command.as_deref().unwrap_or("模型选择"),
                    self.model,
                    self.workspace.display()
                ),
            ),
            "/tools" => {
                let tools = if self.tools.is_empty() {
                    "工具尚未加载".to_owned()
                } else {
                    self.tools.join("\n- ")
                };
                self.push_message(MessageRole::System, format!("可用工具：\n- {tools}"));
            }
            _ => self.push_message(MessageRole::Error, format!("未知命令：{command}")),
        }
    }

    fn resolve_confirmation(&mut self, allowed: bool) {
        if let Some(pending) = self.pending_confirmation.take() {
            let _ = pending.response.send(allowed);
            self.push_event(
                EventKind::Info,
                if allowed {
                    "用户允许执行命令"
                } else {
                    "用户拒绝执行命令"
                },
            );
        }
    }
}

pub async fn run(config: Config) -> Result<()> {
    let (command_tx, command_rx) = mpsc::unbounded_channel();
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let workspace = config.workspace.clone();
    let model = config.model.clone();
    let worker = tokio::spawn(worker_loop(config, command_rx, event_tx));
    let mut app = App::new(workspace, model);
    let mut terminal = TerminalGuard::new()?;

    while !app.should_quit {
        while let Ok(event) = event_rx.try_recv() {
            app.apply_worker_event(event);
        }
        terminal.terminal.draw(|frame| draw(frame, &mut app))?;

        if event::poll(POLL_INTERVAL)? {
            match event::read()? {
                TerminalEvent::Key(key)
                    if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                {
                    handle_key(&mut app, key, &command_tx);
                }
                TerminalEvent::Resize(_, _) => {}
                _ => {}
            }
        }
    }

    app.resolve_confirmation(false);
    let _ = command_tx.send(WorkerCommand::Shutdown);
    worker.abort();
    Ok(())
}

async fn worker_loop(
    config: Config,
    mut command_rx: mpsc::UnboundedReceiver<WorkerCommand>,
    event_tx: mpsc::UnboundedSender<WorkerEvent>,
) {
    let confirm_events = event_tx.clone();
    let approval: ApprovalFn = Arc::new(move |prompt| {
        let (response, receiver) = std_mpsc::channel();
        if confirm_events
            .send(WorkerEvent::Confirm {
                prompt: prompt.to_owned(),
                response,
            })
            .is_err()
        {
            return false;
        }
        receiver.recv().unwrap_or(false)
    });
    let tools = match default_registry_with_approval(
        config.workspace.clone(),
        config.auto_approve,
        approval,
    ) {
        Ok(tools) => tools,
        Err(error) => {
            let _ = event_tx.send(WorkerEvent::Fatal(error.to_string()));
            return;
        }
    };
    let tool_names = tools
        .definitions()
        .into_iter()
        .map(|tool| tool.function.name)
        .collect();
    let model = match HttpLanguageModel::new(
        &config.base_url,
        config.api_key.clone(),
        config.model.clone(),
    ) {
        Ok(model) => Arc::new(model),
        Err(error) => {
            let _ = event_tx.send(WorkerEvent::Fatal(error.to_string()));
            return;
        }
    };
    let mut agent = Agent::new(
        model,
        tools,
        &config.workspace,
        config.max_steps,
        config.context_window_tokens,
    );
    let agent_events = event_tx.clone();
    agent.on_event(move |event| {
        let _ = agent_events.send(WorkerEvent::Agent(event));
    });
    let _ = event_tx.send(WorkerEvent::Ready { tools: tool_names });

    while let Some(command) = command_rx.recv().await {
        match command {
            WorkerCommand::Chat(task) => {
                let _ = event_tx.send(WorkerEvent::Started);
                let result = agent
                    .run_turn(&task)
                    .await
                    .map_err(|error| error.to_string());
                let _ = event_tx.send(WorkerEvent::Done(result));
            }
            WorkerCommand::Shutdown => break,
        }
    }
}

fn handle_key(app: &mut App, key: KeyEvent, command_tx: &mpsc::UnboundedSender<WorkerCommand>) {
    if app.pending_confirmation.is_some() {
        match key.code {
            KeyCode::Char('y' | 'Y') => app.resolve_confirmation(true),
            KeyCode::Char('n' | 'N') | KeyCode::Esc | KeyCode::Enter => {
                app.resolve_confirmation(false);
            }
            _ => {}
        }
        return;
    }
    if app.show_help {
        if matches!(key.code, KeyCode::F(1) | KeyCode::Esc) {
            app.show_help = false;
        }
        return;
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('c' | 'd') => app.should_quit = true,
            KeyCode::Char('l') => app.events_collapsed = !app.events_collapsed,
            KeyCode::Char('p') => {
                if let Some(value) = app.history.prev() {
                    app.input.set_text(&value);
                }
            }
            KeyCode::Char('n') => {
                if let Some(value) = app.history.next() {
                    app.input.set_text(&value);
                }
            }
            _ => {}
        }
        return;
    }
    match key.code {
        KeyCode::F(1) => app.show_help = true,
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => app.input.newline(),
        KeyCode::Enter => app.submit(command_tx),
        KeyCode::Char(character) => {
            app.history.reset();
            app.input.insert_char(character);
        }
        KeyCode::Backspace => app.input.backspace(),
        KeyCode::Left => app.input.move_left(),
        KeyCode::Right => app.input.move_right(),
        KeyCode::Up if app.input.is_empty() => {
            app.follow_chat = false;
            app.chat_scroll = app.chat_scroll.saturating_sub(1);
        }
        KeyCode::Down if app.input.is_empty() => {
            app.chat_scroll = app.chat_scroll.saturating_add(1);
        }
        KeyCode::Up => app.input.move_up(),
        KeyCode::Down => app.input.move_down(),
        KeyCode::PageUp => {
            app.follow_chat = false;
            app.chat_scroll = app.chat_scroll.saturating_sub(app.chat_height.max(1));
        }
        KeyCode::PageDown => {
            app.chat_scroll = app.chat_scroll.saturating_add(app.chat_height.max(1));
        }
        KeyCode::Home if app.input.is_empty() => {
            app.follow_chat = false;
            app.chat_scroll = 0;
        }
        KeyCode::End if app.input.is_empty() => app.follow_chat = true,
        KeyCode::Home => app.input.move_line_start(),
        KeyCode::End => app.input.move_line_end(),
        KeyCode::Esc => app.input.clear(),
        _ => {}
    }
}

fn draw(frame: &mut Frame<'_>, app: &mut App) {
    let Some(areas) = split(frame.area(), app.events_collapsed) else {
        return;
    };
    let workspace = Paragraph::new(vec![
        Line::from(Span::styled(
            "NJU Coding Agent",
            Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(format!("Workspace  {}", display_path(&app.workspace))),
        Line::from("F1 帮助 · Ctrl+L 事件栏 · Ctrl+D 退出"),
    ])
    .block(Block::default().borders(Borders::ALL).title("Workspace"));
    frame.render_widget(workspace, areas.workspace);

    let status_color = if app.busy {
        Color::Yellow
    } else {
        Color::LightGreen
    };
    let runtime = Paragraph::new(vec![
        Line::from(vec![
            Span::raw("状态  "),
            Span::styled(&app.status, Style::default().fg(status_color)),
            Span::raw(format!(" · 工具 {}", app.tools.len())),
        ]),
        Line::from(format!(
            "阶段  {} · rev {} {}",
            app.phase.label(),
            app.workspace_revision,
            verification_label(app.workspace_revision, app.last_verified_revision)
        )),
        Line::from(format!(
            "项目  {} · {}",
            app.project_kind,
            app.verification_command.as_deref().unwrap_or("模型选择")
        )),
        Line::from(format!("模型  {}", app.model)),
    ])
    .block(Block::default().borders(Borders::ALL).title("Runtime"));
    frame.render_widget(runtime, areas.runtime);

    let chat_lines = app
        .messages
        .iter()
        .flat_map(ChatMessage::render_lines)
        .collect::<Vec<_>>();
    app.chat_height = areas.chat.height.saturating_sub(2);
    let max_scroll = u16::try_from(chat_lines.len().saturating_sub(app.chat_height as usize))
        .unwrap_or(u16::MAX);
    if app.follow_chat {
        app.chat_scroll = max_scroll;
    } else {
        app.chat_scroll = app.chat_scroll.min(max_scroll);
    }
    let chat = Paragraph::new(chat_lines)
        .block(Block::default().borders(Borders::ALL).title("Conversation"))
        .wrap(Wrap { trim: false })
        .scroll((app.chat_scroll, 0));
    frame.render_widget(chat, areas.chat);

    if !app.events_collapsed {
        let event_lines = app
            .events
            .iter()
            .map(EventEntry::render_line)
            .collect::<Vec<_>>();
        let event_scroll = u16::try_from(
            event_lines
                .len()
                .saturating_sub(areas.events.height.saturating_sub(2) as usize),
        )
        .unwrap_or(u16::MAX);
        let events = Paragraph::new(event_lines)
            .block(Block::default().borders(Borders::ALL).title("Events"))
            .wrap(Wrap { trim: false })
            .scroll((event_scroll, 0));
        frame.render_widget(events, areas.events);
    }

    let input_scroll = u16::try_from(app.input.cursor_line.saturating_sub(2)).unwrap_or(u16::MAX);
    let input = Paragraph::new(app.input.visual_lines())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(if app.busy {
                    "Input · Agent 忙"
                } else {
                    "Input"
                })
                .border_style(if app.busy {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default().fg(Color::LightBlue)
                }),
        )
        .scroll((input_scroll, 0));
    frame.render_widget(input, areas.input);

    let footer = Paragraph::new(Line::from(vec![
        Span::styled(
            " Enter ",
            Style::default().fg(Color::Black).bg(Color::LightBlue),
        ),
        Span::raw("发送  "),
        Span::styled(
            " Shift+Enter ",
            Style::default().fg(Color::Black).bg(Color::Gray),
        ),
        Span::raw("换行  "),
        Span::styled(
            " F1 ",
            Style::default().fg(Color::Black).bg(Color::LightCyan),
        ),
        Span::raw("帮助"),
    ]));
    frame.render_widget(footer, areas.footer);

    if app.pending_confirmation.is_none() && !app.show_help {
        let visible_line = app.input.cursor_line.saturating_sub(input_scroll as usize);
        let x = areas
            .input
            .x
            .saturating_add(1)
            .saturating_add(u16::try_from(app.input.cursor_display_col()).unwrap_or(u16::MAX));
        let y = areas
            .input
            .y
            .saturating_add(1)
            .saturating_add(u16::try_from(visible_line).unwrap_or(u16::MAX));
        frame.set_cursor_position((x, y));
    }
    if let Some(pending) = &app.pending_confirmation {
        overlay::draw_confirmation(frame, &pending.prompt);
    } else if app.show_help {
        overlay::draw_help(frame);
    }
}

fn display_path(path: &std::path::Path) -> String {
    let display = path.display().to_string();
    display.strip_prefix(r"\\?\").unwrap_or(&display).to_owned()
}

fn phase_event_kind(phase: AgentPhase) -> EventKind {
    match phase {
        AgentPhase::Planning => EventKind::Plan,
        AgentPhase::Executing => EventKind::Execute,
        AgentPhase::Verifying => EventKind::Verify,
        AgentPhase::Done => EventKind::Done,
    }
}

fn phase_status(phase: AgentPhase) -> &'static str {
    match phase {
        AgentPhase::Planning => "规划中",
        AgentPhase::Executing => "执行中",
        AgentPhase::Verifying => "验证中",
        AgentPhase::Done => "已完成",
    }
}

fn verification_label(workspace_revision: u64, last_verified_revision: Option<u64>) -> String {
    if workspace_revision == 0 {
        "clean".to_owned()
    } else if last_verified_revision == Some(workspace_revision) {
        "✓".to_owned()
    } else {
        "未验证".to_owned()
    }
}
