use anyhow::Result;
use regex::Regex;
use serde_json::Value;

use crate::config::PermissionsConfig;

const MAX_RULES_PER_LIST: usize = 500;
const MAX_RULE_LENGTH: usize = 1_024;

#[derive(Debug, Clone)]
pub struct PermissionEngine {
    enabled: bool,
    allow_rules: Vec<CompiledRule>,
    deny_rules: Vec<CompiledRule>,
}

#[derive(Debug, Clone)]
struct CompiledRule {
    raw: String,
    tool_name_matcher: WildcardMatcher,
    matcher: RuleMatcher,
}

#[derive(Debug, Clone)]
enum RuleMatcher {
    AnyArgs,
    Raw(WildcardMatcher),
    Selectors(Vec<SelectorMatcher>),
}

#[derive(Debug, Clone)]
struct SelectorMatcher {
    path: Vec<String>,
    pattern: WildcardMatcher,
}

#[derive(Debug, Clone)]
struct WildcardMatcher {
    regex: Regex,
}

#[derive(Debug, Clone)]
pub struct PermissionDecision {
    pub allowed: bool,
    pub reason: String,
}

impl PermissionDecision {
    fn allow() -> Self {
        Self {
            allowed: true,
            reason: "allowed".to_string(),
        }
    }

    fn deny(reason: impl Into<String>) -> Self {
        Self {
            allowed: false,
            reason: reason.into(),
        }
    }
}

impl PermissionEngine {
    pub fn disabled_allow_all() -> Self {
        Self {
            enabled: false,
            allow_rules: Vec::new(),
            deny_rules: Vec::new(),
        }
    }

    pub fn from_config(config: &PermissionsConfig) -> Result<Self> {
        if config.tools.allow.len() > MAX_RULES_PER_LIST {
            anyhow::bail!(
                "allow rules exceed maximum ({} > {})",
                config.tools.allow.len(),
                MAX_RULES_PER_LIST
            );
        }
        if config.tools.deny.len() > MAX_RULES_PER_LIST {
            anyhow::bail!(
                "deny rules exceed maximum ({} > {})",
                config.tools.deny.len(),
                MAX_RULES_PER_LIST
            );
        }

        let allow_rules = compile_rules(&config.tools.allow, "allow")?;
        let deny_rules = compile_rules(&config.tools.deny, "deny")?;

        Ok(Self {
            enabled: config.enabled,
            allow_rules,
            deny_rules,
        })
    }

    pub fn evaluate(&self, tool_name: &str, args: &Value) -> PermissionDecision {
        if !self.enabled {
            return PermissionDecision::allow();
        }

        let normalized_tool = normalize_tool_name(tool_name);

        if let Some(matched_rule) = self
            .deny_rules
            .iter()
            .find(|rule| rule.matches(&normalized_tool, tool_name, args))
        {
            return PermissionDecision::deny(format!(
                "Tool call blocked by deny rule `{}`",
                matched_rule.raw
            ));
        }

        if self.allow_rules.is_empty() {
            return PermissionDecision::deny(
                "Tool call blocked: no allow rules configured (default deny)",
            );
        }

        if self
            .allow_rules
            .iter()
            .any(|rule| rule.matches(&normalized_tool, tool_name, args))
        {
            return PermissionDecision::allow();
        }

        PermissionDecision::deny("Tool call blocked: no allow rule matched")
    }
}

impl CompiledRule {
    fn matches(&self, normalized_tool_name: &str, original_tool_name: &str, args: &Value) -> bool {
        if !self.tool_name_matcher.is_match(normalized_tool_name) {
            return false;
        }

        match &self.matcher {
            RuleMatcher::AnyArgs => true,
            RuleMatcher::Raw(pattern) => {
                let candidate = canonical_args_text(original_tool_name, args);
                pattern.is_match(&candidate)
            }
            RuleMatcher::Selectors(selectors) => {
                selectors.iter().all(|selector| selector.matches(args))
            }
        }
    }
}

