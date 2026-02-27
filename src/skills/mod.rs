use std::collections::HashMap;
use std::path::PathBuf;

// ── Types ───────────────────────────────────────────────────────────────────

/// Metadata parsed from SKILL.md YAML frontmatter.
#[derive(Debug, Clone)]
pub struct SkillMeta {
    pub name: String,
    pub description: String,
    /// If true, the full skill body is always loaded into the system prompt.
    pub always: bool,
    /// Requirement constraints (binaries on PATH, env vars).
    pub requires: SkillRequirements,
}

/// Requirements that must be met for a skill to be available.
#[derive(Debug, Clone, Default)]
pub struct SkillRequirements {
    pub bins: Vec<String>,
    pub env: Vec<String>,
}

/// Where a skill was loaded from.
#[derive(Debug, Clone)]
pub enum SkillSource {
    Workspace(PathBuf),
}

/// A loaded skill with its metadata, content, and availability status.
#[derive(Debug, Clone)]
pub struct Skill {
    pub meta: SkillMeta,
    /// The full markdown body (without frontmatter).
    pub body: String,
    /// Where this skill was loaded from.
    pub source: SkillSource,
    /// Whether all requirements are met.
    pub available: bool,
}

// ── SkillsLoader ────────────────────────────────────────────────────────────

/// Discovers and loads skills from the workspace directory.
pub struct SkillsLoader {
    workspace_dir: Option<PathBuf>,
}

impl SkillsLoader {
    pub fn new(workspace_dir: Option<PathBuf>) -> Self {
        Self { workspace_dir }
    }

    /// Load all skills from the workspace directory.
    pub fn load_all(&self) -> Vec<Skill> {
        let mut skills_map: HashMap<String, Skill> = HashMap::new();

        // Load workspace skills
        if let Some(ref workspace) = self.workspace_dir {
            if workspace.is_dir() {
                if let Ok(entries) = std::fs::read_dir(workspace) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if !path.is_dir() {
                            continue;
                        }
                        let skill_file = path.join("SKILL.md");
                        if !skill_file.is_file() {
                            continue;
                        }
                        match std::fs::read_to_string(&skill_file) {
                            Ok(content) => {
                                if let Some(skill) =
                                    parse_skill(&content, SkillSource::Workspace(skill_file))
                                {
                                    tracing::info!("Loaded workspace skill: {}", skill.meta.name);
                                    skills_map.insert(skill.meta.name.clone(), skill);
                                }
                            }
                            Err(e) => {
                                tracing::warn!("Failed to read {:?}: {}", skill_file, e);
                            }
                        }
                    }
                }
            }
        }

        let mut skills: Vec<Skill> = skills_map.into_values().collect();
        skills.sort_by(|a, b| a.meta.name.cmp(&b.meta.name));
        skills
    }

    /// Build the skills section for the system prompt.
    ///
    /// - Always-loaded skills: full body injected
    /// - Other skills: XML summary with name, description, path, availability
    pub fn build_prompt_section(&self) -> String {
        let skills = self.load_all();
        if skills.is_empty() {
            return String::new();
        }

        let mut parts: Vec<String> = Vec::new();
        parts.push("# Skills".to_string());

        // Inline skills: always-loaded and available.
        let inline_skills: Vec<&Skill> = skills
            .iter()
            .filter(|s| s.available && s.meta.always)
            .collect();

        if !inline_skills.is_empty() {
            parts.push("\n## Active Skills\n".to_string());
            for skill in &inline_skills {
                parts.push(format!("### {}\n\n{}", skill.meta.name, skill.body));
            }
        }

        // On-demand skills (have a real file path the agent can read on demand)
        let on_demand_skills: Vec<&Skill> = skills
            .iter()
            .filter(|s| !inline_skills.iter().any(|i| i.meta.name == s.meta.name))
            .collect();

        if !on_demand_skills.is_empty() {
            parts.push("\n## Available Skills\n".to_string());
            parts.push(
                "To use a skill, read its full instructions with the filesystem tool: `filesystem_read`\n"
                    .to_string(),
            );
            parts.push("<skills>".to_string());

            for skill in &on_demand_skills {
                let path_attr = match &skill.source {
                    SkillSource::Workspace(p) => format!("\n    <path>{}</path>", p.display()),
                };

                let missing = if !skill.available {
                    let missing_bins: Vec<&str> = skill
                        .meta
                        .requires
                        .bins
                        .iter()
                        .filter(|b| !which(b))
                        .map(|s| s.as_str())
                        .collect();
                    let missing_env: Vec<&str> = skill
                        .meta
                        .requires
                        .env
                        .iter()
                        .filter(|e| std::env::var(e).is_err())
                        .map(|s| s.as_str())
                        .collect();
                    let mut missing_parts = Vec::new();
                    if !missing_bins.is_empty() {
                        missing_parts.push(format!("bins: {}", missing_bins.join(", ")));
                    }
                    if !missing_env.is_empty() {
                        missing_parts.push(format!("env: {}", missing_env.join(", ")));
                    }
                    format!(" missing=\"{}\"", missing_parts.join("; "))
                } else {
                    String::new()
                };

                parts.push(format!(
                    "  <skill available=\"{}\"{}>\n    <name>{}</name>\n    <description>{}</description>{}",
                    skill.available,
                    missing,
                    skill.meta.name,
                    skill.meta.description,
                    path_attr,
                ));
                parts.push("  </skill>".to_string());
            }

            parts.push("</skills>".to_string());
        }

        parts.join("\n")
    }
}

