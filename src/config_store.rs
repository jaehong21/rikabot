use anyhow::Result;
use std::path::PathBuf;
use toml_edit::{value, Array, DocumentMut, Item, Table};

use crate::config::PermissionsConfig;

#[derive(Debug, Clone)]
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn save_permissions(&self, permissions: &PermissionsConfig) -> Result<()> {
        let existing = std::fs::read_to_string(&self.path).unwrap_or_default();
        let mut doc: DocumentMut = if existing.trim().is_empty() {
            DocumentMut::new()
        } else {
            existing
                .parse::<DocumentMut>()
                .map_err(|e| anyhow::anyhow!("failed to parse config TOML: {}", e))?
        };

        if !doc["permissions"].is_table() {
            doc["permissions"] = Item::Table(Table::new());
        }
        doc["permissions"]["enabled"] = value(permissions.enabled);

        if !doc["permissions"]["tools"].is_table() {
            doc["permissions"]["tools"] = Item::Table(Table::new());
        }

        let mut allow = Array::new();
        for rule in &permissions.tools.allow {
            allow.push(rule.as_str());
        }
        doc["permissions"]["tools"]["allow"] = value(allow);

        let mut deny = Array::new();
        for rule in &permissions.tools.deny {
            deny.push(rule.as_str());
        }
        doc["permissions"]["tools"]["deny"] = value(deny);

        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }

        let tmp_path = self.path.with_extension("tmp");
        std::fs::write(&tmp_path, doc.to_string())?;
        std::fs::rename(&tmp_path, &self.path)?;
        Ok(())
    }
}
