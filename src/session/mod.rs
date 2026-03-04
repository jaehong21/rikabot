use std::cmp::Reverse;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::providers::ChatMessage;

const INDEX_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub display_name: String,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionIndex {
    pub version: u32,
    pub current_session_id: String,
    pub sessions: Vec<SessionRecord>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionDeleteResult {
    pub deleted_session_id: String,
    pub current_session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionLine {
    role: String,
    content: String,
    timestamp: String,
}

pub struct SessionManager {
    sessions_dir: PathBuf,
    index_path: PathBuf,
    index: SessionIndex,
}

impl SessionManager {
    pub fn new(workspace_dir: &Path) -> Result<Self> {
        let sessions_dir = workspace_dir.join("sessions");
        fs::create_dir_all(&sessions_dir).with_context(|| {
            format!(
                "failed to create sessions dir at {}",
                sessions_dir.display()
            )
        })?;

        let index_path = sessions_dir.join("sessions.json");
        let mut manager = Self {
            sessions_dir,
            index_path,
            index: SessionIndex {
                version: INDEX_VERSION,
                current_session_id: String::new(),
                sessions: Vec::new(),
            },
        };

        manager.index = manager.load_or_recover_index()?;
        manager.ensure_bootstrap_session()?;
        Ok(manager)
    }

    pub fn current_session_id(&self) -> &str {
        &self.index.current_session_id
    }

    pub fn list_sessions(&self) -> Vec<SessionRecord> {
        let mut sessions = self.index.sessions.clone();
        sessions.sort_by_key(|item| Reverse(item.updated_at.clone()));
        sessions
    }

    pub fn get_session(&self, session_id: &str) -> Option<SessionRecord> {
        self.index
            .sessions
            .iter()
            .find(|record| record.id == session_id)
            .cloned()
    }

    pub fn sessions_dir_path(&self) -> &Path {
        &self.sessions_dir
    }

    pub fn reload_from_disk(&mut self) -> Result<()> {
        self.index = self.load_or_recover_index()?;
        self.ensure_bootstrap_session_internal(false)?;
        Ok(())
    }

    pub fn create_session(&mut self, display_name: Option<&str>) -> Result<SessionRecord> {
        let id = Uuid::new_v4().to_string();
        let now = now_rfc3339();
        let name = display_name
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| id.clone());

        let record = SessionRecord {
            id: id.clone(),
            display_name: name,
            created_at: now.clone(),
            updated_at: now,
            message_count: 0,
        };

        self.create_or_truncate_session_file(&id)?;
        self.index.current_session_id = id.clone();
        self.index.sessions.push(record.clone());
        self.save_index()?;

        Ok(record)
    }

    pub fn rename_session(
        &mut self,
        session_id: &str,
        display_name: &str,
    ) -> Result<SessionRecord> {
        validate_session_id(session_id)?;
        let new_name = display_name.trim();
        if new_name.is_empty() {
            anyhow::bail!("display_name must not be empty");
        }

        let idx = self
            .find_session_index(session_id)
            .ok_or_else(|| anyhow::anyhow!("session not found: {}", session_id))?;
        self.index.sessions[idx].display_name = new_name.to_string();
        self.index.sessions[idx].updated_at = now_rfc3339();
        self.save_index()?;
        Ok(self.index.sessions[idx].clone())
    }

    pub fn switch_session(
        &mut self,
        session_id: &str,
    ) -> Result<(SessionRecord, Vec<ChatMessage>)> {
        validate_session_id(session_id)?;
        let idx = self
            .find_session_index(session_id)
            .ok_or_else(|| anyhow::anyhow!("session not found: {}", session_id))?;

        self.index.current_session_id = session_id.to_string();
        self.index.sessions[idx].updated_at = now_rfc3339();
        self.save_index()?;

        let history = self.load_history(session_id)?;
        Ok((self.index.sessions[idx].clone(), history))
    }

    pub fn clear_current_session(&mut self) -> Result<(SessionRecord, Vec<ChatMessage>)> {
        let session_id = self.index.current_session_id.clone();
        let idx = self
            .find_session_index(&session_id)
            .ok_or_else(|| anyhow::anyhow!("current session not found: {}", session_id))?;

        self.create_or_truncate_session_file(&session_id)?;
        self.index.sessions[idx].message_count = 0;
        self.index.sessions[idx].updated_at = now_rfc3339();
        self.save_index()?;

        Ok((self.index.sessions[idx].clone(), Vec::new()))
    }

    pub fn clear_session(&mut self, session_id: &str) -> Result<(SessionRecord, Vec<ChatMessage>)> {
        validate_session_id(session_id)?;
        let idx = self
            .find_session_index(session_id)
            .ok_or_else(|| anyhow::anyhow!("session not found: {}", session_id))?;

        self.create_or_truncate_session_file(session_id)?;
        self.index.sessions[idx].message_count = 0;
        self.index.sessions[idx].updated_at = now_rfc3339();
        self.save_index()?;

        Ok((self.index.sessions[idx].clone(), Vec::new()))
    }

    pub fn delete_session(&mut self, session_id: &str) -> Result<SessionDeleteResult> {
        validate_session_id(session_id)?;
        let idx = self
            .find_session_index(session_id)
            .ok_or_else(|| anyhow::anyhow!("session not found: {}", session_id))?;

        self.index.sessions.remove(idx);
        let path = self.session_path(session_id)?;
        if path.exists() {
            let _ = fs::remove_file(&path);
        }

        if self.index.sessions.is_empty() {
            let rec = self.create_session(None)?;
            return Ok(SessionDeleteResult {
                deleted_session_id: session_id.to_string(),
                current_session_id: rec.id,
            });
        }

        self.index
            .sessions
            .sort_by_key(|session| Reverse(session.updated_at.clone()));
        self.index.current_session_id = self.index.sessions[0].id.clone();
        self.save_index()?;

        Ok(SessionDeleteResult {
            deleted_session_id: session_id.to_string(),
            current_session_id: self.index.current_session_id.clone(),
        })
    }

    pub fn load_history(&mut self, session_id: &str) -> Result<Vec<ChatMessage>> {
        validate_session_id(session_id)?;
        let path = self.session_path(session_id)?;
        if !path.exists() {
            self.create_or_truncate_session_file(session_id)?;
            return Ok(Vec::new());
        }

        let file = OpenOptions::new()
            .read(true)
            .open(&path)
            .with_context(|| format!("failed to open session file {}", path.display()))?;
        let reader = BufReader::new(file);
        let mut history = Vec::new();

        for line in reader.lines() {
            let raw = line?;
            if raw.trim().is_empty() {
                continue;
            }
            let item: SessionLine = match serde_json::from_str(&raw) {
                Ok(item) => item,
                Err(err) => {
                    tracing::warn!(
                        "Session file {} is malformed ({}), recreating empty file",
                        path.display(),
                        err
                    );
                    self.create_or_truncate_session_file(session_id)?;
                    self.update_message_count(session_id, 0)?;
                    return Ok(Vec::new());
                }
            };
            history.push(ChatMessage {
                role: item.role,
                content: item.content,
            });
        }

        self.update_message_count(session_id, history.len())?;
        Ok(history)
    }

    pub fn append_messages(&mut self, session_id: &str, messages: &[ChatMessage]) -> Result<()> {
        validate_session_id(session_id)?;
        if messages.is_empty() {
            return Ok(());
        }

        let path = self.session_path(session_id)?;
        if !path.exists() {
            self.create_or_truncate_session_file(session_id)?;
        }

        let mut file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)
            .with_context(|| format!("failed to open session file {}", path.display()))?;

        for msg in messages {
            let line = SessionLine {
                role: msg.role.clone(),
                content: msg.content.clone(),
                timestamp: now_rfc3339(),
            };
            file.write_all(serde_json::to_string(&line)?.as_bytes())?;
            file.write_all(b"\n")?;
        }
        file.flush()?;

        let idx = self
            .find_session_index(session_id)
            .ok_or_else(|| anyhow::anyhow!("session not found: {}", session_id))?;
        self.index.sessions[idx].message_count += messages.len();
        self.index.sessions[idx].updated_at = now_rfc3339();
        self.save_index()?;
        Ok(())
    }

    fn load_or_recover_index(&self) -> Result<SessionIndex> {
        if !self.index_path.exists() {
            return Ok(SessionIndex {
                version: INDEX_VERSION,
                current_session_id: String::new(),
                sessions: Vec::new(),
            });
        }

        let text = fs::read_to_string(&self.index_path).with_context(|| {
            format!(
                "failed to read sessions index {}",
                self.index_path.display()
            )
        })?;
        match serde_json::from_str::<SessionIndex>(&text) {
            Ok(index) => Ok(index),
            Err(err) => {
                tracing::warn!(
                    "sessions index {} is malformed ({}), recreating",
                    self.index_path.display(),
                    err
                );
                Ok(SessionIndex {
                    version: INDEX_VERSION,
                    current_session_id: String::new(),
                    sessions: Vec::new(),
                })
            }
        }
    }

    fn ensure_bootstrap_session(&mut self) -> Result<()> {
        self.ensure_bootstrap_session_internal(true)
    }

    fn ensure_bootstrap_session_internal(&mut self, persist_index: bool) -> Result<()> {
        self.index.version = INDEX_VERSION;

        for record in &self.index.sessions {
            validate_session_id(&record.id)?;
            if !self.session_path(&record.id)?.exists() {
                self.create_or_truncate_session_file(&record.id)?;
            }
        }

        let valid_current = self
            .index
            .sessions
            .iter()
            .any(|s| s.id == self.index.current_session_id);
        if self.index.sessions.is_empty() || !valid_current {
            let rec = self.create_session(None)?;
            self.index.current_session_id = rec.id;
            return Ok(());
        }

        if persist_index {
            self.save_index()?;
        }
        Ok(())
    }

    fn find_session_index(&self, session_id: &str) -> Option<usize> {
        self.index.sessions.iter().position(|s| s.id == session_id)
    }

    fn update_message_count(&mut self, session_id: &str, count: usize) -> Result<()> {
        if let Some(idx) = self.find_session_index(session_id) {
            if self.index.sessions[idx].message_count != count {
                self.index.sessions[idx].message_count = count;
                self.save_index()?;
            }
        }
        Ok(())
    }

    fn session_path(&self, session_id: &str) -> Result<PathBuf> {
        validate_session_id(session_id)?;
        Ok(self.sessions_dir.join(format!("{}.jsonl", session_id)))
    }

    fn create_or_truncate_session_file(&self, session_id: &str) -> Result<()> {
        let path = self.session_path(session_id)?;
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .with_context(|| {
                format!("failed to create/truncate session file {}", path.display())
            })?;
        Ok(())
    }

    fn save_index(&self) -> Result<()> {
        let json = serde_json::to_string_pretty(&self.index)?;
        let tmp_path = self
            .index_path
            .with_file_name(format!("sessions-{}.tmp", Uuid::new_v4()));
        fs::write(&tmp_path, json)
            .with_context(|| format!("failed to write temp index {}", tmp_path.display()))?;
        fs::rename(&tmp_path, &self.index_path).with_context(|| {
            format!(
                "failed to atomically replace index {}",
                self.index_path.display()
            )
        })?;
        Ok(())
    }
}

