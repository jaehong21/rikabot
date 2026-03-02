use super::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

const DEFAULT_OUTPUT_LINES: usize = 120;
const MAX_OUTPUT_LINES: usize = 2_000;
const WAIT_POLL_INTERVAL_MS: u64 = 200;
const KILL_POLL_INTERVAL_MS: u64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessStatus {
    Running,
    Finished,
    Failed,
    Killed,
}

impl ProcessStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Finished => "finished",
            Self::Failed => "failed",
            Self::Killed => "killed",
        }
    }
}

#[derive(Debug, Clone)]
struct ProcessState {
    status: ProcessStatus,
    exit_code: Option<i32>,
    started_at_unix_secs: u64,
    finished_at_unix_secs: Option<u64>,
    kill_requested: bool,
}

impl ProcessState {
    fn running() -> Self {
        Self {
            status: ProcessStatus::Running,
            exit_code: None,
            started_at_unix_secs: unix_now_secs(),
            finished_at_unix_secs: None,
            kill_requested: false,
        }
    }
}

#[derive(Debug, Default, Clone)]
struct OutputBuffer {
    data: String,
    dropped_prefix_bytes: u64,
}

struct ProcessEntry {
    id: String,
    command: String,
    pid: u32,
    started_at: Instant,
    child: Mutex<Child>,
    stdout_buf: Arc<Mutex<OutputBuffer>>,
    stderr_buf: Arc<Mutex<OutputBuffer>>,
    state: Mutex<ProcessState>,
}

pub struct ProcessTool {
    config: crate::config::ProcessConfig,
    workspace_dir: Option<PathBuf>,
    processes: Arc<RwLock<HashMap<String, Arc<ProcessEntry>>>>,
    next_id: AtomicU64,
}

impl ProcessTool {
    pub fn with_workspace_dir(
        config: crate::config::ProcessConfig,
        workspace_dir: PathBuf,
    ) -> Self {
        Self {
            config,
            workspace_dir: Some(workspace_dir),
            processes: Arc::new(RwLock::new(HashMap::new())),
            next_id: AtomicU64::new(0),
        }
    }

    fn build_snapshot(&self, entry: &Arc<ProcessEntry>, state: &ProcessState) -> serde_json::Value {
        serde_json::json!({
            "id": entry.id,
            "command": entry.command,
            "pid": entry.pid,
            "status": state.status.as_str(),
            "exit_code": state.exit_code,
            "started_at_unix_secs": state.started_at_unix_secs,
            "finished_at_unix_secs": state.finished_at_unix_secs,
            "uptime_secs": entry.started_at.elapsed().as_secs(),
        })
    }

    fn parse_id(args: &serde_json::Value, action: &str) -> Result<String> {
        let id = args
            .get("id")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter for '{}' action", action))?;

        if id.is_empty() {
            anyhow::bail!("'id' parameter must not be empty");
        }

        Ok(id.to_string())
    }

