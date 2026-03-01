use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::skills::SkillsLoader;

const DEFAULT_BOOTSTRAP_MAX_CHARS: usize = 20_000;
const DEFAULT_BOOTSTRAP_TOTAL_MAX_CHARS: usize = 150_000;

const REQUIRED_BOOTSTRAP_FILES: [(&str, &str); 6] = [
    ("AGENTS.md", AGENTS_TEMPLATE),
    ("SOUL.md", SOUL_TEMPLATE),
    ("TOOLS.md", TOOLS_TEMPLATE),
    ("IDENTITY.md", IDENTITY_TEMPLATE),
    ("USER.md", USER_TEMPLATE),
    ("HEARTBEAT.md", HEARTBEAT_TEMPLATE),
];

const OPTIONAL_BOOTSTRAP_FILES: [&str; 1] = ["MEMORY.md"];

const AGENTS_TEMPLATE: &str = include_str!("../../templates/AGENTS.md");
const SOUL_TEMPLATE: &str = include_str!("../../templates/SOUL.md");
const TOOLS_TEMPLATE: &str = include_str!("../../templates/TOOLS.md");
const IDENTITY_TEMPLATE: &str = include_str!("../../templates/IDENTITY.md");
const USER_TEMPLATE: &str = include_str!("../../templates/USER.md");
const HEARTBEAT_TEMPLATE: &str = include_str!("../../templates/HEARTBEAT.md");

#[derive(Debug, Clone, Copy)]
pub struct PromptLimits {
    pub bootstrap_max_chars: usize,
    pub bootstrap_total_max_chars: usize,
}

pub struct PromptManager {
    workspace_dir: PathBuf,
    skills_enabled: bool,
    limits: PromptLimits,
}

impl PromptManager {
    pub fn new(workspace_dir: &Path, skills_enabled: bool, limits: PromptLimits) -> Result<Self> {
        fs::create_dir_all(workspace_dir).with_context(|| {
            format!("failed to create workspace dir {}", workspace_dir.display())
        })?;

        let workspace_dir = workspace_dir
            .canonicalize()
            .unwrap_or_else(|_| workspace_dir.to_path_buf());

        let manager = Self {
            workspace_dir,
            skills_enabled,
            limits: PromptLimits {
                bootstrap_max_chars: sanitize_limit(
                    limits.bootstrap_max_chars,
                    DEFAULT_BOOTSTRAP_MAX_CHARS,
                ),
                bootstrap_total_max_chars: sanitize_limit(
                    limits.bootstrap_total_max_chars,
                    DEFAULT_BOOTSTRAP_TOTAL_MAX_CHARS,
                ),
            },
        };

        manager.ensure_workspace_layout()?;
        Ok(manager)
    }

    pub fn build_prompt(&self) -> Result<String> {
        let mut sections: Vec<String> = Vec::new();

        if self.skills_enabled {
            let skills_loader = SkillsLoader::new(Some(self.workspace_dir.join("skills")));
            let skills_section = skills_loader.build_prompt_section();
            if !skills_section.trim().is_empty() {
                sections.push(skills_section);
            }
        }

        sections.push(self.build_workspace_context());
        Ok(sections.join("\n\n---\n\n"))
    }

    fn ensure_workspace_layout(&self) -> Result<()> {
        for (name, template) in REQUIRED_BOOTSTRAP_FILES {
            write_file_if_missing(&self.workspace_dir.join(name), template)?;
        }

        fs::create_dir_all(self.workspace_dir.join("memory")).with_context(|| {
            format!(
                "failed to create memory dir {}",
                self.workspace_dir.join("memory").display()
            )
        })?;
        Ok(())
    }

    fn build_workspace_context(&self) -> String {
        let mut context = String::from(
            "# Project Context\n\nThe following workspace files have been loaded into context:\n\n",
        );
        let mut remaining = self.limits.bootstrap_total_max_chars;

        for name in REQUIRED_BOOTSTRAP_FILES
            .iter()
            .map(|(name, _)| *name)
            .chain(OPTIONAL_BOOTSTRAP_FILES.iter().copied())
        {
            if remaining == 0 {
                break;
            }

            let path = self.workspace_dir.join(name);
            let content = match load_bootstrap_content(
                &path,
                REQUIRED_BOOTSTRAP_FILES.iter().any(|(n, _)| *n == name),
            ) {
                Some(content) => content,
                None => continue,
            };

            let mut body = trim_to_chars(&content, self.limits.bootstrap_max_chars);
            let header = format!("## {}\n\n", path.display());
            let header_len = count_chars(&header);
            if header_len >= remaining {
                break;
            }

            let available_for_body = remaining.saturating_sub(header_len + 2);
            if available_for_body == 0 {
                break;
            }

            body = trim_to_chars(&body, available_for_body);
            if body.trim().is_empty() {
                continue;
            }

            let block = format!("{}{}\n\n", header, body);
            let block_len = count_chars(&block);
            if block_len > remaining {
                break;
            }
            context.push_str(&block);
            remaining = remaining.saturating_sub(block_len);
        }

        context
    }
}