// ── Parsing helpers ─────────────────────────────────────────────────────────

/// Parse a SKILL.md file's content into a Skill.
fn parse_skill(content: &str, source: SkillSource) -> Option<Skill> {
    let (meta, body) = parse_frontmatter(content)?;
    let available = check_requirements(&meta.requires);
    Some(Skill {
        meta,
        body,
        source,
        available,
    })
}

/// Parse YAML frontmatter from a SKILL.md file.
///
/// Expects the format:
/// ```text
/// ---
/// name: skill-name
/// description: "what it does"
/// always: true
/// requires:
///   bins: ["git", "gh"]
///   env: ["GITHUB_TOKEN"]
/// ---
///
/// # Markdown body...
/// ```
fn parse_frontmatter(content: &str) -> Option<(SkillMeta, String)> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }

    // Find the closing ---
    let after_open = &trimmed[3..].trim_start_matches('\r');
    let after_open = after_open.strip_prefix('\n')?;

    let end_idx = after_open.find("\n---")?;
    let yaml_block = &after_open[..end_idx];
    let body = after_open[end_idx + 4..]
        .trim_start_matches('\r')
        .trim_start_matches('\n')
        .to_string();

    // Parse the YAML block as simple key-value pairs
    let mut name: Option<String> = None;
    let mut description: Option<String> = None;
    let mut always = false;
    let mut bins: Vec<String> = Vec::new();
    let mut env_vars: Vec<String> = Vec::new();

    let mut in_requires = false;

    for line in yaml_block.lines() {
        let trimmed_line = line.trim();

        // Top-level key: value
        if !line.starts_with(' ') && !line.starts_with('\t') && line.contains(':') {
            in_requires = trimmed_line.starts_with("requires:");

            if let Some((key, value)) = trimmed_line.split_once(':') {
                let key = key.trim();
                let value = value.trim().trim_matches('"').trim_matches('\'');

                match key {
                    "name" => name = Some(value.to_string()),
                    "description" => description = Some(value.to_string()),
                    "always" => always = value == "true",
                    _ => {}
                }
            }
        } else if in_requires {
            // Indented line under requires:
            if let Some((key, value)) = trimmed_line.split_once(':') {
                let key = key.trim();
                let value = value.trim();
                let items = parse_yaml_array(value);
                match key {
                    "bins" => bins = items,
                    "env" => env_vars = items,
                    _ => {}
                }
            }
        }
    }

    let name = name?;

    Some((
        SkillMeta {
            name,
            description: description.unwrap_or_default(),
            always,
            requires: SkillRequirements {
                bins,
                env: env_vars,
            },
        },
        body,
    ))
}

/// Parse a simple YAML inline array like `["git", "gh"]` into a Vec<String>.
fn parse_yaml_array(value: &str) -> Vec<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "[]" {
        return Vec::new();
    }

    // Handle ["item1", "item2"] format
    let inner = trimmed.trim_start_matches('[').trim_end_matches(']').trim();
    if inner.is_empty() {
        return Vec::new();
    }

    inner
        .split(',')
        .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Check if all requirements (binaries on PATH, env vars set) are satisfied.
fn check_requirements(reqs: &SkillRequirements) -> bool {
    for bin in &reqs.bins {
        if !which(bin) {
            return false;
        }
    }
    for env in &reqs.env {
        if std::env::var(env).is_err() {
            return false;
        }
    }
    true
}

