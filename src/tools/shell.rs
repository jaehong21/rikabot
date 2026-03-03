use super::*;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

/// Shell command execution tool.
///
/// Runs arbitrary shell commands via `sh -c` with a configurable timeout.
/// Captures both stdout and stderr, truncating excessively long output
/// to prevent memory issues.
pub struct ShellTool {
    timeout_secs: u64,
    max_output_bytes: usize,
    workspace_dir: Option<PathBuf>,
}

impl ShellTool {
    /// Create a new ShellTool with the given timeout in seconds.
    pub fn new(timeout_secs: u64) -> Self {
        Self::with_limits(timeout_secs, 10_000)
    }

    /// Create a new ShellTool with timeout and output limit.
    pub fn with_limits(timeout_secs: u64, max_output_bytes: usize) -> Self {
        Self {
            timeout_secs,
            max_output_bytes,
            workspace_dir: None,
        }
    }

    /// Create a ShellTool anchored to a workspace directory.
    pub fn with_workspace_dir(timeout_secs: u64, workspace_dir: PathBuf) -> Self {
        Self::with_workspace_dir_and_limits(timeout_secs, 10_000, workspace_dir)
    }

    /// Create a ShellTool anchored to a workspace directory with explicit output limit.
    pub fn with_workspace_dir_and_limits(
        timeout_secs: u64,
        max_output_bytes: usize,
        workspace_dir: PathBuf,
    ) -> Self {
        Self {
            timeout_secs,
            max_output_bytes,
            workspace_dir: Some(workspace_dir),
        }
    }
}