    fn parse_optional_u64(args: &serde_json::Value, key: &str) -> Result<Option<u64>> {
        let Some(value) = args.get(key) else {
            return Ok(None);
        };
        let parsed = value
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("'{}' must be an integer", key))?;
        Ok(Some(parsed))
    }

    fn parse_optional_usize(args: &serde_json::Value, key: &str) -> Result<Option<usize>> {
        let Some(value) = args.get(key) else {
            return Ok(None);
        };
        let parsed = value
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("'{}' must be an integer", key))?;
        Ok(Some(parsed as usize))
    }

    fn next_id(&self) -> String {
        let seq = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        format!("p-{seq:08x}")
    }

    fn running_count(&self) -> usize {
        let processes = match self.processes.read() {
            Ok(guard) => guard,
            Err(_) => return 0,
        };
        processes
            .values()
            .filter(|entry| self.refresh_state(entry).status == ProcessStatus::Running)
            .count()
    }

    fn cleanup_expired(&self) {
        let retention_secs = self.config.cleanup_retention_secs;
        let now = unix_now_secs();
        let Ok(mut processes) = self.processes.write() else {
            return;
        };
        processes.retain(|_, entry| {
            let state = self.refresh_state(entry);
            if state.status == ProcessStatus::Running {
                return true;
            }
            let Some(finished_at) = state.finished_at_unix_secs else {
                return true;
            };
            now.saturating_sub(finished_at) < retention_secs
        });
    }

    fn mark_kill_requested(entry: &Arc<ProcessEntry>) {
        if let Ok(mut state) = entry.state.lock() {
            state.kill_requested = true;
        }
    }

    fn refresh_state(&self, entry: &Arc<ProcessEntry>) -> ProcessState {
        let child_status = {
            let Ok(mut child) = entry.child.lock() else {
                return ProcessState {
                    status: ProcessStatus::Failed,
                    exit_code: None,
                    started_at_unix_secs: unix_now_secs(),
                    finished_at_unix_secs: Some(unix_now_secs()),
                    kill_requested: false,
                };
            };
            child.try_wait().ok().flatten()
        };

        if let Some(status) = child_status {
            if let Ok(mut state) = entry.state.lock() {
                if state.status == ProcessStatus::Running {
                    state.exit_code = status.code();
                    state.finished_at_unix_secs = Some(unix_now_secs());
                    state.status = if state.kill_requested {
                        ProcessStatus::Killed
                    } else if status.success() {
                        ProcessStatus::Finished
                    } else {
                        ProcessStatus::Failed
                    };
                }
                return state.clone();
            }
        }

        entry
            .state
            .lock()
            .map(|state| state.clone())
            .unwrap_or_else(|_| ProcessState {
                status: ProcessStatus::Failed,
                exit_code: None,
                started_at_unix_secs: unix_now_secs(),
                finished_at_unix_secs: Some(unix_now_secs()),
                kill_requested: false,
            })
    }

    async fn handle_spawn(&self, args: &serde_json::Value) -> Result<ToolResult> {
        let command = args
            .get("command")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .ok_or_else(|| anyhow::anyhow!("Missing 'command' parameter for 'spawn' action"))?;
        if command.is_empty() {
            anyhow::bail!("'command' must not be empty");
        }

        let running = self.running_count();
        let max_concurrent = self.config.resolved_max_concurrent();
        if running >= max_concurrent {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!(
                    "Maximum concurrent processes ({max_concurrent}) reached"
                )),
            });
        }

        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command);
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        if let Some(workspace_dir) = &self.workspace_dir {
            cmd.current_dir(workspace_dir);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to spawn process: {}", e))?;
        let pid = child.id().unwrap_or(0);

        let stdout_buf = Arc::new(Mutex::new(OutputBuffer::default()));
        let stderr_buf = Arc::new(Mutex::new(OutputBuffer::default()));
        let max_output_bytes = self.config.max_output_bytes;

        if let Some(stdout) = child.stdout.take() {
            spawn_reader_task(stdout, stdout_buf.clone(), max_output_bytes);
        }
        if let Some(stderr) = child.stderr.take() {
            spawn_reader_task(stderr, stderr_buf.clone(), max_output_bytes);
        }

        let id = self.next_id();
        let entry = Arc::new(ProcessEntry {
            id: id.clone(),
            command: command.to_string(),
            pid,
            started_at: Instant::now(),
            child: Mutex::new(child),
            stdout_buf,
            stderr_buf,
            state: Mutex::new(ProcessState::running()),
        });

        self.processes
            .write()
            .map_err(|_| anyhow::anyhow!("process registry write lock poisoned"))?
            .insert(id.clone(), entry.clone());

        let state = self.refresh_state(&entry);
        let output = self.build_snapshot(&entry, &state).to_string();
        Ok(ToolResult {
            success: true,
            output,
            error: None,
        })
    }

    async fn handle_list(&self) -> Result<ToolResult> {
        let processes: Vec<Arc<ProcessEntry>> = self
            .processes
            .read()
            .map_err(|_| anyhow::anyhow!("process registry read lock poisoned"))?
            .values()
            .cloned()
            .collect();

        let mut snapshots = Vec::with_capacity(processes.len());
        for entry in processes {
            let state = self.refresh_state(&entry);
            snapshots.push(self.build_snapshot(&entry, &state));
        }
        snapshots.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));

        Ok(ToolResult {
            success: true,
            output: serde_json::to_string(&snapshots)?,
            error: None,
        })
    }

    fn find_entry(&self, id: &str) -> Option<Arc<ProcessEntry>> {
        self.processes.read().ok()?.get(id).cloned()
    }

    async fn handle_status(&self, args: &serde_json::Value) -> Result<ToolResult> {
        let id = Self::parse_id(args, "status")?;
        let Some(entry) = self.find_entry(&id) else {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("No process found for id '{}'", id)),
            });
        };
        let state = self.refresh_state(&entry);
        Ok(ToolResult {
            success: true,
            output: self.build_snapshot(&entry, &state).to_string(),
            error: None,
        })
    }

    async fn handle_output(&self, args: &serde_json::Value) -> Result<ToolResult> {
        let id = Self::parse_id(args, "output")?;
        let lines = Self::parse_optional_usize(args, "lines")?.unwrap_or(DEFAULT_OUTPUT_LINES);
        if lines == 0 {
            anyhow::bail!("'lines' must be greater than 0");
        }
        let lines = lines.min(MAX_OUTPUT_LINES);

        let Some(entry) = self.find_entry(&id) else {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("No process found for id '{}'", id)),
            });
        };
        let state = self.refresh_state(&entry);

        let stdout_snapshot = snapshot_buffer(&entry.stdout_buf);
        let stderr_snapshot = snapshot_buffer(&entry.stderr_buf);

        let stdout = tail_lines(&stdout_snapshot.data, lines);
        let stderr = tail_lines(&stderr_snapshot.data, lines);

        let output = serde_json::json!({
            "id": id,
            "status": state.status.as_str(),
            "stdout": stdout,
            "stderr": stderr,
            "stdout_truncated": stdout_snapshot.dropped_prefix_bytes > 0,
            "stderr_truncated": stderr_snapshot.dropped_prefix_bytes > 0,
        });

        Ok(ToolResult {
            success: true,
            output: output.to_string(),
            error: None,
        })
    }

    async fn handle_wait(&self, args: &serde_json::Value) -> Result<ToolResult> {
        let id = Self::parse_id(args, "wait")?;
        let requested_wait = Self::parse_optional_u64(args, "max_wait_secs")?
            .unwrap_or(self.config.wait_default_secs);
        if requested_wait == 0 {
            anyhow::bail!("'max_wait_secs' must be greater than 0");
        }
        let max_wait_secs = requested_wait.min(self.config.wait_max_secs);
        let deadline = Instant::now() + Duration::from_secs(max_wait_secs);

        loop {
            let Some(entry) = self.find_entry(&id) else {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("No process found for id '{}'", id)),
                });
            };

            let state = self.refresh_state(&entry);
            if state.status != ProcessStatus::Running {
                let output = serde_json::json!({
                    "wait_timed_out": false,
                    "waited_secs": max_wait_secs,
                    "process": self.build_snapshot(&entry, &state),
                });
                return Ok(ToolResult {
                    success: true,
                    output: output.to_string(),
                    error: None,
                });
            }

            if Instant::now() >= deadline {
                let output = serde_json::json!({
                    "wait_timed_out": true,
                    "waited_secs": max_wait_secs,
                    "still_running": true,
                    "process": self.build_snapshot(&entry, &state),
                });
                return Ok(ToolResult {
                    success: true,
                    output: output.to_string(),
                    error: None,
                });
            }

            tokio::time::sleep(Duration::from_millis(WAIT_POLL_INTERVAL_MS)).await;
        }
    }

    async fn handle_kill(&self, args: &serde_json::Value) -> Result<ToolResult> {
        let id = Self::parse_id(args, "kill")?;
        let Some(entry) = self.find_entry(&id) else {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("No process found for id '{}'", id)),
            });
        };

        let initial_state = self.refresh_state(&entry);
        if initial_state.status != ProcessStatus::Running {
            let output = serde_json::json!({
                "already_stopped": true,
                "process": self.build_snapshot(&entry, &initial_state),
            });
            return Ok(ToolResult {
                success: true,
                output: output.to_string(),
                error: None,
            });
        }

        Self::mark_kill_requested(&entry);

        #[cfg(unix)]
        {
            let _ = std::process::Command::new("kill")
                .arg("-TERM")
                .arg(entry.pid.to_string())
                .status();
        }
        #[cfg(not(unix))]
        {
            let _ = entry.child.lock().map(|mut child| child.start_kill());
        }

        let graceful_deadline = Instant::now() + Duration::from_secs(self.config.kill_grace_secs);
        loop {
            let state = self.refresh_state(&entry);
            if state.status != ProcessStatus::Running {
                let output = serde_json::json!({
                    "forced": false,
                    "process": self.build_snapshot(&entry, &state),
                });
                return Ok(ToolResult {
                    success: true,
                    output: output.to_string(),
                    error: None,
                });
            }
            if Instant::now() >= graceful_deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(KILL_POLL_INTERVAL_MS)).await;
        }

        let _ = entry.child.lock().map(|mut child| child.start_kill());

        let forced_deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let state = self.refresh_state(&entry);
            if state.status != ProcessStatus::Running || Instant::now() >= forced_deadline {
                let output = serde_json::json!({
                    "forced": true,
                    "process": self.build_snapshot(&entry, &state),
                });
                return Ok(ToolResult {
                    success: true,
                    output: output.to_string(),
                    error: None,
                });
            }
            tokio::time::sleep(Duration::from_millis(KILL_POLL_INTERVAL_MS)).await;
        }
    }
}

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn append_bounded(buf: &Mutex<OutputBuffer>, incoming: &str, max_bytes: usize) {
    let Ok(mut guard) = buf.lock() else {
        return;
    };
    guard.data.push_str(incoming);
    if guard.data.len() <= max_bytes {
        return;
    }

    let excess = guard.data.len() - max_bytes;
    let mut drain_to = excess;
    while drain_to < guard.data.len() && !guard.data.is_char_boundary(drain_to) {
        drain_to += 1;
    }
    guard.data.drain(..drain_to);
    guard.dropped_prefix_bytes = guard
        .dropped_prefix_bytes
        .saturating_add(u64::try_from(drain_to).unwrap_or(u64::MAX));
}