impl SelectorMatcher {
    fn matches(&self, args: &Value) -> bool {
        let Some(value) = lookup_path(args, &self.path) else {
            return false;
        };
        let text = value_to_match_text(value);
        self.pattern.is_match(&text)
    }
}

impl WildcardMatcher {
    fn compile(pattern: &str) -> Result<Self> {
        if pattern.trim().is_empty() {
            anyhow::bail!("wildcard pattern cannot be empty");
        }

        let mut regex = String::from("^");
        for ch in pattern.chars() {
            if ch == '*' {
                regex.push_str(".*");
            } else {
                regex.push_str(&regex::escape(&ch.to_string()));
            }
        }
        regex.push('$');

        Ok(Self {
            regex: Regex::new(&regex)?,
        })
    }

    fn is_match(&self, value: &str) -> bool {
        self.regex.is_match(value)
    }
}

fn compile_rules(raw_rules: &[String], label: &str) -> Result<Vec<CompiledRule>> {
    let mut compiled = Vec::with_capacity(raw_rules.len());
    for (idx, raw_rule) in raw_rules.iter().enumerate() {
        let trimmed = raw_rule.trim();
        if trimmed.is_empty() {
            anyhow::bail!("{label} rule at index {idx} cannot be empty");
        }
        if trimmed.chars().count() > MAX_RULE_LENGTH {
            anyhow::bail!(
                "{label} rule at index {idx} exceeds max length of {}",
                MAX_RULE_LENGTH
            );
        }
        compiled.push(compile_rule(trimmed)?);
    }
    Ok(compiled)
}

fn compile_rule(raw: &str) -> Result<CompiledRule> {
    let open = raw
        .find('(')
        .ok_or_else(|| anyhow::anyhow!("invalid rule '{}': missing '('", raw))?;
    let close = raw
        .rfind(')')
        .ok_or_else(|| anyhow::anyhow!("invalid rule '{}': missing ')'", raw))?;
    if close <= open || close != raw.len() - 1 {
        anyhow::bail!("invalid rule '{}': malformed '(...)' section", raw);
    }

    let tool_name = raw[..open].trim();
    if tool_name.is_empty() {
        anyhow::bail!("invalid rule '{}': tool name is empty", raw);
    }

    let arg_expression = raw[open + 1..close].trim();
    if arg_expression.is_empty() {
        anyhow::bail!("invalid rule '{}': argument matcher is empty", raw);
    }

    let matcher = if arg_expression == "*" {
        RuleMatcher::AnyArgs
    } else if let Some(selectors) = try_parse_selectors(arg_expression)? {
        RuleMatcher::Selectors(selectors)
    } else {
        RuleMatcher::Raw(WildcardMatcher::compile(arg_expression)?)
    };

    Ok(CompiledRule {
        raw: raw.to_string(),
        tool_name_matcher: WildcardMatcher::compile(&normalize_tool_name(tool_name))?,
        matcher,
    })
}

fn try_parse_selectors(input: &str) -> Result<Option<Vec<SelectorMatcher>>> {
    let parts: Vec<&str> = input
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        return Ok(None);
    }

    // Selector mode requires every part to contain `:`.
    if !parts.iter().all(|part| part.contains(':')) {
        return Ok(None);
    }

    let mut selectors = Vec::with_capacity(parts.len());
    for part in parts {
        let Some((path_raw, pattern_raw)) = part.split_once(':') else {
            return Ok(None);
        };

        let path_raw = path_raw.trim();
        let pattern_raw = pattern_raw.trim();
        if path_raw.is_empty() || pattern_raw.is_empty() {
            return Ok(None);
        }

        let path = parse_selector_path(path_raw)?;
        let pattern = WildcardMatcher::compile(pattern_raw)?;
        selectors.push(SelectorMatcher { path, pattern });
    }

    Ok(Some(selectors))
}

