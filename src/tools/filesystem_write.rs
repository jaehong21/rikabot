use super::*;
use std::path::PathBuf;

/// Filesystem file-write tool.
///
/// Writes full content to a file path, creating parent directories as needed.
pub struct FilesystemWriteTool {
    workspace_dir: Option<PathBuf>,
}

impl FilesystemWriteTool {
    pub fn new() -> Self {
        Self {
            workspace_dir: None,
        }
    }

    pub fn with_workspace_dir(workspace_dir: PathBuf) -> Self {
        Self {
            workspace_dir: Some(workspace_dir),
        }
    }

    fn resolve_path(&self, input: &str) -> PathBuf {
        let path = PathBuf::from(input);
        if path.is_absolute() {
            path
        } else if let Some(workspace_dir) = &self.workspace_dir {
            workspace_dir.join(path)
        } else {
            path
        }
    }
}

#[async_trait]
impl Tool for FilesystemWriteTool {
    fn name(&self) -> &str {
        "filesystem_write"
    }

    fn description(&self) -> &str {
        "Write full contents to a file. Creates parent directories when needed."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to write. Can be absolute or relative to the workspace root."
                },
                "content": {
                    "type": "string",
                    "description": "Full file content to write."
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'path' argument"))?;

        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'content' argument"))?;

        if path.trim().is_empty() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Path cannot be empty".to_string()),
            });
        }

        let path_buf = self.resolve_path(path);

        if let Ok(meta) = tokio::fs::metadata(&path_buf).await {
            if meta.is_dir() {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!(
                        "Path '{}' points to a directory, not a file",
                        path_buf.display()
                    )),
                });
            }
        }

        if let Some(parent) = path_buf.parent().filter(|p| !p.as_os_str().is_empty()) {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!(
                        "Failed to create parent directory '{}': {}",
                        parent.display(),
                        e
                    )),
                });
            }
        }

        match tokio::fs::write(&path_buf, content).await {
            Ok(()) => Ok(ToolResult {
                success: true,
                output: format!("Written {} bytes to {}", content.len(), path_buf.display()),
                error: None,
            }),
            Err(e) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!(
                    "Failed to write file '{}': {}",
                    path_buf.display(),
                    e
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
        std::env::temp_dir().join(format!("rikabot_test_filesystem_write_{name}_{nonce}"))
    }

    #[test]
    fn filesystem_write_name_and_schema() {
        let tool = FilesystemWriteTool::new();
        assert_eq!(tool.name(), "filesystem_write");
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["path"].is_object());
        assert!(schema["properties"]["content"].is_object());
    }

    #[tokio::test]
    async fn filesystem_write_creates_file() {
        let dir = make_temp_dir("create");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let file = dir.join("hello.txt");

        let tool = FilesystemWriteTool::new();
        let result = tool
            .execute(serde_json::json!({
                "path": file.to_string_lossy().to_string(),
                "content": "hello"
            }))
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.output.contains("Written 5 bytes"));
        let written = tokio::fs::read_to_string(&file).await.unwrap();
        assert_eq!(written, "hello");

        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn filesystem_write_creates_parent_dirs() {
        let dir = make_temp_dir("nested");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let file = dir.join("a/b/c/note.txt");

        let tool = FilesystemWriteTool::new();
        let result = tool
            .execute(serde_json::json!({
                "path": file.to_string_lossy().to_string(),
                "content": "deep"
            }))
            .await
            .unwrap();

        assert!(result.success);
        let written = tokio::fs::read_to_string(&file).await.unwrap();
        assert_eq!(written, "deep");

        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn filesystem_write_missing_path_param_is_error() {
        let tool = FilesystemWriteTool::new();
        let result = tool
            .execute(serde_json::json!({
                "content": "hello"
            }))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn filesystem_write_resolves_relative_path_from_workspace_dir() {
        let workspace = make_temp_dir("workspace_relative");
        tokio::fs::create_dir_all(&workspace).await.unwrap();

        let tool = FilesystemWriteTool::with_workspace_dir(workspace.clone());
        let result = tool
            .execute(serde_json::json!({
                "path": "notes/bootstrap.md",
                "content": "workspace-root write"
            }))
            .await
            .unwrap();

        assert!(result.success);
        let written = tokio::fs::read_to_string(workspace.join("notes/bootstrap.md"))
            .await
            .unwrap();
        assert_eq!(written, "workspace-root write");

        let _ = tokio::fs::remove_dir_all(workspace).await;
    }
}