fn spawn_reader_task<R: tokio::io::AsyncRead + Unpin + Send + 'static>(
    mut reader: R,
    buffer: Arc<Mutex<OutputBuffer>>,
    max_bytes: usize,
) {
    tokio::spawn(async move {
        let mut chunk = vec![0u8; 8192];
        loop {
            match reader.read(&mut chunk).await {
                Ok(0) => break,
                Ok(n) => {
                    let text = String::from_utf8_lossy(&chunk[..n]);
                    append_bounded(&buffer, &text, max_bytes);
                }
                Err(_) => break,
            }
        }
    });
}

fn snapshot_buffer(buf: &Mutex<OutputBuffer>) -> OutputBuffer {
    buf.lock().map(|v| v.clone()).unwrap_or_default()
}

fn tail_lines(text: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= max_lines {
        return text.to_string();
    }
    lines[lines.len() - max_lines..].join("\n")
}

#[async_trait]
impl Tool for ProcessTool {
    fn name(&self) -> &str {
        "process"
    }

    fn description(&self) -> &str {
        "Manage long-running background processes with lifecycle actions: spawn, list, status, output, wait, and kill."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["spawn", "list", "status", "output", "wait", "kill"],
                    "description": "Action to perform for background process management"
                },
                "command": {
                    "type": "string",
                    "description": "Shell command to run in background (required for 'spawn')"
                },
                "id": {
                    "type": "string",
                    "description": "Process id (required for status/output/wait/kill)"
                },
                "max_wait_secs": {
                    "type": "integer",
                    "description": "Maximum seconds to wait in 'wait' action"
                },
                "lines": {
                    "type": "integer",
                    "description": "Number of output lines to return for 'output' action"
                },
                "approved": {
                    "type": "boolean",
                    "description": "Reserved for parity with shell approvals",
                    "default": false
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        self.cleanup_expired();

        let action = args
            .get("action")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

        if action.is_empty() {
            anyhow::bail!("'action' parameter must not be empty");
        }

        match action {
            "spawn" => self.handle_spawn(&args).await,
            "list" => self.handle_list().await,
            "status" => self.handle_status(&args).await,
            "output" => self.handle_output(&args).await,
            "wait" => self.handle_wait(&args).await,
            "kill" => self.handle_kill(&args).await,
            other => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!(
                    "Unknown action '{}'. Expected: spawn, list, status, output, wait, kill",
                    other
                )),
            }),
        }
    }
}