/// Check if a binary is available on PATH.
fn which(binary: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| {
                let full = dir.join(binary);
                full.is_file() || full.with_extension("exe").is_file()
            })
        })
        .unwrap_or(false)
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_temp_skills_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        std::env::temp_dir().join(format!("rikabot_test_skills_{name}_{nonce}"))
    }

    #[test]
    fn test_parse_frontmatter_basic() {
        let content = r#"---
name: test-skill
description: "A test skill"
always: true
---

# Test Skill

Some body content here.
"#;
        let (meta, body) = parse_frontmatter(content).unwrap();
        assert_eq!(meta.name, "test-skill");
        assert_eq!(meta.description, "A test skill");
        assert!(meta.always);
        assert!(meta.requires.bins.is_empty());
        assert!(meta.requires.env.is_empty());
        assert!(body.contains("# Test Skill"));
        assert!(body.contains("Some body content here."));
    }

    #[test]
    fn test_parse_frontmatter_with_requires() {
        let content = r#"---
name: github
description: "GitHub CLI"
requires:
  bins: ["gh", "git"]
  env: ["GITHUB_TOKEN"]
---

# GitHub
"#;
        let (meta, body) = parse_frontmatter(content).unwrap();
        assert_eq!(meta.name, "github");
        assert!(!meta.always);
        assert_eq!(meta.requires.bins, vec!["gh", "git"]);
        assert_eq!(meta.requires.env, vec!["GITHUB_TOKEN"]);
        assert!(body.contains("# GitHub"));
    }

    #[test]
    fn test_parse_frontmatter_no_frontmatter() {
        let content = "# Just a markdown file\n\nNo frontmatter here.";
        assert!(parse_frontmatter(content).is_none());
    }

    #[test]
    fn test_parse_frontmatter_missing_name() {
        let content = r#"---
description: "No name field"
---

Body
"#;
        assert!(parse_frontmatter(content).is_none());
    }

    #[test]
    fn test_parse_yaml_array() {
        assert_eq!(parse_yaml_array(r#"["gh", "git"]"#), vec!["gh", "git"]);
        assert_eq!(parse_yaml_array(r#"["single"]"#), vec!["single"]);
        assert!(parse_yaml_array("[]").is_empty());
        assert!(parse_yaml_array("").is_empty());
    }

    #[test]
    fn test_which_finds_common_binary() {
        // `sh` should exist on any Unix system
        assert!(which("sh"));
    }

    #[test]
    fn test_which_missing_binary() {
        assert!(!which("nonexistent_binary_xyz_12345"));
    }

    #[test]
    fn test_check_requirements_no_requirements() {
        let reqs = SkillRequirements::default();
        assert!(check_requirements(&reqs));
    }

    #[test]
    fn test_check_requirements_missing_bin() {
        let reqs = SkillRequirements {
            bins: vec!["nonexistent_binary_xyz_12345".to_string()],
            env: vec![],
        };
        assert!(!check_requirements(&reqs));
    }

    #[test]
    fn test_check_requirements_missing_env() {
        let reqs = SkillRequirements {
            bins: vec![],
            env: vec!["NONEXISTENT_ENV_VAR_XYZ_12345".to_string()],
        };
        assert!(!check_requirements(&reqs));
    }

    #[test]
    fn test_skills_loader_without_workspace_dir() {
        let loader = SkillsLoader::new(None);
        let skills = loader.load_all();
        assert!(skills.is_empty());
    }

    #[test]
    fn test_build_prompt_section_empty_without_workspace_dir() {
        let loader = SkillsLoader::new(None);
        let section = loader.build_prompt_section();
        assert!(section.is_empty());
    }

    #[test]
    fn test_workspace_skill_loads() {
        let tmp = make_temp_skills_dir("workspace_skill_loads");
        let skill_dir = tmp.join("github");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            r#"---
name: github
description: "Custom github skill"
always: true
---

# Custom GitHub
"#,
        )
        .unwrap();

        let loader = SkillsLoader::new(Some(tmp.clone()));
        let skills = loader.load_all();
        let github = skills.iter().find(|s| s.meta.name == "github").unwrap();

        assert_eq!(github.meta.description, "Custom github skill");
        assert!(github.meta.always);
        assert!(matches!(github.source, SkillSource::Workspace(_)));

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_prompt_uses_filesystem_read_instructions_for_on_demand_skills() {
        let tmp = make_temp_skills_dir("filesystem_read_prompt");
        let skill_dir = tmp.join("github");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            r#"---
name: github
description: "Custom github skill"
always: false
---

# Custom GitHub
"#,
        )
        .unwrap();

        let loader = SkillsLoader::new(Some(tmp.clone()));
        let prompt = loader.build_prompt_section();

        assert!(prompt.contains("filesystem_read"));
        assert!(!prompt.contains("cat <path>"));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
