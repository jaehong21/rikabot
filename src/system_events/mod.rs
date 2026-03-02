use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SystemEventStatus {
    Pending,
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemEventRecord {
    pub event_id: String,
    pub text: String,
    pub session_id: String,
    pub session_display_name: String,
    pub created_at: String,
    pub status: SystemEventStatus,
}

pub struct SystemEventHandle {
    path: PathBuf,
    record: SystemEventRecord,
}

impl SystemEventHandle {
    pub fn create(
        workspace_dir: &Path,
        text: &str,
        session_id: &str,
        session_display_name: &str,
    ) -> Result<Self> {
        let events_dir = workspace_dir.join("events");
        fs::create_dir_all(&events_dir)
            .with_context(|| format!("failed to create events dir {}", events_dir.display()))?;

        let event_id = Uuid::new_v4().to_string();
        let path = events_dir.join(format!("{event_id}.json"));

        let mut handle = Self {
            path,
            record: SystemEventRecord {
                event_id,
                text: text.to_string(),
                session_id: session_id.to_string(),
                session_display_name: session_display_name.to_string(),
                created_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
                status: SystemEventStatus::Pending,
            },
        };
        handle.write()?;
        Ok(handle)
    }

    pub fn event_id(&self) -> &str {
        &self.record.event_id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn mark_running(&mut self) -> Result<()> {
        self.record.status = SystemEventStatus::Running;
        self.write()
    }

    pub fn mark_done(&mut self) -> Result<()> {
        self.record.status = SystemEventStatus::Done;
        self.write()
    }

    pub fn mark_failed(&mut self) -> Result<()> {
        self.record.status = SystemEventStatus::Failed;
        self.write()
    }

    pub fn cleanup(self) -> Result<()> {
        if self.path.exists() {
            fs::remove_file(&self.path)
                .with_context(|| format!("failed to remove event file {}", self.path.display()))?;
        }
        Ok(())
    }

    fn write(&mut self) -> Result<()> {
        let payload =
            serde_json::to_vec_pretty(&self.record).context("failed to serialize event record")?;
        fs::write(&self.path, payload)
            .with_context(|| format!("failed to write event file {}", self.path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_workspace(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rikabot-system-events-{name}-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp workspace");
        dir
    }

    fn read_status(path: &Path) -> SystemEventStatus {
        let text = fs::read_to_string(path).expect("read event file");
        let parsed: SystemEventRecord = serde_json::from_str(&text).expect("parse event record");
        parsed.status
    }

    #[test]
    fn persists_pending_event_on_create() {
        let workspace = temp_workspace("pending");
        let handle = SystemEventHandle::create(&workspace, "done", "sid", "name").expect("create");

        assert!(handle.path().exists());
        assert_eq!(read_status(handle.path()), SystemEventStatus::Pending);
    }

    #[test]
    fn lifecycle_updates_and_cleanup() {
        let workspace = temp_workspace("lifecycle");
        let mut handle =
            SystemEventHandle::create(&workspace, "done", "sid", "name").expect("create");
        let path = handle.path().to_path_buf();

        handle.mark_running().expect("mark running");
        assert_eq!(read_status(&path), SystemEventStatus::Running);

        handle.mark_done().expect("mark done");
        assert_eq!(read_status(&path), SystemEventStatus::Done);

        handle.cleanup().expect("cleanup");
        assert!(!path.exists());
    }
}