impl Drop for ProcessTool {
    fn drop(&mut self) {
        if let Ok(processes) = self.processes.read() {
            for entry in processes.values() {
                if let Ok(mut child) = entry.child.lock() {
                    let _ = child.start_kill();
                }
            }
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
        std::env::temp_dir().join(format!("rikabot_test_process_{name}_{nonce}"))
    }

    fn test_config() -> crate::config::ProcessConfig {
        crate::config::ProcessConfig {
            enabled: true,
            max_concurrent: 8,
            max_output_bytes: 524_288,
            cleanup_retention_secs: 600,
            kill_grace_secs: 1,
            wait_default_secs: 1,
            wait_max_secs: 5,
        }
    }

    fn parse_json(result: &ToolResult) -> serde_json::Value {
        serde_json::from_str(&result.output).expect("tool output should be valid json")
    }

    #[tokio::test]
    async fn process_spawn_wait_and_output_flow() {
        let workspace = make_temp_dir("spawn_wait_output");
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        let tool = ProcessTool::with_workspace_dir(test_config(), workspace.clone());

        let spawned = tool
            .execute(serde_json::json!({
                "action": "spawn",
                "command": "echo begin && sleep 1 && echo done"
            }))
            .await
            .unwrap();
        assert!(spawned.success, "{:?}", spawned.error);
        let spawn_json = parse_json(&spawned);
        let id = spawn_json["id"].as_str().unwrap().to_string();
        assert_eq!(spawn_json["status"], "running");

        let waited = tool
            .execute(serde_json::json!({
                "action": "wait",
                "id": id,
                "max_wait_secs": 3
            }))
            .await
            .unwrap();
        assert!(waited.success, "{:?}", waited.error);
        let waited_json = parse_json(&waited);
        assert_eq!(waited_json["wait_timed_out"], false);
        assert_ne!(waited_json["process"]["status"], "running");

        let output = tool
            .execute(serde_json::json!({
                "action": "output",
                "id": spawn_json["id"],
                "lines": 20
            }))
            .await
            .unwrap();
        assert!(output.success);
        let output_json = parse_json(&output);
        let stdout = output_json["stdout"].as_str().unwrap_or("");
        assert!(stdout.contains("begin"));
        assert!(stdout.contains("done"));

        let _ = tokio::fs::remove_dir_all(workspace).await;
    }

    #[tokio::test]
    async fn process_wait_can_timeout_without_failure() {
        let workspace = make_temp_dir("wait_timeout");
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        let tool = ProcessTool::with_workspace_dir(test_config(), workspace.clone());

        let spawned = tool
            .execute(serde_json::json!({
                "action": "spawn",
                "command": "sleep 2"
            }))
            .await
            .unwrap();
        let spawn_json = parse_json(&spawned);
        let id = spawn_json["id"].as_str().unwrap();

        let waited = tool
            .execute(serde_json::json!({
                "action": "wait",
                "id": id,
                "max_wait_secs": 1
            }))
            .await
            .unwrap();
        assert!(waited.success);
        let waited_json = parse_json(&waited);
        assert_eq!(waited_json["wait_timed_out"], true);
        assert_eq!(waited_json["still_running"], true);

        let _ = tool
            .execute(serde_json::json!({"action":"kill", "id": id}))
            .await;

        let _ = tokio::fs::remove_dir_all(workspace).await;
    }

    #[tokio::test]
    async fn process_kill_terminates_running_process() {
        let workspace = make_temp_dir("kill");
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        let tool = ProcessTool::with_workspace_dir(test_config(), workspace.clone());

        let spawned = tool
            .execute(serde_json::json!({
                "action": "spawn",
                "command": "sleep 20"
            }))
            .await
            .unwrap();
        let spawn_json = parse_json(&spawned);
        let id = spawn_json["id"].as_str().unwrap();

        let killed = tool
            .execute(serde_json::json!({"action":"kill", "id": id}))
            .await
            .unwrap();
        assert!(killed.success, "{:?}", killed.error);
        let killed_json = parse_json(&killed);
        assert_eq!(killed_json["process"]["status"], "killed");

        let status = tool
            .execute(serde_json::json!({"action":"status", "id": id}))
            .await
            .unwrap();
        let status_json = parse_json(&status);
        assert_ne!(status_json["status"], "running");

        let _ = tokio::fs::remove_dir_all(workspace).await;
    }

    #[tokio::test]
    async fn process_enforces_max_concurrent_limit() {
        let workspace = make_temp_dir("max_concurrent");
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        let mut cfg = test_config();
        cfg.max_concurrent = 1;
        let tool = ProcessTool::with_workspace_dir(cfg, workspace.clone());

        let first = tool
            .execute(serde_json::json!({
                "action": "spawn",
                "command": "sleep 5"
            }))
            .await
            .unwrap();
        assert!(first.success);
        let first_id = parse_json(&first)["id"].as_str().unwrap().to_string();

        let second = tool
            .execute(serde_json::json!({
                "action": "spawn",
                "command": "sleep 5"
            }))
            .await
            .unwrap();
        assert!(!second.success);
        assert!(second
            .error
            .unwrap_or_default()
            .contains("Maximum concurrent processes"));

        let _ = tool
            .execute(serde_json::json!({"action":"kill", "id": first_id}))
            .await;

        let _ = tokio::fs::remove_dir_all(workspace).await;
    }

    #[tokio::test]
    async fn process_cleanup_removes_expired_completed_entries() {
        let workspace = make_temp_dir("cleanup");
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        let mut cfg = test_config();
        cfg.cleanup_retention_secs = 1;
        let tool = ProcessTool::with_workspace_dir(cfg, workspace.clone());

        let spawned = tool
            .execute(serde_json::json!({
                "action": "spawn",
                "command": "echo done"
            }))
            .await
            .unwrap();
        assert!(spawned.success);
        let id = parse_json(&spawned)["id"].as_str().unwrap().to_string();

        let _ = tool
            .execute(serde_json::json!({"action":"status", "id": id}))
            .await
            .unwrap();

        let _ = tool
            .execute(serde_json::json!({
                "action":"wait",
                "id": id,
                "max_wait_secs": 2
            }))
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_secs(2)).await;

        let listed = tool
            .execute(serde_json::json!({"action":"list"}))
            .await
            .unwrap();
        assert!(listed.success);
        let listed_json = parse_json(&listed);
        let ids: Vec<String> = listed_json
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|entry| entry["id"].as_str().map(ToString::to_string))
            .collect();
        assert!(!ids.contains(&id));

        let _ = tokio::fs::remove_dir_all(workspace).await;
    }
}
