use super::*;
use regex::RegexBuilder;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

const MAX_RESULTS: usize = 1000;

/// Filesystem text search tool.
///
/// Searches files under a path for lines matching a regex pattern.
pub struct FilesystemSearchTool;

impl FilesystemSearchTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FilesystemSearchTool {
    fn name(&self) -> &str {
        "filesystem_search"
    }

    fn description(&self) -> &str {
        "Search file contents by regex pattern under a path. Returns matching lines with file path and line numbers."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regex pattern to search for."
                },
                "path": {
                    "type": "string",
                    "description": "File or directory path to search in. Defaults to current directory.",
                    "default": "."
                },
                "include": {
                    "type": "string",
                    "description": "Optional glob pattern filter relative to search path (for example: '**/*.rs')."
                },
                "case_sensitive": {
                    "type": "boolean",
                    "description": "Case-sensitive search. Defaults to true.",
                    "default": true
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of matching lines to return (default: 1000)."
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

        let search_path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let include = args.get("include").and_then(|v| v.as_str());
        let case_sensitive = args
            .get("case_sensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|v| usize::try_from(v).unwrap_or(MAX_RESULTS))
            .unwrap_or(MAX_RESULTS)
            .min(MAX_RESULTS);

        let regex = match RegexBuilder::new(pattern)
            .case_insensitive(!case_sensitive)
            .build()
        {
            Ok(regex) => regex,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Invalid regex pattern: {}", e)),
                });
            }
        };

        let include_pattern = match include {
            Some(glob_pattern) => match glob::Pattern::new(glob_pattern) {
                Ok(p) => Some(p),
                Err(e) => {
                    return Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some(format!("Invalid include pattern: {}", e)),
                    });
                }
            },
            None => None,
        };

        let root = PathBuf::from(search_path);
        if !root.exists() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Search path does not exist: {}", search_path)),
            });
        }

        let mut files = match collect_files(&root) {
            Ok(files) => files,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!(
                        "Failed to scan search path '{}': {}",
                        search_path, e
                    )),
                });
            }
        };
        files.sort();

        let mut results = Vec::new();
        let mut truncated = false;

        'file_loop: for file in files {
            if let Some(ref include_pattern) = include_pattern {
                let rel = file.strip_prefix(&root).unwrap_or(&file);
                if !include_pattern.matches_path(rel) && !include_pattern.matches_path(&file) {
                    continue;
                }
            }

            let content = match read_text_lossy(&file) {
                Ok(c) => c,
                Err(_) => continue,
            };

            for (line_idx, line) in content.lines().enumerate() {
                if regex.is_match(line) {
                    results.push(format!("{}:{}:{}", file.display(), line_idx + 1, line));
                    if results.len() >= max_results {
                        truncated = true;
                        break 'file_loop;
                    }
                }
            }
        }

        let output = if results.is_empty() {
            format!("No matches found for pattern '{}'.", pattern)
        } else {
            let mut out = results.join("\n");
            if truncated {
                out.push_str(&format!(
                    "\n\n[Results truncated: showing first {} matches]",
                    max_results
                ));
            }
            out.push_str(&format!("\n\nTotal: {} match(es)", results.len()));
            out
        };

        Ok(ToolResult {
            success: true,
            output,
            error: None,
        })
    }
}

fn collect_files(path: &Path) -> std::io::Result<Vec<PathBuf>> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }

    let mut files = Vec::new();
    let mut stack = vec![path.to_path_buf()];

    while let Some(current) = stack.pop() {
        for entry in std::fs::read_dir(&current)? {
            let entry = entry?;
            let entry_path = entry.path();
            let file_type = entry.file_type()?;

            if file_type.is_dir() {
                stack.push(entry_path);
            } else if file_type.is_file() {
                files.push(entry_path);
            }
        }
    }

    Ok(files)
}

fn read_text_lossy(path: &Path) -> std::io::Result<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == ErrorKind::InvalidData => {
            let bytes = std::fs::read(path)?;
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        }
        Err(e) => Err(e),
    }
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
        std::env::temp_dir().join(format!("rikabot_test_filesystem_search_{name}_{nonce}"))
    }

    #[test]
    fn filesystem_search_name_and_schema() {
        let tool = FilesystemSearchTool::new();
        assert_eq!(tool.name(), "filesystem_search");
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["pattern"].is_object());
    }

    #[tokio::test]
    async fn filesystem_search_finds_matches() {
        let dir = make_temp_dir("find");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("a.txt"), "alpha\nbeta\n")
            .await
            .unwrap();
        tokio::fs::write(dir.join("b.txt"), "gamma\nalpha\n")
            .await
            .unwrap();

        let tool = FilesystemSearchTool::new();
        let result = tool
            .execute(serde_json::json!({
                "pattern": "alpha",
                "path": dir.to_string_lossy().to_string()
            }))
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.output.contains("a.txt:1:alpha"));
        assert!(result.output.contains("b.txt:2:alpha"));

        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn filesystem_search_respects_include_filter() {
        let dir = make_temp_dir("include");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("a.txt"), "needle\n")
            .await
            .unwrap();
        tokio::fs::write(dir.join("b.md"), "needle\n")
            .await
            .unwrap();

        let tool = FilesystemSearchTool::new();
        let result = tool
            .execute(serde_json::json!({
                "pattern": "needle",
                "path": dir.to_string_lossy().to_string(),
                "include": "*.md"
            }))
            .await
            .unwrap();

        assert!(result.success);
        assert!(!result.output.contains("a.txt"));
        assert!(result.output.contains("b.md"));

        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn filesystem_search_missing_pattern_param_is_error() {
        let tool = FilesystemSearchTool::new();
        let result = tool
            .execute(serde_json::json!({
                "path": "."
            }))
            .await;
        assert!(result.is_err());
    }
}