pub fn resolve_effective_path(
    workspace_dir: Option<&Path>,
    requested_path: Option<&str>,
) -> Result<PathBuf> {
    let requested_path = requested_path.map(str::trim).filter(|raw| !raw.is_empty());
    let default_dir = workspace_dir
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok());

    let unresolved = match requested_path {
        Some(raw) => {
            let path = PathBuf::from(raw);
            if path.is_absolute() {
                path
            } else {
                let workspace = default_dir.as_ref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "Missing base directory to resolve relative shell path '{}'",
                        raw
                    )
                })?;
                workspace.join(path)
            }
        }
        None => {
            let workspace = default_dir.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "Missing workspace_dir (or current directory) for shell path default"
                )
            })?;
            workspace.to_path_buf()
        }
    };

    let effective = std::fs::canonicalize(&unresolved).map_err(|e| {
        anyhow::anyhow!(
            "Failed to resolve shell path '{}': {}",
            unresolved.display(),
            e
        )
    })?;

    if !effective.is_dir() {
        anyhow::bail!("Shell path '{}' is not a directory", effective.display());
    }

    Ok(effective)
}

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str {
        "shell"
    }

    fn description(&self) -> &str {
        "Execute a shell command and return its output. Use `path` to choose the working directory (do not use `cd ...` or `git -C ...` for that)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute. Keep directory selection out of command text."
                },
                "path": {
                    "type": "string",
                    "description": "Execution directory. If omitted, workspace_dir is used. Prefer this over `cd ...` or `git -C ...`."
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'command' argument"))?;
        let requested_path = match args.get("path") {
            Some(value) => Some(
                value
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("'path' must be a string"))?,
            ),
            None => None,
        };
        let effective_path =
            match resolve_effective_path(self.workspace_dir.as_deref(), requested_path) {
                Ok(path) => path,
                Err(err) => {
                    return Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some(err.to_string()),
                    });
                }
            };

        tracing::info!(
            "Executing shell command: {} (path: {})",
            command,
            effective_path.display()
        );

        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command);
        cmd.current_dir(&effective_path);

        let result =
            tokio::time::timeout(Duration::from_secs(self.timeout_secs), cmd.output()).await;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let combined = if stderr.is_empty() {
                    stdout.to_string()
                } else if stdout.is_empty() {
                    stderr.to_string()
                } else {
                    format!("{}\n{}", stdout, stderr)
                };

                // Truncate very long output to prevent memory issues.
                let truncated = if combined.len() > self.max_output_bytes {
                    format!(
                        "{}...\n[output truncated, {} bytes total]",
                        &combined[..self.max_output_bytes],
                        combined.len()
                    )
                } else {
                    combined
                };

                Ok(ToolResult {
                    success: output.status.success(),
                    output: truncated,
                    error: if output.status.success() {
                        None
                    } else {
                        Some(format!("Exit code: {}", output.status.code().unwrap_or(-1)))
                    },
                })
            }
            Ok(Err(e)) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Failed to execute command: {}", e)),
            }),
            Err(_) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!(
                    "Command timed out after {} seconds",
                    self.timeout_secs
                )),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        std::env::temp_dir().join(format!("rikabot_test_shell_{name}_{nonce}"))
    }

    #[test]
    fn shell_tool_name() {
        let tool = ShellTool::new(30);
        assert_eq!(tool.name(), "shell");
    }

    #[test]
    fn shell_tool_description_not_empty() {
        let tool = ShellTool::new(30);
        assert!(!tool.description().is_empty());
    }

    #[test]
    fn shell_tool_schema_has_command() {
        let tool = ShellTool::new(30);
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["command"].is_object());
        assert!(schema["properties"]["path"].is_object());
        assert!(schema["required"]
            .as_array()
            .expect("required should be an array")
            .contains(&serde_json::json!("command")));
    }

    #[test]
    fn shell_tool_spec_is_consistent() {
        let tool = ShellTool::new(30);
        let spec = tool.spec();
        assert_eq!(spec.name, "shell");
        assert_eq!(spec.description, tool.description());
        assert_eq!(spec.parameters, tool.parameters_schema());
    }

    #[tokio::test]
    async fn shell_executes_echo() {
        let tool = ShellTool::new(30);
        let result = tool
            .execute(serde_json::json!({"command": "echo hello"}))
            .await
            .expect("echo command should succeed");
        assert!(result.success);
        assert!(result.output.trim().contains("hello"));
        assert!(result.error.is_none());
    }

    #[tokio::test]
    async fn shell_captures_exit_code() {
        let tool = ShellTool::new(30);
        let result = tool
            .execute(serde_json::json!({"command": "exit 42"}))
            .await
            .expect("exit command should return a result");
        assert!(!result.success);
        assert!(result
            .error
            .as_deref()
            .unwrap_or("")
            .contains("Exit code: 42"));
    }

    #[tokio::test]
    async fn shell_captures_stderr() {
        let tool = ShellTool::new(30);
        let result = tool
            .execute(serde_json::json!({"command": "echo error_msg >&2"}))
            .await
            .expect("stderr command should return a result");
        assert!(result.output.contains("error_msg"));
    }

    #[tokio::test]
    async fn shell_missing_command_param() {
        let tool = ShellTool::new(30);
        let result = tool.execute(serde_json::json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("command"));
    }

    #[tokio::test]
    async fn shell_wrong_type_param() {
        let tool = ShellTool::new(30);
        let result = tool.execute(serde_json::json!({"command": 123})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn shell_handles_nonexistent_command() {
        let tool = ShellTool::new(30);
        let result = tool
            .execute(serde_json::json!({"command": "nonexistent_binary_xyz_12345"}))
            .await
            .expect("nonexistent command should return a result");
        assert!(!result.success);
    }

    #[tokio::test]
    async fn shell_resolves_relative_commands_from_workspace_dir() {
        let workspace = make_temp_dir("workspace_relative");
        tokio::fs::create_dir_all(&workspace).await.unwrap();

        let tool = ShellTool::with_workspace_dir(30, workspace.clone());
        let result = tool
            .execute(serde_json::json!({
                "command": "pwd"
            }))
            .await
            .expect("pwd should succeed");

        assert!(result.success);
        let output = result.output.trim();
        let actual = tokio::fs::canonicalize(PathBuf::from(output))
            .await
            .unwrap();
        let expected = tokio::fs::canonicalize(&workspace).await.unwrap();
        assert_eq!(actual, expected);

        let _ = tokio::fs::remove_dir_all(workspace).await;
    }

    #[tokio::test]
    async fn shell_uses_relative_path_argument_from_workspace() {
        let workspace = make_temp_dir("workspace_with_relative_path_arg");
        let nested = workspace.join("nested");
        tokio::fs::create_dir_all(&nested).await.unwrap();

        let tool = ShellTool::with_workspace_dir(30, workspace.clone());
        let result = tool
            .execute(serde_json::json!({
                "command": "pwd",
                "path": "nested"
            }))
            .await
            .expect("pwd should succeed");

        assert!(result.success);
        let output = result.output.trim();
        let actual = tokio::fs::canonicalize(PathBuf::from(output))
            .await
            .unwrap();
        let expected = tokio::fs::canonicalize(&nested).await.unwrap();
        assert_eq!(actual, expected);

        let _ = tokio::fs::remove_dir_all(workspace).await;
    }

    #[tokio::test]
    async fn shell_uses_absolute_path_argument() {
        let absolute = make_temp_dir("workspace_absolute_path_arg");
        tokio::fs::create_dir_all(&absolute).await.unwrap();

        let tool = ShellTool::new(30);
        let result = tool
            .execute(serde_json::json!({
                "command": "pwd",
                "path": absolute.to_string_lossy()
            }))
            .await
            .expect("pwd should succeed");

        assert!(result.success);
        let output = result.output.trim();
        let actual = tokio::fs::canonicalize(PathBuf::from(output))
            .await
            .unwrap();
        let expected = tokio::fs::canonicalize(&absolute).await.unwrap();
        assert_eq!(actual, expected);

        let _ = tokio::fs::remove_dir_all(absolute).await;
    }

    #[tokio::test]
    async fn shell_empty_path_falls_back_to_workspace_dir() {
        let workspace = make_temp_dir("workspace_empty_path_fallback");
        tokio::fs::create_dir_all(&workspace).await.unwrap();

        let tool = ShellTool::with_workspace_dir(30, workspace.clone());
        let result = tool
            .execute(serde_json::json!({
                "command": "pwd",
                "path": "   "
            }))
            .await
            .expect("pwd should succeed");

        assert!(result.success);
        let output = result.output.trim();
        let actual = tokio::fs::canonicalize(PathBuf::from(output))
            .await
            .unwrap();
        let expected = tokio::fs::canonicalize(&workspace).await.unwrap();
        assert_eq!(actual, expected);

        let _ = tokio::fs::remove_dir_all(workspace).await;
    }

    #[tokio::test]
    async fn shell_fails_when_path_is_missing_or_invalid() {
        let file_path = make_temp_dir("workspace_invalid_file_path");
        tokio::fs::write(&file_path, "x").await.unwrap();
        let tool = ShellTool::new(30);
        let result = tool
            .execute(serde_json::json!({
                "command": "pwd",
                "path": file_path.to_string_lossy()
            }))
            .await
            .expect("invalid path should return tool result");
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("not a directory"));

        let _ = tokio::fs::remove_file(file_path).await;
    }

    #[tokio::test]
    async fn shell_timeout_message_uses_configured_value() {
        let tool = ShellTool::new(1);
        let result = tool
            .execute(serde_json::json!({"command": "sleep 2"}))
            .await
            .expect("timeout should return tool result");

        assert!(!result.success);
        let error = result.error.unwrap_or_default();
        assert!(error.contains("timed out after 1 seconds"));
    }

    #[tokio::test]
    async fn shell_respects_custom_output_limit() {
        let tool = ShellTool::with_limits(30, 32);
        let result = tool
            .execute(serde_json::json!({
                "command": "printf 'abcdefghijklmnopqrstuvwxyz0123456789'"
            }))
            .await
            .expect("printf should succeed");

        assert!(result.success);
        assert!(result.output.contains("[output truncated"));
    }
}