fn parse_selector_path(path: &str) -> Result<Vec<String>> {
    let mut segments = Vec::new();
    for segment in path.split('.') {
        let segment = segment.trim();
        if segment.is_empty() {
            anyhow::bail!("selector path '{}' contains an empty segment", path);
        }
        if !segment
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
        {
            anyhow::bail!("selector path '{}' contains invalid characters", path);
        }
        segments.push(segment.to_string());
    }
    Ok(segments)
}

fn lookup_path<'a>(value: &'a Value, path: &[String]) -> Option<&'a Value> {
    let mut cursor = value;
    for segment in path {
        match cursor {
            Value::Object(map) => {
                cursor = map.get(segment)?;
            }
            Value::Array(list) => {
                let idx = segment.parse::<usize>().ok()?;
                cursor = list.get(idx)?;
            }
            _ => return None,
        }
    }
    Some(cursor)
}

fn value_to_match_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => "null".to_string(),
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn canonical_args_text(tool_name: &str, args: &Value) -> String {
    if tool_name.eq_ignore_ascii_case("shell") {
        return args
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
    }
    serde_json::to_string(args).unwrap_or_default()
}

fn normalize_tool_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{PermissionsConfig, ToolPermissionsConfig};

    fn cfg(enabled: bool, allow: &[&str], deny: &[&str]) -> PermissionsConfig {
        PermissionsConfig {
            enabled,
            tools: ToolPermissionsConfig {
                allow: allow.iter().map(|s| s.to_string()).collect(),
                deny: deny.iter().map(|s| s.to_string()).collect(),
            },
        }
    }

    #[test]
    fn denies_by_default_when_enabled_and_allow_empty() {
        let engine = PermissionEngine::from_config(&cfg(true, &[], &[])).unwrap();
        let decision = engine.evaluate("shell", &serde_json::json!({"command":"echo hi"}));
        assert!(!decision.allowed);
        assert!(decision.reason.contains("default deny"));
    }

    #[test]
    fn allow_rule_matches_shell_raw_pattern() {
        let engine =
            PermissionEngine::from_config(&cfg(true, &["shell(git commit *)"], &[])).unwrap();
        let allowed = engine.evaluate("shell", &serde_json::json!({"command":"git commit -m hi"}));
        let denied = engine.evaluate(
            "shell",
            &serde_json::json!({"command":"git push origin main"}),
        );
        assert!(allowed.allowed);
        assert!(!denied.allowed);
    }

    #[test]
    fn deny_rule_takes_precedence() {
        let engine =
            PermissionEngine::from_config(&cfg(true, &["shell(git *)"], &["shell(git push *)"]))
                .unwrap();
        let decision = engine.evaluate(
            "shell",
            &serde_json::json!({"command":"git push origin main"}),
        );
        assert!(!decision.allowed);
        assert!(decision.reason.contains("deny rule"));
    }

    #[test]
    fn structured_selector_matches() {
        let engine =
            PermissionEngine::from_config(&cfg(true, &["filesystem_read(path:docs/*)"], &[]))
                .unwrap();

        let allow = engine.evaluate(
            "filesystem_read",
            &serde_json::json!({"path":"docs/prd.md"}),
        );
        let deny = engine.evaluate(
            "filesystem_read",
            &serde_json::json!({"path":"src/main.rs"}),
        );

        assert!(allow.allowed);
        assert!(!deny.allowed);
    }

    #[test]
    fn wildcard_tool_pattern_is_supported() {
        let engine = PermissionEngine::from_config(&cfg(true, &["mcp_linear_*(*)"], &[])).unwrap();
        let allowed = engine.evaluate("mcp_linear__search_issues", &serde_json::json!({}));
        let denied = engine.evaluate("mcp_notion__search", &serde_json::json!({}));
        assert!(allowed.allowed);
        assert!(!denied.allowed);
    }
}