fn sanitize_limit(value: usize, default_value: usize) -> usize {
    if value == 0 {
        default_value
    } else {
        value
    }
}

fn write_file_if_missing(path: &Path, content: &str) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    fs::write(path, content)
        .with_context(|| format!("failed to seed workspace file {}", path.display()))
}

fn load_bootstrap_content(path: &Path, required: bool) -> Option<String> {
    match fs::read_to_string(path) {
        Ok(content) => {
            let trimmed = content.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Err(err) if err.kind() == ErrorKind::NotFound && !required => None,
        Err(err) if err.kind() == ErrorKind::NotFound && required => {
            Some(format!("[MISSING] Expected file at {}", path.display()))
        }
        Err(err) => {
            tracing::warn!("failed to read bootstrap file {}: {}", path.display(), err);
            Some(format!(
                "[UNREADABLE] Could not read {}: {}",
                path.display(),
                err
            ))
        }
    }
}

fn trim_to_chars(input: &str, max_chars: usize) -> String {
    if count_chars(input) <= max_chars {
        return input.to_string();
    }
    input.chars().take(max_chars).collect()
}

fn count_chars(input: &str) -> usize {
    input.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn temp_workspace(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rikabot-prompt-{}-{}", name, Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp workspace");
        dir
    }

    fn manager(workspace: &Path, max: usize, total: usize) -> PromptManager {
        PromptManager::new(
            workspace,
            false,
            PromptLimits {
                bootstrap_max_chars: max,
                bootstrap_total_max_chars: total,
            },
        )
        .expect("create manager")
    }

    #[test]
    fn seeds_required_files_and_memory_directory() {
        let workspace = temp_workspace("seed");
        let _manager = manager(&workspace, 20_000, 150_000);

        for (name, _) in REQUIRED_BOOTSTRAP_FILES {
            assert!(
                workspace.join(name).exists(),
                "expected {} to be seeded",
                name
            );
        }
        assert!(workspace.join("memory").is_dir());
    }

    #[test]
    fn seeded_templates_include_bootstrap_placeholders_and_guidance() {
        let workspace = temp_workspace("template_contract");
        let _manager = manager(&workspace, 20_000, 150_000);

        let identity = fs::read_to_string(workspace.join("IDENTITY.md")).expect("read identity");
        assert!(!identity.contains("Name: Rika"));
        assert!(identity.contains("Name: TBD"));
        assert!(identity.contains("Role: TBD"));

        let user = fs::read_to_string(workspace.join("USER.md")).expect("read user");
        assert!(user.contains("Name: TBD"));
        assert!(user.contains("Preferred address: TBD"));
        assert!(user.contains("Timezone: TBD"));

        let agents = fs::read_to_string(workspace.join("AGENTS.md")).expect("read agents");
        assert!(agents.contains("First Run / Profile Bootstrap"));
        assert!(agents.contains("Soft gate behavior"));
        assert!(agents.contains("If the user says \"later\""));
    }

    #[test]
    fn does_not_overwrite_existing_file() {
        let workspace = temp_workspace("preserve");
        let agents = workspace.join("AGENTS.md");
        fs::write(&agents, "custom agents").expect("write custom");

        let _manager = manager(&workspace, 20_000, 150_000);
        let actual = fs::read_to_string(&agents).expect("read agents");
        assert_eq!(actual, "custom agents");
    }

    #[test]
    fn includes_missing_marker_for_required_files() {
        let workspace = temp_workspace("missing");
        let manager = manager(&workspace, 20_000, 150_000);
        fs::remove_file(workspace.join("SOUL.md")).expect("remove soul");

        let prompt = manager.build_prompt().expect("build prompt");
        assert!(prompt.contains("[MISSING] Expected file at"));
        assert!(prompt.contains("SOUL.md"));
    }

    #[test]
    fn auto_injects_memory_when_present() {
        let workspace = temp_workspace("memory");
        let manager = manager(&workspace, 20_000, 150_000);
        fs::write(workspace.join("MEMORY.md"), "Long-term facts").expect("write memory");

        let prompt = manager.build_prompt().expect("build prompt");
        assert!(prompt.contains("## "));
        assert!(prompt.contains("MEMORY.md"));
        assert!(prompt.contains("Long-term facts"));
    }

    #[test]
    fn applies_per_file_truncation_limit() {
        let workspace = temp_workspace("truncate");
        let manager = manager(&workspace, 32, 500);
        let large = "A".repeat(200);
        fs::write(workspace.join("AGENTS.md"), &large).expect("write large agents");

        let prompt = manager.build_prompt().expect("build prompt");
        assert!(prompt.contains(&"A".repeat(32)));
        assert!(!prompt.contains(&"A".repeat(64)));
    }
}
