use super::*;
use std::path::PathBuf;
use std::time::Duration;
use tokio::process::Command;

/// Shell command execution tool.
///
/// Runs arbitrary shell commands via `sh -c` with a configurable timeout.
/// Captures both stdout and stderr, truncating excessively long output
/// to prevent memory issues.
pub struct ShellTool {
    timeout_secs: u64,
    workspace_dir: Option<PathBuf>,
}

impl ShellTool {
    /// Create a new ShellTool with the given timeout in seconds.
    pub fn new(timeout_secs: u64) -> Self {
        Self {
            timeout_secs,
            workspace_dir: None,
        }
    }

    /// Create a ShellTool anchored to a workspace directory.
    pub fn with_workspace_dir(timeout_secs: u64, workspace_dir: PathBuf) -> Self {
        Self {
            timeout_secs,
            workspace_dir: Some(workspace_dir),
        }
    }
}

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str {
        "shell"
    }

    fn description(&self) -> &str {
        "Execute a shell command and return its output. Commands run from the workspace root."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute"
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

        tracing::info!("Executing shell command: {}", command);

        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command);
        if let Some(workspace_dir) = &self.workspace_dir {
            cmd.current_dir(workspace_dir);
        }

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
                let truncated = if combined.len() > 10_000 {
                    format!(
                        "{}...\n[output truncated, {} bytes total]",
                        &combined[..10_000],
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
}
