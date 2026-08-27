use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{Local, SecondsFormat};
use serde::{Deserialize, Serialize};

use crate::agent::AgentState;
use crate::error::{Error, Result};

const SESSION_VERSION: u32 = 1;
const DEFAULT_TITLE: &str = "新对话";
static SESSION_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoredSession {
    pub version: u32,
    pub id: String,
    pub title: String,
    pub workspace: PathBuf,
    pub created_at: String,
    pub updated_at: String,
    pub state: AgentState,
}

impl StoredSession {
    pub fn update(&mut self, state: AgentState, first_task: Option<&str>) {
        if self.title == DEFAULT_TITLE
            && let Some(task) = first_task
            && !task.trim().is_empty()
        {
            self.title = session_title(task);
        }
        self.state = state;
        self.updated_at = timestamp();
    }

    pub fn belongs_to(&self, workspace: &Path) -> bool {
        same_path(&self.workspace, workspace)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub workspace: PathBuf,
    pub updated_at: String,
    pub workspace_revision: u64,
    pub last_verified_revision: Option<u64>,
}

impl From<&StoredSession> for SessionSummary {
    fn from(session: &StoredSession) -> Self {
        Self {
            id: session.id.clone(),
            title: session.title.clone(),
            workspace: session.workspace.clone(),
            updated_at: session.updated_at.clone(),
            workspace_revision: session.state.workspace_revision,
            last_verified_revision: session.state.last_verified_revision,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionStore {
    root: PathBuf,
}

impl SessionStore {
    pub fn open_default() -> Result<Self> {
        Self::new(default_session_root()?)
    }

    pub fn new(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn create(&self, workspace: &Path, state: AgentState) -> Result<StoredSession> {
        let now = timestamp();
        let session = StoredSession {
            version: SESSION_VERSION,
            id: new_session_id(),
            title: DEFAULT_TITLE.to_owned(),
            workspace: workspace.to_path_buf(),
            created_at: now.clone(),
            updated_at: now,
            state,
        };
        self.save(&session)?;
        Ok(session)
    }

    pub fn save(&self, session: &StoredSession) -> Result<()> {
        validate_id(&session.id)?;
        let payload = serde_json::to_vec_pretty(session)?;
        let path = self.path_for(&session.id);
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)?;
        file.write_all(&payload)?;
        file.sync_all()?;
        Ok(())
    }

    pub fn load(&self, query: &str) -> Result<StoredSession> {
        let id = self.resolve_id(query)?;
        let payload = fs::read(self.path_for(&id))?;
        let session = serde_json::from_slice::<StoredSession>(&payload)?;
        validate_id(&session.id)?;
        if session.id != id {
            return Err(Error::Config(format!(
                "session id does not match its file name: {id}"
            )));
        }
        if session.version != SESSION_VERSION {
            return Err(Error::Config(format!(
                "unsupported session version {} for '{id}'",
                session.version
            )));
        }
        Ok(session)
    }

    pub fn list(&self) -> Result<Vec<SessionSummary>> {
        let mut sessions = Vec::new();
        for entry in fs::read_dir(&self.root)?.flatten() {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let Some(file_id) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            let Ok(payload) = fs::read(&path) else {
                continue;
            };
            let Ok(session) = serde_json::from_slice::<StoredSession>(&payload) else {
                continue;
            };
            if session.version == SESSION_VERSION
                && session.id == file_id
                && validate_id(&session.id).is_ok()
            {
                sessions.push(SessionSummary::from(&session));
            }
        }
        sessions.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        Ok(sessions)
    }

    pub fn list_for_workspace(&self, workspace: &Path) -> Result<Vec<SessionSummary>> {
        Ok(self
            .list()?
            .into_iter()
            .filter(|session| same_path(&session.workspace, workspace))
            .collect())
    }

    pub fn latest_for_workspace(&self, workspace: &Path) -> Result<Option<StoredSession>> {
        self.list_for_workspace(workspace)?
            .into_iter()
            .next()
            .map(|session| self.load(&session.id))
            .transpose()
    }

    pub fn delete(&self, query: &str) -> Result<String> {
        let id = self.resolve_id(query)?;
        fs::remove_file(self.path_for(&id))?;
        Ok(id)
    }

    fn resolve_id(&self, query: &str) -> Result<String> {
        validate_id(query)?;
        if self.path_for(query).is_file() {
            return Ok(query.to_owned());
        }
        let matches = self
            .list()?
            .into_iter()
            .filter(|session| session.id.starts_with(query))
            .map(|session| session.id)
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [id] => Ok(id.clone()),
            [] => Err(Error::Config(format!("session not found: {query}"))),
            _ => Err(Error::Config(format!(
                "session id prefix is ambiguous: {query}"
            ))),
        }
    }

    fn path_for(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.json"))
    }
}

fn default_session_root() -> Result<PathBuf> {
    if let Some(root) = non_empty_env("CODING_AGENT_DATA_DIR") {
        return Ok(PathBuf::from(root).join("sessions"));
    }

    #[cfg(windows)]
    if let Some(root) = non_empty_env("LOCALAPPDATA") {
        return Ok(PathBuf::from(root)
            .join("nju-coding-agent")
            .join("sessions"));
    }

    #[cfg(not(windows))]
    {
        if let Some(root) = non_empty_env("XDG_DATA_HOME") {
            return Ok(PathBuf::from(root)
                .join("nju-coding-agent")
                .join("sessions"));
        }
        if let Some(home) = non_empty_env("HOME") {
            return Ok(PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("nju-coding-agent")
                .join("sessions"));
        }
    }

    Ok(env::current_dir()?
        .join(".nju-coding-agent-data")
        .join("sessions"))
}

fn non_empty_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn validate_id(id: &str) -> Result<()> {
    if id.is_empty()
        || !id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(Error::Config("invalid session id".to_owned()));
    }
    Ok(())
}

fn new_session_id() -> String {
    let counter = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(
        "{}-{}-{counter}",
        Local::now().format("%Y%m%d-%H%M%S-%3f"),
        std::process::id()
    )
}

fn timestamp() -> String {
    Local::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn session_title(task: &str) -> String {
    let normalized = task.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut title = normalized.chars().take(36).collect::<String>();
    if normalized.chars().count() > 36 {
        title.push('…');
    }
    if title.is_empty() {
        DEFAULT_TITLE.to_owned()
    } else {
        title
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    {
        normalize_windows_path(left) == normalize_windows_path(right)
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

#[cfg(windows)]
fn normalize_windows_path(path: &Path) -> String {
    path.display()
        .to_string()
        .replace('\\', "/")
        .trim_start_matches("//?/")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::Value;
    use tempfile::tempdir;

    use crate::agent::AgentState;
    use crate::llm::Message;

    use super::SessionStore;

    fn state() -> AgentState {
        AgentState {
            messages: vec![Message::system("system"), Message::user("hello")],
            workspace_revision: 2,
            last_verified_revision: Some(2),
        }
    }

    #[test]
    fn saves_lists_loads_and_deletes_sessions() {
        let data = tempdir().expect("data");
        let workspace = tempdir().expect("workspace");
        let store = SessionStore::new(data.path().join("sessions")).expect("store");
        let mut session = store
            .create(workspace.path(), state())
            .expect("create session");
        session.update(state(), Some("Fix the checkout discount calculation"));
        store.save(&session).expect("save session");

        let listed = store
            .list_for_workspace(workspace.path())
            .expect("list sessions");
        assert_eq!(listed.len(), 1);
        assert!(listed[0].title.starts_with("Fix the checkout"));

        let prefix = &session.id[..12];
        let loaded = store.load(prefix).expect("load by prefix");
        assert_eq!(loaded.state, state());
        assert!(loaded.belongs_to(workspace.path()));

        let deleted = store.delete(prefix).expect("delete");
        assert_eq!(deleted, session.id);
        assert!(store.list().expect("empty list").is_empty());
    }

    #[test]
    fn rejects_path_traversal_as_a_session_id() {
        let data = tempdir().expect("data");
        let store = SessionStore::new(data.path().join("sessions")).expect("store");

        let error = store.load("../secret").expect_err("must reject traversal");

        assert!(error.to_string().contains("invalid session id"));
    }

    #[test]
    fn rejects_a_session_whose_embedded_id_does_not_match_its_file() {
        let data = tempdir().expect("data");
        let workspace = tempdir().expect("workspace");
        let store = SessionStore::new(data.path().join("sessions")).expect("store");
        let session = store
            .create(workspace.path(), state())
            .expect("create session");
        let path = store.root().join(format!("{}.json", session.id));
        let mut payload = serde_json::from_slice::<Value>(&fs::read(&path).expect("read session"))
            .expect("parse session");
        payload["id"] = Value::String("different-id".to_owned());
        fs::write(
            &path,
            serde_json::to_vec(&payload).expect("serialize session"),
        )
        .expect("corrupt session id");

        let error = store.load(&session.id).expect_err("mismatch must fail");

        assert!(error.to_string().contains("does not match"));
        assert!(store.list().expect("list sessions").is_empty());
    }
}
