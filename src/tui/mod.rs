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
use crate::context::{ContextUsage, DEFAULT_CONTEXT_WINDOW_TOKENS};
use crate::llm::{Message, Role};
use crate::session::{SessionStore, SessionSummary, StoredSession};
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
const MAX_WORKER_EVENTS_PER_TICK: usize = 256;

enum WorkerCommand {
    Chat(String),
    SetPlanMode(bool),
    NewSession,
    ListSessions,
    SwitchSession(String),
    DeleteSession(String),
    Shutdown,
}

enum WorkerEvent {
    Ready {
        tools: Vec<String>,
        data_dir: PathBuf,
        project_kind: String,
        verification_command: Option<String>,
    },
    Started,
    Agent(AgentEvent),
    Done(std::result::Result<String, String>),
    SessionChanged {
        id: String,
        title: String,
        messages: Vec<Message>,
        workspace_revision: u64,
        last_verified_revision: Option<u64>,
        planning_enabled: bool,
        context_usage: ContextUsage,
        message_count: usize,
    },
    ContextUpdated {
        usage: ContextUsage,
        message_count: usize,
    },
    PlanModeChanged(bool),
    SessionList {
        current_id: String,
        sessions: Vec<SessionSummary>,
    },
    Notice(String),
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
    session_id: String,
    session_data_dir: Option<PathBuf>,
    should_quit: bool,
    show_help: bool,
    show_context: bool,
    context_usage: ContextUsage,
    message_count: usize,
    planning_enabled: bool,
    events_collapsed: bool,
    chat_scroll: u16,
    follow_chat: bool,
    chat_height: u16,
    pending_confirmation: Option<PendingConfirmation>,
    streaming_index: Option<usize>,
    streaming_phase: Option<AgentPhase>,
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
            session_id: "初始化".to_owned(),
            session_data_dir: None,
            should_quit: false,
            show_help: false,
            show_context: false,
            context_usage: ContextUsage::empty(DEFAULT_CONTEXT_WINDOW_TOKENS),
            message_count: 0,
            planning_enabled: true,
            events_collapsed: false,
            chat_scroll: 0,
            follow_chat: true,
            chat_height: 10,
            pending_confirmation: None,
            streaming_index: None,
            streaming_phase: None,
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
            WorkerEvent::Ready {
                tools,
                data_dir,
                project_kind,
                verification_command,
            } => {
                self.tools = tools;
                self.session_data_dir = Some(data_dir);
                self.project_kind = project_kind;
                self.verification_command = verification_command;
                self.status = "就绪".to_owned();
                self.push_event(EventKind::Info, "Agent 初始化完成");
            }
            WorkerEvent::Started => {
                self.discard_streaming();
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
                    self.finalize_streaming(MessageRole::Plan, &plan);
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
                AgentEvent::TextDelta { step, phase, delta } => {
                    self.phase = phase;
                    self.status = format!("{} · step {step} · 流式输出", phase.label());
                    self.append_streaming_delta(phase, &delta);
                }
                AgentEvent::ToolCall {
                    step,
                    phase,
                    name,
                    arguments,
                } => {
                    self.discard_streaming();
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
                    self.discard_streaming();
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
                        self.finalize_streaming(MessageRole::Agent, &answer);
                        self.push_event(EventKind::State, "任务完成");
                    }
                    Err(error) => {
                        self.discard_streaming();
                        self.status = "失败".to_owned();
                        self.push_message(MessageRole::Error, error.clone());
                        self.push_event(EventKind::Error, error);
                    }
                }
            }
            WorkerEvent::SessionChanged {
                id,
                title,
                messages,
                workspace_revision,
                last_verified_revision,
                planning_enabled,
                context_usage,
                message_count,
            } => {
                self.discard_streaming();
                self.session_id = id.clone();
                self.workspace_revision = workspace_revision;
                self.last_verified_revision = last_verified_revision;
                self.planning_enabled = planning_enabled;
                self.context_usage = context_usage;
                self.message_count = message_count;
                self.phase = AgentPhase::Done;
                self.status = "就绪".to_owned();
                self.busy = false;
                self.history = History::default();
                self.restore_transcript(&id, &title, &messages);
                self.events.clear();
                self.push_event(
                    EventKind::Session,
                    format!("当前会话 {} · {title}", short_session_id(&id)),
                );
            }
            WorkerEvent::ContextUpdated {
                usage,
                message_count,
            } => {
                self.context_usage = usage;
                self.message_count = message_count;
            }
            WorkerEvent::PlanModeChanged(enabled) => {
                self.planning_enabled = enabled;
                self.push_message(
                    MessageRole::System,
                    format!(
                        "Plan 模式已{}。{}",
                        if enabled { "开启" } else { "关闭" },
                        if enabled {
                            "后续任务执行 PLAN → EXECUTE → VERIFY。"
                        } else {
                            "后续任务直接 EXECUTE → VERIFY；修改后的验证要求仍然有效。"
                        }
                    ),
                );
                self.push_event(
                    EventKind::State,
                    format!("Plan mode {}", if enabled { "ON" } else { "OFF" }),
                );
            }
            WorkerEvent::SessionList {
                current_id,
                sessions,
            } => {
                self.push_message(
                    MessageRole::System,
                    format_session_list(&current_id, &sessions),
                );
                self.push_event(EventKind::Session, format!("共 {} 个会话", sessions.len()));
            }
            WorkerEvent::Notice(message) => {
                self.push_message(MessageRole::System, message.clone());
                self.push_event(EventKind::Session, message);
            }
            WorkerEvent::Confirm { prompt, response } => {
                self.pending_confirmation = Some(PendingConfirmation { prompt, response });
            }
            WorkerEvent::Fatal(error) => {
                self.discard_streaming();
                self.busy = false;
                self.status = "初始化失败".to_owned();
                self.push_message(MessageRole::Error, error.clone());
                self.push_event(EventKind::Error, error);
            }
        }
    }

    fn restore_transcript(&mut self, id: &str, title: &str, messages: &[Message]) {
        self.messages.clear();
        self.push_message(
            MessageRole::System,
            format!("会话 {} · {title}", short_session_id(id)),
        );
        for (index, message) in messages.iter().enumerate() {
            let Some(content) = message
                .content
                .as_deref()
                .filter(|text| !text.trim().is_empty())
            else {
                continue;
            };
            match message.role {
                Role::User => self.push_message(MessageRole::User, content),
                Role::Assistant if message.tool_calls.is_none() => {
                    let role = if is_plan_message(messages, index) {
                        MessageRole::Plan
                    } else {
                        MessageRole::Agent
                    };
                    self.push_message(role, content);
                }
                Role::System | Role::Assistant | Role::Tool => {}
            }
        }
    }

    fn append_streaming_delta(&mut self, phase: AgentPhase, delta: &str) {
        if delta.is_empty() {
            return;
        }
        if self.streaming_phase.is_some_and(|active| active != phase) {
            self.discard_streaming();
        }
        if self.streaming_index.is_none() {
            if self.messages.len() >= MAX_MESSAGES {
                self.messages.remove(0);
            }
            let role = if phase == AgentPhase::Planning {
                MessageRole::Plan
            } else {
                MessageRole::Agent
            };
            self.messages.push(ChatMessage::new(role, ""));
            self.streaming_index = Some(self.messages.len().saturating_sub(1));
            self.streaming_phase = Some(phase);
        }
        if let Some(message) = self
            .streaming_index
            .and_then(|index| self.messages.get_mut(index))
        {
            message.append(delta);
        }
        self.follow_chat = true;
    }

    fn finalize_streaming(&mut self, role: MessageRole, final_text: &str) {
        let index = self.streaming_index.take();
        self.streaming_phase = None;
        match index {
            Some(index)
                if final_text.is_empty()
                    && self.messages.get(index).is_some_and(ChatMessage::is_empty) =>
            {
                self.messages.remove(index);
            }
            Some(_) if final_text.is_empty() => {}
            Some(index) => {
                if let Some(message) = self.messages.get_mut(index) {
                    message.finalize(role, final_text);
                }
            }
            None if !final_text.is_empty() => self.push_message(role, final_text),
            None => {}
        }
        self.follow_chat = true;
    }

    fn discard_streaming(&mut self) {
        if let Some(index) = self.streaming_index.take()
            && index < self.messages.len()
        {
            self.messages.remove(index);
        }
        self.streaming_phase = None;
    }

    fn submit(&mut self, command_tx: &mpsc::UnboundedSender<WorkerCommand>) {
        let text = self.input.text();
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        if trimmed.starts_with('/') {
            self.handle_command(trimmed, command_tx);
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

    fn handle_command(&mut self, command: &str, command_tx: &mpsc::UnboundedSender<WorkerCommand>) {
        let mut parts = command.split_whitespace();
        let name = parts.next().unwrap_or_default();
        match name {
            "/exit" | "/quit" => self.should_quit = true,
            "/help" => {
                self.show_context = false;
                self.show_help = !self.show_help;
            }
            "/context" if parts.next().is_none() => {
                self.show_help = false;
                self.show_context = !self.show_context;
            }
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
                    "状态：{}\n会话：{}\nPlan：{}\n阶段：{}\n版本：rev {} / {}\n上下文：{} / {} tokens ({}%)\n项目：{}\n验证：{}\n模型：{}\nWorkspace：{}\n会话目录：{}",
                    self.status,
                    self.session_id,
                    if self.planning_enabled { "ON" } else { "OFF" },
                    self.phase.label(),
                    self.workspace_revision,
                    verification_label(self.workspace_revision, self.last_verified_revision),
                    self.context_usage.used_tokens,
                    self.context_usage.window_tokens,
                    self.context_usage.used_percent(),
                    self.project_kind,
                    self.verification_command.as_deref().unwrap_or("模型选择"),
                    self.model,
                    self.workspace.display(),
                    self.session_data_dir
                        .as_deref()
                        .map_or_else(|| "初始化中".to_owned(), display_path)
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
            "/plan" => match (parts.next(), parts.next()) {
                (None, None) => self.push_message(
                    MessageRole::System,
                    format!(
                        "Plan 模式：{}。使用 /plan on 或 /plan off 切换；Verify 约束始终开启。",
                        if self.planning_enabled { "ON" } else { "OFF" }
                    ),
                ),
                (Some("on"), None) => {
                    self.send_session_command(command_tx, WorkerCommand::SetPlanMode(true))
                }
                (Some("off"), None) => {
                    self.send_session_command(command_tx, WorkerCommand::SetPlanMode(false))
                }
                _ => self.push_message(MessageRole::Error, "用法：/plan [on|off]"),
            },
            "/new" if parts.next().is_none() => {
                self.send_session_command(command_tx, WorkerCommand::NewSession)
            }
            "/sessions" if parts.next().is_none() => {
                self.send_session_command(command_tx, WorkerCommand::ListSessions)
            }
            "/switch" => match (parts.next(), parts.next()) {
                (Some(id), None) => self.send_session_command(
                    command_tx,
                    WorkerCommand::SwitchSession(id.to_owned()),
                ),
                _ => self.push_message(MessageRole::Error, "用法：/switch <id>"),
            },
            "/delete" => match (parts.next(), parts.next()) {
                (Some(id), None) => self.send_session_command(
                    command_tx,
                    WorkerCommand::DeleteSession(id.to_owned()),
                ),
                _ => self.push_message(MessageRole::Error, "用法：/delete <id>"),
            },
            _ => self.push_message(MessageRole::Error, format!("未知命令：{command}")),
        }
    }

    fn send_session_command(
        &mut self,
        command_tx: &mpsc::UnboundedSender<WorkerCommand>,
        command: WorkerCommand,
    ) {
        if self.busy {
            self.push_event(EventKind::Info, "Agent 执行中，暂不能切换会话");
        } else if command_tx.send(command).is_err() {
            self.push_message(MessageRole::Error, "Agent worker 已停止");
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
    let context_window_tokens = config.context_window_tokens;
    let planning_enabled = config.planning_enabled;
    let worker = tokio::spawn(worker_loop(config, command_rx, event_tx));
    let mut app = App::new(workspace, model);
    app.context_usage = ContextUsage::empty(context_window_tokens);
    app.planning_enabled = planning_enabled;
    let mut terminal = TerminalGuard::new()?;

    while !app.should_quit {
        for _ in 0..MAX_WORKER_EVENTS_PER_TICK {
            let Ok(event) = event_rx.try_recv() else {
                break;
            };
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
    if app.busy {
        worker.abort();
    } else {
        let _ = command_tx.send(WorkerCommand::Shutdown);
        let _ = worker.await;
    }
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
    agent.set_planning_enabled(config.planning_enabled);
    let agent_events = event_tx.clone();
    agent.on_event(move |event| {
        let _ = agent_events.send(WorkerEvent::Agent(event));
    });
    let store = match SessionStore::open_default() {
        Ok(store) => store,
        Err(error) => {
            let _ = event_tx.send(WorkerEvent::Fatal(format!("无法初始化会话存储：{error}")));
            return;
        }
    };
    let mut current_session = match store.latest_for_workspace(&config.workspace) {
        Ok(Some(session)) => {
            if let Err(error) = agent.restore_state(session.state.clone()) {
                let _ = event_tx.send(WorkerEvent::Fatal(format!(
                    "无法恢复会话 {}: {error}",
                    session.id
                )));
                return;
            }
            session
        }
        Ok(None) => match store.create(&config.workspace, agent.export_state()) {
            Ok(session) => session,
            Err(error) => {
                let _ = event_tx.send(WorkerEvent::Fatal(format!("无法创建会话：{error}")));
                return;
            }
        },
        Err(error) => {
            let _ = event_tx.send(WorkerEvent::Fatal(format!("无法读取会话：{error}")));
            return;
        }
    };
    // `--no-plan` is an explicit startup override. With the default setting,
    // an existing session keeps its own persisted Plan preference.
    if !config.planning_enabled && agent.planning_enabled() {
        agent.set_planning_enabled(false);
        current_session.update(agent.export_state(), None);
        if let Err(error) = store.save(&current_session) {
            let _ = event_tx.send(WorkerEvent::Notice(format!(
                "无法保存 --no-plan 会话设置：{error}"
            )));
        }
    }
    let _ = event_tx.send(WorkerEvent::Ready {
        tools: tool_names,
        data_dir: store.root().to_path_buf(),
        project_kind: agent.project_profile().kind.label().to_owned(),
        verification_command: agent.project_profile().verification_command.clone(),
    });
    send_session_changed(&event_tx, &current_session, &agent);

    while let Some(command) = command_rx.recv().await {
        match command {
            WorkerCommand::Chat(task) => {
                let _ = event_tx.send(WorkerEvent::Started);
                let result = agent
                    .run_turn(&task)
                    .await
                    .map_err(|error| error.to_string());
                current_session.update(agent.export_state(), Some(&task));
                if let Err(error) = store.save(&current_session) {
                    let _ = event_tx.send(WorkerEvent::Notice(format!("会话保存失败：{error}")));
                }
                send_context_updated(&event_tx, &agent);
                let _ = event_tx.send(WorkerEvent::Done(result));
            }
            WorkerCommand::SetPlanMode(enabled) => {
                let previous = agent.planning_enabled();
                agent.set_planning_enabled(enabled);
                current_session.update(agent.export_state(), None);
                match store.save(&current_session) {
                    Ok(()) => {
                        let _ = event_tx.send(WorkerEvent::PlanModeChanged(enabled));
                        send_context_updated(&event_tx, &agent);
                    }
                    Err(error) => {
                        agent.set_planning_enabled(previous);
                        current_session.update(agent.export_state(), None);
                        let _ = event_tx
                            .send(WorkerEvent::Notice(format!("Plan 模式保存失败：{error}")));
                    }
                }
            }
            WorkerCommand::NewSession => {
                current_session.update(agent.export_state(), None);
                if let Err(error) = store.save(&current_session) {
                    let _ =
                        event_tx.send(WorkerEvent::Notice(format!("当前会话保存失败：{error}")));
                    continue;
                }
                agent.reset_state();
                match store.create(&config.workspace, agent.export_state()) {
                    Ok(session) => {
                        current_session = session;
                        send_session_changed(&event_tx, &current_session, &agent);
                    }
                    Err(error) => {
                        let _ = agent.restore_state(current_session.state.clone());
                        let _ =
                            event_tx.send(WorkerEvent::Notice(format!("新建会话失败：{error}")));
                    }
                }
            }
            WorkerCommand::ListSessions => match store.list_for_workspace(&config.workspace) {
                Ok(sessions) => {
                    let _ = event_tx.send(WorkerEvent::SessionList {
                        current_id: current_session.id.clone(),
                        sessions,
                    });
                }
                Err(error) => {
                    let _ =
                        event_tx.send(WorkerEvent::Notice(format!("读取会话列表失败：{error}")));
                }
            },
            WorkerCommand::SwitchSession(query) => {
                current_session.update(agent.export_state(), None);
                if let Err(error) = store.save(&current_session) {
                    let _ =
                        event_tx.send(WorkerEvent::Notice(format!("当前会话保存失败：{error}")));
                    continue;
                }
                match store.load(&query) {
                    Ok(session) if !session.belongs_to(&config.workspace) => {
                        let _ = event_tx.send(WorkerEvent::Notice(
                            "不能切换到其他 workspace 的会话；请用对应 --workspace 重启".to_owned(),
                        ));
                    }
                    Ok(session) => match agent.restore_state(session.state.clone()) {
                        Ok(()) => {
                            current_session = session;
                            send_session_changed(&event_tx, &current_session, &agent);
                        }
                        Err(error) => {
                            let _ = event_tx
                                .send(WorkerEvent::Notice(format!("会话恢复失败：{error}")));
                        }
                    },
                    Err(error) => {
                        let _ =
                            event_tx.send(WorkerEvent::Notice(format!("会话切换失败：{error}")));
                    }
                }
            }
            WorkerCommand::DeleteSession(query) => match store.load(&query) {
                Ok(session) if session.id == current_session.id => {
                    let previous_state = agent.export_state();
                    agent.reset_state();
                    match store.create(&config.workspace, agent.export_state()) {
                        Ok(replacement) => match store.delete(&session.id) {
                            Ok(id) => {
                                current_session = replacement;
                                send_session_changed(&event_tx, &current_session, &agent);
                                let _ =
                                    event_tx.send(WorkerEvent::Notice(format!("已删除会话 {id}")));
                            }
                            Err(error) => {
                                let _ = store.delete(&replacement.id);
                                let _ = agent.restore_state(previous_state);
                                let _ = event_tx
                                    .send(WorkerEvent::Notice(format!("删除会话失败：{error}")));
                            }
                        },
                        Err(error) => {
                            let _ = agent.restore_state(previous_state);
                            let _ = event_tx.send(WorkerEvent::Notice(format!(
                                "删除当前会话前无法创建替代会话：{error}"
                            )));
                        }
                    }
                }
                Ok(session) if !session.belongs_to(&config.workspace) => {
                    let _ = event_tx.send(WorkerEvent::Notice(
                        "不能删除其他 workspace 的会话".to_owned(),
                    ));
                }
                Ok(session) => match store.delete(&session.id) {
                    Ok(id) => {
                        let _ = event_tx.send(WorkerEvent::Notice(format!("已删除会话 {id}")));
                    }
                    Err(error) => {
                        let _ =
                            event_tx.send(WorkerEvent::Notice(format!("删除会话失败：{error}")));
                    }
                },
                Err(error) => {
                    let _ = event_tx.send(WorkerEvent::Notice(format!("删除会话失败：{error}")));
                }
            },
            WorkerCommand::Shutdown => {
                current_session.update(agent.export_state(), None);
                let _ = store.save(&current_session);
                break;
            }
        }
    }
}

fn send_session_changed(
    event_tx: &mpsc::UnboundedSender<WorkerEvent>,
    session: &StoredSession,
    agent: &Agent,
) {
    let _ = event_tx.send(WorkerEvent::SessionChanged {
        id: session.id.clone(),
        title: session.title.clone(),
        messages: session.state.messages.clone(),
        workspace_revision: session.state.workspace_revision,
        last_verified_revision: session.state.last_verified_revision,
        planning_enabled: agent.planning_enabled(),
        context_usage: agent.context_usage(),
        message_count: session.state.messages.len(),
    });
}

fn send_context_updated(event_tx: &mpsc::UnboundedSender<WorkerEvent>, agent: &Agent) {
    let _ = event_tx.send(WorkerEvent::ContextUpdated {
        usage: agent.context_usage(),
        message_count: agent.messages().len(),
    });
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
    if app.show_context {
        if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
            app.show_context = false;
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
        Line::from(format!("Session    {}", short_session_id(&app.session_id))),
        Line::from("F1 帮助 · /context 用量 · Ctrl+L 事件栏 · Ctrl+D 退出"),
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
        Line::from(format!(
            "模型  {} · Plan {} · Context {}%",
            app.model,
            if app.planning_enabled { "ON" } else { "OFF" },
            app.context_usage.used_percent()
        )),
    ])
    .block(Block::default().borders(Borders::ALL).title("Runtime"));
    frame.render_widget(runtime, areas.runtime);

    let chat_lines = app
        .messages
        .iter()
        .flat_map(ChatMessage::render_lines)
        .collect::<Vec<_>>();
    app.chat_height = areas.chat.height.saturating_sub(2);
    let chat_width = areas.chat.width.saturating_sub(2).max(1);
    let chat = Paragraph::new(chat_lines)
        .block(Block::default().borders(Borders::ALL).title("Conversation"))
        .wrap(Wrap { trim: false });
    // A logical `Line` can occupy several terminal rows after wrapping. Using
    // `chat_lines.len()` here leaves the last message below the viewport.
    let max_scroll = u16::try_from(
        chat.line_count(chat_width)
            .saturating_sub(areas.chat.height as usize),
    )
    .unwrap_or(u16::MAX);
    if app.follow_chat {
        app.chat_scroll = max_scroll;
    } else {
        app.chat_scroll = app.chat_scroll.min(max_scroll);
    }
    let chat = chat.scroll((app.chat_scroll, 0));
    frame.render_widget(chat, areas.chat);

    if !app.events_collapsed {
        let event_lines = app
            .events
            .iter()
            .map(EventEntry::render_line)
            .collect::<Vec<_>>();
        let events = Paragraph::new(event_lines)
            .block(Block::default().borders(Borders::ALL).title("Events"))
            .wrap(Wrap { trim: false });
        let event_width = areas.events.width.saturating_sub(2).max(1);
        let event_scroll = u16::try_from(
            events
                .line_count(event_width)
                .saturating_sub(areas.events.height as usize),
        )
        .unwrap_or(u16::MAX);
        let events = events.scroll((event_scroll, 0));
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
        Span::raw("帮助 · /context · /plan on|off"),
    ]));
    frame.render_widget(footer, areas.footer);

    if app.pending_confirmation.is_none() && !app.show_help && !app.show_context {
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
    } else if app.show_context {
        overlay::draw_context(frame, app.context_usage, app.message_count);
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

fn short_session_id(id: &str) -> String {
    const LIMIT: usize = 22;
    if id.chars().count() <= LIMIT {
        id.to_owned()
    } else {
        format!("{}…", id.chars().take(LIMIT).collect::<String>())
    }
}

fn is_plan_message(messages: &[Message], index: usize) -> bool {
    messages.get(index + 1).is_some_and(|next| {
        next.role == Role::System
            && next
                .content
                .as_deref()
                .is_some_and(|content| content.starts_with("The plan is recorded."))
    })
}

fn format_session_list(current_id: &str, sessions: &[SessionSummary]) -> String {
    if sessions.is_empty() {
        return "当前 workspace 还没有保存的会话。".to_owned();
    }
    let mut output = String::from("会话列表：\n");
    for session in sessions {
        let marker = if session.id == current_id { "*" } else { " " };
        let verification =
            verification_label(session.workspace_revision, session.last_verified_revision);
        output.push_str(&format!(
            "{marker} {}  {}  rev {} {}  plan {}  {}\n",
            session.id,
            session.title,
            session.workspace_revision,
            verification,
            if session.planning_enabled {
                "on"
            } else {
                "off"
            },
            session.updated_at
        ));
    }
    output.push_str("\n使用 /switch <id或唯一前缀> 切换。");
    output
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use tokio::sync::mpsc;

    use super::{App, WorkerCommand, WorkerEvent, draw, format_session_list};
    use crate::agent::{AgentEvent, AgentPhase};
    use crate::session::SessionSummary;
    use crate::tui::model::MessageRole;

    #[test]
    fn session_commands_are_dispatched_with_the_requested_id() {
        let mut app = App::new(PathBuf::from("workspace"), "model".to_owned());
        let (sender, mut receiver) = mpsc::unbounded_channel();

        app.handle_command("/new", &sender);
        assert!(matches!(receiver.try_recv(), Ok(WorkerCommand::NewSession)));

        app.handle_command("/sessions", &sender);
        assert!(matches!(
            receiver.try_recv(),
            Ok(WorkerCommand::ListSessions)
        ));

        app.handle_command("/switch abc123", &sender);
        assert!(matches!(
            receiver.try_recv(),
            Ok(WorkerCommand::SwitchSession(id)) if id == "abc123"
        ));

        app.handle_command("/delete def456", &sender);
        assert!(matches!(
            receiver.try_recv(),
            Ok(WorkerCommand::DeleteSession(id)) if id == "def456"
        ));

        app.handle_command("/plan off", &sender);
        assert!(matches!(
            receiver.try_recv(),
            Ok(WorkerCommand::SetPlanMode(false))
        ));

        app.handle_command("/context", &sender);
        assert!(app.show_context);
        app.handle_command("/context", &sender);
        assert!(!app.show_context);
    }

    #[test]
    fn session_list_marks_current_and_displays_revisions() {
        let sessions = vec![SessionSummary {
            id: "session-123".to_owned(),
            title: "fix checkout".to_owned(),
            workspace: PathBuf::from("workspace"),
            updated_at: "2026-08-27T20:00:00+08:00".to_owned(),
            workspace_revision: 2,
            last_verified_revision: Some(2),
            planning_enabled: true,
        }];

        let output = format_session_list("session-123", &sessions);

        assert!(output.contains("* session-123"));
        assert!(output.contains("fix checkout"));
        assert!(output.contains("rev 2"));
        assert!(output.contains("/switch"));
    }

    #[test]
    fn streamed_plan_and_final_answer_are_finalized_without_duplicates() {
        let mut app = App::new(PathBuf::from("workspace"), "model".to_owned());
        app.apply_worker_event(WorkerEvent::Started);
        app.apply_worker_event(WorkerEvent::Agent(AgentEvent::TextDelta {
            step: 1,
            phase: AgentPhase::Planning,
            delta: "1. Read ".to_owned(),
        }));
        app.apply_worker_event(WorkerEvent::Agent(AgentEvent::TextDelta {
            step: 1,
            phase: AgentPhase::Planning,
            delta: "the file.".to_owned(),
        }));
        app.apply_worker_event(WorkerEvent::Agent(AgentEvent::PlanCreated {
            plan: "1. Read the file.".to_owned(),
        }));
        app.apply_worker_event(WorkerEvent::Agent(AgentEvent::TextDelta {
            step: 2,
            phase: AgentPhase::Executing,
            delta: "Hello ".to_owned(),
        }));
        app.apply_worker_event(WorkerEvent::Agent(AgentEvent::TextDelta {
            step: 2,
            phase: AgentPhase::Executing,
            delta: "Session B".to_owned(),
        }));
        app.apply_worker_event(WorkerEvent::Done(Ok("Hello Session B".to_owned())));

        let plan_messages = app
            .messages
            .iter()
            .filter(|message| message.role() == MessageRole::Plan)
            .collect::<Vec<_>>();
        let agent_messages = app
            .messages
            .iter()
            .filter(|message| message.role() == MessageRole::Agent)
            .collect::<Vec<_>>();
        assert_eq!(plan_messages.len(), 1);
        assert_eq!(plan_messages[0].content(), "1. Read the file.");
        assert_eq!(agent_messages.len(), 1);
        assert_eq!(agent_messages[0].content(), "Hello Session B");
        assert!(app.streaming_index.is_none());
    }

    #[test]
    fn streamed_text_attached_to_a_tool_call_is_temporary() {
        let mut app = App::new(PathBuf::from("workspace"), "model".to_owned());
        let original_messages = app.messages.len();
        app.apply_worker_event(WorkerEvent::Agent(AgentEvent::TextDelta {
            step: 1,
            phase: AgentPhase::Planning,
            delta: "I will inspect it.".to_owned(),
        }));
        app.apply_worker_event(WorkerEvent::Agent(AgentEvent::ToolCall {
            step: 1,
            phase: AgentPhase::Planning,
            name: "read_file".to_owned(),
            arguments: r#"{"path":"hello.py"}"#.to_owned(),
        }));

        assert_eq!(app.messages.len(), original_messages);
        assert!(app.streaming_index.is_none());
    }

    #[test]
    fn follow_chat_keeps_latest_user_body_visible_after_wrapped_text() {
        let mut app = App::new(PathBuf::from("workspace"), "model".to_owned());
        app.messages.clear();
        app.push_message(MessageRole::Agent, "wrapped words ".repeat(24));
        app.push_message(MessageRole::User, "LATEST_USER_BODY");
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal
            .draw(|frame| draw(frame, &mut app))
            .expect("draw TUI");

        let rendered =
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .fold(String::new(), |mut output, cell| {
                    output.push_str(cell.symbol());
                    output
                });
        assert!(rendered.contains("LATEST_USER_BODY"));
        assert!(app.chat_scroll > 0, "wrapped rows must require scrolling");
    }
}