fn validate_session_id(session_id: &str) -> Result<()> {
    Uuid::parse_str(session_id)
        .map(|_| ())
        .map_err(|_| anyhow::anyhow!("invalid session_id (must be UUID): {}", session_id))
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_workspace(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rikabot-session-test-{}-{}", name, Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp workspace");
        dir
    }

    #[test]
    fn creates_default_session_and_persists_on_reload() {
        let workspace = temp_workspace("bootstrap");

        let manager = SessionManager::new(&workspace).expect("create manager");
        let sessions = manager.list_sessions();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].display_name, sessions[0].id);
        let initial_id = sessions[0].id.clone();

        let manager2 = SessionManager::new(&workspace).expect("reload manager");
        assert_eq!(manager2.current_session_id(), initial_id);
        assert_eq!(manager2.list_sessions().len(), 1);
    }

    #[test]
    fn append_and_load_history_roundtrips_and_updates_count() {
        let workspace = temp_workspace("roundtrip");
        let mut manager = SessionManager::new(&workspace).expect("create manager");
        let sid = manager.current_session_id().to_string();

        let to_append = vec![
            ChatMessage::user("hello"),
            ChatMessage::assistant("hi"),
            ChatMessage::tool("{\"tool_call_id\":\"1\",\"content\":\"ok\"}"),
        ];
        manager
            .append_messages(&sid, &to_append)
            .expect("append messages");

        let history = manager.load_history(&sid).expect("load history");
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].role, "user");
        assert_eq!(history[1].role, "assistant");
        assert_eq!(history[2].role, "tool");

        let record = manager
            .list_sessions()
            .into_iter()
            .find(|s| s.id == sid)
            .expect("session exists");
        assert_eq!(record.message_count, 3);
    }

    #[test]
    fn rename_switch_clear_and_delete_fallback_behavior() {
        let workspace = temp_workspace("ops");
        let mut manager = SessionManager::new(&workspace).expect("create manager");
        let s1 = manager.current_session_id().to_string();

        let s2 = manager
            .create_session(Some("work thread"))
            .expect("create thread");
        assert_eq!(manager.current_session_id(), s2.id);
        assert_eq!(s2.display_name, "work thread");

        let renamed = manager
            .rename_session(&s2.id, "renamed thread")
            .expect("rename");
        assert_eq!(renamed.display_name, "renamed thread");
        let fetched = manager.get_session(&s2.id).expect("get session");
        assert_eq!(fetched.display_name, "renamed thread");

        manager
            .append_messages(
                &s2.id,
                &[ChatMessage::user("x"), ChatMessage::assistant("y")],
            )
            .expect("append");
        let (_, switched_history) = manager.switch_session(&s2.id).expect("switch");
        assert_eq!(switched_history.len(), 2);

        let (cleared, history) = manager.clear_current_session().expect("clear");
        assert_eq!(cleared.display_name, "renamed thread");
        assert!(history.is_empty());

        let deleted = manager.delete_session(&s2.id).expect("delete");
        assert_eq!(deleted.deleted_session_id, s2.id);
        assert_eq!(deleted.current_session_id, s1);
        assert_eq!(manager.current_session_id(), s1);
    }

    #[test]
    fn malformed_index_or_history_recovers_with_empty_files() {
        let workspace = temp_workspace("recover");
        let manager = SessionManager::new(&workspace).expect("create manager");
        let sid = manager.current_session_id().to_string();

        fs::write(
            workspace.join("sessions").join("sessions.json"),
            "{not-json",
        )
        .expect("write bad index");
        let manager2 = SessionManager::new(&workspace).expect("reload with recovered index");
        assert!(!manager2.list_sessions().is_empty());

        let session_path = workspace.join("sessions").join(format!("{}.jsonl", sid));
        fs::write(&session_path, "{\"role\":\"user\"").expect("write bad session line");

        let mut manager3 = SessionManager::new(&workspace).expect("create manager 3");
        let history = manager3.load_history(&sid).expect("load recovered history");
        assert!(history.is_empty());
    }

    #[test]
    fn reload_from_disk_observes_external_session_updates() {
        let workspace = temp_workspace("reload");
        let mut manager = SessionManager::new(&workspace).expect("create manager");

        let created = {
            let mut external = SessionManager::new(&workspace).expect("create external manager");
            external
                .create_session(Some("external session"))
                .expect("create external session")
        };

        manager.reload_from_disk().expect("reload from disk");
        assert_eq!(manager.current_session_id(), created.id);
        assert_eq!(
            manager
                .get_session(&created.id)
                .expect("session should exist")
                .display_name,
            "external session"
        );
    }
}
