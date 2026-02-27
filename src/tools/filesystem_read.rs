use super::*;
use std::io::ErrorKind;
use std::path::PathBuf;

/// Filesystem file-read tool.
///
/// Reads file contents from an absolute or relative path and returns numbered
/// lines. Supports optional 1-based line offset and line limit.
pub struct FilesystemReadTool;

impl FilesystemReadTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FilesystemReadTool {
    fn name(&self) -> &str {
        "filesystem_read"
    }

    fn description(&self) -> &str {
        "Read file contents with line numbers. Supports optional line slicing via offset and limit."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file. Can be absolute or relative."
                },
                "offset": {
                    "type": "integer",
                    "description": "Starting line number (1-based, default: 1)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to return (default: all)"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'path' argument"))?;

        let offset = args
            .get("offset")
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
            .max(1);

        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| usize::try_from(v).unwrap_or(usize::MAX));

        let path_buf = PathBuf::from(path);
        let read_path = match tokio::fs::canonicalize(&path_buf).await {
            Ok(p) => p,
            Err(_) => path_buf,
        };

        let metadata = match tokio::fs::metadata(&read_path).await {
            Ok(meta) => meta,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!(
                        "Failed to read file metadata for '{}': {}",
                        path, e
                    )),
                });
            }
        };

        if metadata.is_dir() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Path '{}' points to a directory, not a file", path)),
            });
        }

        let contents = match tokio::fs::read_to_string(&read_path).await {
            Ok(text) => text,
            Err(e) if e.kind() == ErrorKind::InvalidData => {
                let bytes = match tokio::fs::read(&read_path).await {
                    Ok(bytes) => bytes,
                    Err(read_err) => {
                        return Ok(ToolResult {
                            success: false,
                            output: String::new(),
                            error: Some(format!("Failed to read file '{}': {}", path, read_err)),
                        });
                    }
                };
                String::from_utf8_lossy(&bytes).into_owned()
            }
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Failed to read file '{}': {}", path, e)),
                });
            }
        };

        Ok(ToolResult {
            success: true,
            output: format_lines(&contents, offset, limit),
            error: None,
        })
    }
}

fn format_lines(contents: &str, offset: u64, limit: Option<usize>) -> String {
    let lines: Vec<&str> = contents.lines().collect();
    let total = lines.len();

    if total == 0 {
        return String::new();
    }

    let start = usize::try_from(offset.saturating_sub(1))
        .unwrap_or(usize::MAX)
        .min(total);

    let end = match limit {
        Some(line_limit) => start.saturating_add(line_limit).min(total),
        None => total,
    };

    if start >= end {
        return format!("[No lines in range, file has {total} lines]");
    }

    let numbered = lines[start..end]
        .iter()
        .enumerate()
        .map(|(i, line)| format!("{}: {}", start + i + 1, line))
        .collect::<Vec<_>>()
        .join("\n");

    let summary = if start > 0 || end < total {
        format!("\n[Lines {}-{} of {total}]", start + 1, end)
    } else {
        format!("\n[{total} lines total]")
    };

    format!("{numbered}{summary}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        std::env::temp_dir().join(format!("rikabot_test_filesystem_read_{name}_{nonce}"))
    }

    #[test]
    fn filesystem_read_name_and_schema() {
        let tool = FilesystemReadTool::new();
        assert_eq!(tool.name(), "filesystem_read");
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["path"].is_object());
        assert!(schema["required"]
            .as_array()
            .expect("required must be array")
            .contains(&serde_json::json!("path")));
    }

    #[tokio::test]
    async fn filesystem_read_full_file() {
        let dir = make_temp_dir("full");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let file = dir.join("note.txt");
        tokio::fs::write(&file, "one\ntwo\n").await.unwrap();

        let tool = FilesystemReadTool::new();
        let result = tool
            .execute(serde_json::json!({ "path": file.to_string_lossy().to_string() }))
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.output.contains("1: one"));
        assert!(result.output.contains("2: two"));
        assert!(result.output.contains("[2 lines total]"));

        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn filesystem_read_offset_and_limit() {
        let dir = make_temp_dir("offset_limit");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let file = dir.join("note.txt");
        tokio::fs::write(&file, "one\ntwo\nthree\nfour\n")
            .await
            .unwrap();

        let tool = FilesystemReadTool::new();
        let result = tool
            .execute(serde_json::json!({
                "path": file.to_string_lossy().to_string(),
                "offset": 2,
                "limit": 2
            }))
            .await
            .unwrap();

        assert!(result.success);
        assert!(!result.output.contains("1: one"));
        assert!(result.output.contains("2: two"));
        assert!(result.output.contains("3: three"));
        assert!(result.output.contains("[Lines 2-3 of 4]"));

        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn filesystem_read_missing_path_param_is_error() {
        let tool = FilesystemReadTool::new();
        let result = tool.execute(serde_json::json!({})).await;
        assert!(result.is_err());
        assert!(result
            .expect_err("missing path should error")
            .to_string()
            .contains("path"));
    }

    #[tokio::test]
    async fn filesystem_read_rejects_directory_path() {
        let dir = make_temp_dir("directory");
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let tool = FilesystemReadTool::new();
        let result = tool
            .execute(serde_json::json!({ "path": dir.to_string_lossy().to_string() }))
            .await
            .unwrap();

        assert!(!result.success);
        assert!(result.error.as_deref().unwrap_or("").contains("directory"));

        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn filesystem_read_non_utf8_falls_back_to_lossy() {
        let dir = make_temp_dir("non_utf8");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let file = dir.join("bytes.bin");
        tokio::fs::write(&file, vec![0x66, 0x6f, 0x80, 0x0a, 0x62])
            .await
            .unwrap();

        let tool = FilesystemReadTool::new();
        let result = tool
            .execute(serde_json::json!({ "path": file.to_string_lossy().to_string() }))
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.output.contains("1: fo"));
        assert!(result.output.contains("2: b"));

        let _ = tokio::fs::remove_dir_all(dir).await;
    }
}
