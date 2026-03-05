export type ConnectionState = "connecting" | "connected" | "disconnected";

export type ThreadRecord = {
  id: string;
  display_name: string;
  created_at: string;
  updated_at: string;
  message_count: number;
};

export type HistoryMessage = {
  role: string;
  content: string;
};

export type QueuedInput = {
  id: string;
  content: string;
};

export type TokenUsage = {
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
};

export type ResponseStats = {
  elapsedMs: number;
  toolCalls: number;
  toolSuccess: number;
  toolFailed: number;
  toolDenied: number;
  usage?: TokenUsage;
};

export type MessageEntry = {
  id: string;
  kind: "message";
  role: "user" | "assistant";
  text: string;
  error?: boolean;
  stats?: ResponseStats;
};

export type ToolStatus = "running" | "success" | "failed" | "denied";

export type ToolApprovalDecision = "allow_persist" | "allow_once" | "deny";

export type ToolApprovalState = {
  requestId: string;
  suggestedAllowRule: string;
  allowRuleInput: string;
  submitting: boolean;
};

export type ToolEntry = {
  id: string;
  kind: "tool";
  callId: string;
  name: string;
  args: string;
  argsPreview: string;
  output: string;
  status: ToolStatus;
  awaitingApproval: boolean;
  approval?: ToolApprovalState;
  resolved: boolean;
  open: boolean;
};

export type Entry = MessageEntry | ToolEntry;

export type PermissionsState = {
  enabled?: boolean;
  tools?: {
    allow?: string[];
    deny?: string[];
  };
};

export type McpServerState =
  | "pending"
  | "connecting"
  | "ready"
  | "failed"
  | "disabled";

export type McpToolStatus = {
  name: string;
  description?: string | null;
};

export type McpServerStatus = {
  name: string;
  state: McpServerState;
  tool_count?: number;
  tools?: McpToolStatus[];
  error?: string | null;
};

export type McpStatusSnapshot = {
  enabled?: boolean;
  servers?: McpServerStatus[];
};

export type SkillStatus = {
  name: string;
  description: string;
  always: boolean;
  available: boolean;
  path: string;
  missing?: string[];
};

export type SkillsStatusSnapshot = {
  enabled?: boolean;
  skills?: SkillStatus[];
};

export type DoneEvent = {
  type: "done";
  session_id?: string;
  run_id?: number;
  full_response?: string;
  elapsed_ms?: number;
  tool_call_count?: number;
  tool_call_success?: number;
  tool_call_failed?: number;
  tool_call_denied?: number;
  usage?: Partial<TokenUsage>;
};

export type StoppedEvent = {
  type: "stopped";
  reason?: string;
  session_id?: string;
  run_id?: number;
};

export type ToolCallStartEvent = {
  type: "tool_call_start";
  session_id?: string;
  run_id?: number;
  call_id?: string;
  name?: string;
  args?: unknown;
};

export type ToolCallResultEvent = {
  type: "tool_call_result";
  session_id?: string;
  run_id?: number;
  call_id?: string;
  name?: string;
  output?: string;
  success?: boolean;
  status?: ToolStatus | "failure";
  approval_request_id?: string;
  awaiting_approval?: boolean;
};

export type ToolApprovalRequiredEvent = {
  type: "tool_approval_required";
  session_id?: string;
  run_id?: number;
  request_id?: string;
  call_id?: string;
  name?: string;
  args?: unknown;
  deny_reason?: string;
  suggested_allow_rule?: string;
};

export type ServerEvent =
  | {
      type: "user_message";
      session_id?: string;
      run_id?: number;
      content?: string;
      started_at_unix_ms?: number;
    }
  | { type: "chunk"; session_id?: string; run_id?: number; content?: string }
  | ToolCallStartEvent
  | ToolCallResultEvent
  | ToolApprovalRequiredEvent
  | DoneEvent
  | StoppedEvent
  | { type: "error"; session_id?: string; run_id?: number; message?: string }
  | {
      type: "queue_updated";
      session_id?: string;
      items?: QueuedInput[];
    }
  | {
      type: "thread_list";
      sessions?: ThreadRecord[];
    }
  | {
      type: "permissions_updated";
      permissions?: PermissionsState;
    }
  | {
      type: "mcp_status";
      mcp?: McpStatusSnapshot;
    };

export type ThreadsResponse = {
  sessions: ThreadRecord[];
};

export type CreateThreadResponse = {
  session: ThreadRecord;
  sessions: ThreadRecord[];
};

export type RenameThreadResponse = {
  session: ThreadRecord;
  sessions: ThreadRecord[];
};

export type ThreadMessagesResponse = {
  session_id: string;
  history: HistoryMessage[];
};

export type DeleteThreadResponse = {
  deleted_session_id: string;
  fallback_session_id: string;
  sessions: ThreadRecord[];
};

export type PermissionsResponse = {
  permissions: PermissionsState;
};

export type SkillsResponse = {
  skills: SkillsStatusSnapshot;
};

export type SkillContentResponse = {
  path: string;
  content: string;
};

export type UpdateSkillContentResponse = {
  skills: SkillsStatusSnapshot;
  path: string;
  content: string;
};
