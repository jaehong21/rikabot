use super::*;

const MAX_RESULTS: usize = 1000;

/// Filesystem glob tool.
///
/// Finds files matching a glob pattern.
pub struct FilesystemGlobTool;

impl FilesystemGlobTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FilesystemGlobTool {
    fn name(&self) -> &str {
        "filesystem_glob"
    }

    fn description(&self) -> &str {
        "Find files matching a glob pattern (for example: '**/*.rs')."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern to match files."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of files to return (default: 1000)."
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'pattern' argument"))?;

        if pattern.trim().is_empty() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Pattern cannot be empty".to_string()),
            });
        }

        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|v| usize::try_from(v).unwrap_or(MAX_RESULTS))
            .unwrap_or(MAX_RESULTS)
            .min(MAX_RESULTS);

        let entries = match glob::glob(pattern) {
            Ok(entries) => entries,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Invalid glob pattern: {}", e)),
                });
            }
        };

        let mut files = Vec::new();
        let mut truncated = false;

        for entry in entries {
            let path = match entry {
                Ok(p) => p,
                Err(_) => continue,
            };

            if !path.is_file() {
                continue;
            }

            files.push(path.to_string_lossy().to_string());
            if files.len() >= max_results {
                truncated = true;
                break;
            }
        }

        files.sort();

        let output = if files.is_empty() {
            format!("No files matched pattern '{}'.", pattern)
        } else {
            let mut out = files.join("\n");
            if truncated {
                out.push_str(&format!(
                    "\n\n[Results truncated: showing first {} matches]",
                    max_results
                ));
            }
            out.push_str(&format!("\n\nTotal: {} file(s)", files.len()));
            out
        };

        Ok(ToolResult {
            success: true,
            output,
            error: None,
        })
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
        std::env::temp_dir().join(format!("rikabot_test_filesystem_glob_{name}_{nonce}"))
    }

    #[test]
    fn filesystem_glob_name_and_schema() {
        let tool = FilesystemGlobTool::new();
        assert_eq!(tool.name(), "filesystem_glob");
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["pattern"].is_object());
    }

    #[tokio::test]
    async fn filesystem_glob_finds_files() {
        let dir = make_temp_dir("find_files");
        tokio::fs::create_dir_all(dir.join("src")).await.unwrap();
        tokio::fs::write(dir.join("src/main.rs"), "fn main() {}")
            .await
            .unwrap();
        tokio::fs::write(dir.join("src/lib.rs"), "").await.unwrap();
        tokio::fs::write(dir.join("README.md"), "").await.unwrap();

        let pattern = format!("{}/**/*.rs", dir.to_string_lossy());
        let tool = FilesystemGlobTool::new();
        let result = tool
            .execute(serde_json::json!({ "pattern": pattern }))
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.output.contains("main.rs"));
        assert!(result.output.contains("lib.rs"));
        assert!(!result.output.contains("README.md"));

        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn filesystem_glob_invalid_pattern_returns_error() {
        let tool = FilesystemGlobTool::new();
        let result = tool
            .execute(serde_json::json!({ "pattern": "[" }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result
            .error
            .as_deref()
            .unwrap_or("")
            .contains("Invalid glob"));
    }
}
