# Skills System

## Context

Rikabot has a static system prompt that doesn't adapt to available capabilities. The Skills feature adds a **markdown-based knowledge injection system** inspired by nanobot's architecture, implemented in Rust following zeroclaw patterns. Skills are instructional markdown documents (SKILL.md) with YAML frontmatter that teach the agent when and how to use its existing tools.

**Key distinction**: Tools = executable capabilities (code), Skills = knowledge/guidance (markdown). The agent already has the `shell` tool; skills tell it _what commands to run and when_.

## Architecture Overview

```
skills/                          # Built-in skills (embedded via include_str!)
  github/SKILL.md                # gh CLI patterns (requires: gh)

~/.rika/workspace/skills/        # User workspace skills (filesystem, runtime)
  custom-skill/SKILL.md

src/skills/mod.rs                # SkillMeta, Skill, SkillsLoader, prompt builder
```

### Progressive Loading

Following nanobot's progressive loading model:

- **Always-loaded skills** (`always: true`): Full SKILL.md body injected into system prompt
- **On-demand skills**: Only name + description summary shown in prompt; agent reads full content via shell tool (`cat <path>`) when triggered
- **Unavailable skills** (missing requirements): Listed with `available="false"` so agent knows they exist but can't use them yet

### SKILL.md Format

Each skill is a directory containing a `SKILL.md` file with YAML frontmatter:

```yaml
---
name: github
description: "Interact with GitHub using the gh CLI"
always: false
requires:
  bins: ["gh"]
  env: []
---
# GitHub Skill

[markdown instructions for the agent...]
```

**Frontmatter fields:**

| Field           | Type     | Default  | Description                                   |
| --------------- | -------- | -------- | --------------------------------------------- |
| `name`          | string   | required | Skill identifier                              |
| `description`   | string   | required | What this skill does (shown in summary)       |
| `always`        | bool     | `false`  | If true, full body is always in system prompt |
| `requires.bins` | string[] | `[]`     | Binaries that must be on PATH                 |
| `requires.env`  | string[] | `[]`     | Environment variables that must be set        |

## Core Types

### `src/skills/mod.rs`

```rust
pub struct SkillMeta {
    pub name: String,
    pub description: String,
    pub always: bool,
    pub requires: SkillRequirements,
}

pub struct SkillRequirements {
    pub bins: Vec<String>,
    pub env: Vec<String>,
}

pub struct Skill {
    pub meta: SkillMeta,
    pub body: String,           // markdown content without frontmatter
    pub source: SkillSource,    // Builtin or Workspace(PathBuf)
    pub available: bool,        // requirements met?
}

pub enum SkillSource {
    Builtin,
    Workspace(PathBuf),
}

pub struct SkillsLoader {
    workspace_dir: Option<PathBuf>,
}
```

### SkillsLoader API

```rust
impl SkillsLoader {
    pub fn new(workspace_dir: Option<PathBuf>) -> Self;
    pub fn load_all(&self) -> Vec<Skill>;
    pub fn build_prompt_section(&self) -> String;
}
```

### Helper Functions

- `parse_frontmatter(content) -> Option<(SkillMeta, String)>` — split YAML frontmatter from body
- `check_requirements(reqs) -> bool` — verify bins on PATH + env vars set
- `which(binary) -> bool` — check if binary exists on PATH

## Configuration

### `src/config.rs`

`workspace_dir` is a top-level field on `AppConfig`, not nested under `SkillsConfig`:

```rust
pub struct AppConfig {
    // ...other fields...
    /// Optional path to workspace directory. Defaults to ~/.rika/workspace
    #[serde(default)]
    pub workspace_dir: Option<String>,
    #[serde(default)]
    pub skills: SkillsConfig,
}

pub struct SkillsConfig {
    pub enabled: bool,          // default: true
}
```

Skills are located at `{workspace_dir}/skills/`.

### `config.toml`

```toml
workspace_dir = "/path/to/custom/workspace"

[skills]
# enabled = true
```

## Integration

Skills integrate at the system prompt level in `main.rs`. No changes to Agent struct or agent loop — the agent just receives a richer system prompt. The `workspace_dir` is resolved from the top-level config field, defaulting to `~/.rika/workspace`, and skills are loaded from `{workspace_dir}/skills/`.

```rust
let skills_prompt = if config.skills.enabled {
    let workspace_dir = config.workspace_dir.as_deref()
        .map(PathBuf::from)
        .or_else(|| std::env::var("HOME").ok()
            .map(|h| PathBuf::from(h).join(".rika").join("workspace")));
    let skills_dir = workspace_dir.map(|w| w.join("skills"));
    let loader = skills::SkillsLoader::new(skills_dir);
    loader.build_prompt_section()
} else {
    String::new()
};

let system_prompt = format!("{}\n\n{}", config.system_prompt, skills_prompt);
```

## Generated Prompt Format

```
You are Rika, a helpful personal AI assistant.

---

# Skills

## Available Skills

To use a skill, read its full instructions: `cat <path>`

<skills>
  <skill available="true">
    <name>github</name>
    <description>Interact with GitHub using the gh CLI</description>
    <path>/Users/x/.rika/workspace/skills/github/SKILL.md</path>
  </skill>
</skills>
```

## Built-in Skills

| Skill    | Purpose                                      | Always | Requires    |
| -------- | -------------------------------------------- | ------ | ----------- |
| `github` | GitHub CLI integration for issues, PRs, runs | no     | `gh` binary |

Built-in skills are embedded via `include_str!` (same pattern as `web/index.html`). Workspace skills override built-in skills with the same name.

## Dependencies

No new crate dependencies. Uses existing: `serde`, `serde_json`, `std::fs`, `std::env`.

## Verification

1. `cargo build` compiles
2. `cargo test` — unit tests for frontmatter parsing, requirements checking, prompt building
3. Run `cargo run` — verify skills section appears in agent behavior
4. Chat: "what skills do you have?" — agent lists available skills
5. Workspace override: create `~/.rika/workspace/skills/test/SKILL.md`, verify it loads
6. Requirement filtering: skill requiring missing binary shows `available="false"`
