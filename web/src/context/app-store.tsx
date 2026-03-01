import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";

import type {
  DoneEvent,
  Entry,
  HistoryMessage,
  McpServerStatus,
  McpStatusSnapshot,
  MessageEntry,
  PermissionsState,
  ResponseStats,
  ServerEvent,
  StoppedEvent,
  ThreadRecord,
  TokenUsage,
  ToolApprovalDecision,
  ToolApprovalRequiredEvent,
  ToolCallResultEvent,
  ToolEntry,
  ToolStatus,
} from "@/types/app";

const TOOL_PREFS_STORAGE_KEY = "rika.toolOutputsExpanded";
const TOOL_VISIBILITY_STORAGE_KEY = "rika.showToolCalls";

type AppState = {
  entries: Entry[];
  threads: ThreadRecord[];
  currentSessionId: string | null;
  connectionState: "connecting" | "connected" | "disconnected";
  isWaiting: boolean;
  killRequested: boolean;
  showReconnectOverlay: boolean;
  toolOutputsExpanded: boolean;
  showToolCalls: boolean;
  permissionsLoaded: boolean;
  permissionsSaving: boolean;
  permissionsEnabled: boolean;
  permissionsAllowText: string;
  permissionsDenyText: string;
  permissionsErrors: string[];
  permissionsSavedAt: number | null;
  mcpEnabled: boolean;
  mcpServers: McpServerStatus[];
};

type AppStore = {
  state: AppState;
  sendMessage: (text: string) => void;
  runSlashCommand: (raw: string) => boolean;
  requestKillSwitch: () => void;
  switchThread: (sessionId: string) => void;
  createThread: (displayName?: string) => void;
  renameThread: (sessionId: string, displayName: string) => void;
  clearCurrentThread: () => void;
  deleteThread: (sessionId: string) => void;
  toggleToolOpen: (entryId: string) => void;
  setToolOutputsExpanded: (value: boolean) => void;
  setShowToolCalls: (value: boolean) => void;
  updateApprovalRule: (entryId: string, value: string) => void;
  submitToolApproval: (entryId: string, decision: ToolApprovalDecision) => void;
  savePermissions: () => void;
  updatePermissionsField: (field: "allow" | "deny", value: string) => void;
  updatePermissionsEnabled: (value: boolean) => void;
  refreshPermissions: () => void;
  refreshThreads: () => void;
};

const DEFAULT_STATE: AppState = {
  entries: [],
  threads: [],
  currentSessionId: null,
  connectionState: "connecting",
  isWaiting: false,
  killRequested: false,
  showReconnectOverlay: false,
  toolOutputsExpanded: true,
  showToolCalls: true,
  permissionsLoaded: false,
  permissionsSaving: false,
  permissionsEnabled: true,
  permissionsAllowText: "",
  permissionsDenyText: "",
  permissionsErrors: [],
  permissionsSavedAt: null,
  mcpEnabled: true,
  mcpServers: [],
};

const AppStoreContext = createContext<AppStore | null>(null);

function readBooleanPreference(key: string, fallback: boolean): boolean {
  try {
    const raw = window.localStorage.getItem(key);
    if (raw === "true") return true;
    if (raw === "false") return false;
    return fallback;
  } catch {
    return fallback;
  }
}

function writeBooleanPreference(key: string, value: boolean): void {
  try {
    window.localStorage.setItem(key, String(value));
  } catch {
    // Ignore storage errors.
  }
}

function formatArgs(args: unknown): string {
  if (typeof args === "object" && args !== null) {
    return JSON.stringify(args, null, 2);
  }

  if (typeof args === "string") {
    try {
      return JSON.stringify(JSON.parse(args), null, 2);
    } catch {
      return args;
    }
  }

  return String(args ?? "");
}

function makePreview(content: string): string {
  const compact = content.replace(/\s+/g, " ").trim();
  if (!compact) {
    return "(no arguments)";
  }

  if (compact.length <= 120) {
    return compact;
  }

  return `${compact.slice(0, 120)}...`;
}

function normalizeUsage(usage?: Partial<TokenUsage>): TokenUsage | undefined {
  if (!usage) {
    return undefined;
  }

  const promptTokens = Math.max(
    0,
    Math.round(Number(usage.prompt_tokens ?? 0)),
  );
  const completionTokens = Math.max(
    0,
    Math.round(Number(usage.completion_tokens ?? 0)),
  );
  const providedTotal = Math.max(
    0,
    Math.round(Number(usage.total_tokens ?? 0)),
  );
  const totalTokens =
    providedTotal > 0 ? providedTotal : promptTokens + completionTokens;

  if (promptTokens === 0 && completionTokens === 0 && totalTokens === 0) {
    return undefined;
  }

  return {
    prompt_tokens: promptTokens,
    completion_tokens: completionTokens,
    total_tokens: totalTokens,
  };
}

function normalizeToolStatus(
  raw: unknown,
  successFallback = false,
): ToolStatus {
  if (
    raw === "running" ||
    raw === "success" ||
    raw === "failed" ||
    raw === "denied"
  ) {
    return raw;
  }
  if (raw === "failure") {
    return "failed";
  }
  return successFallback ? "success" : "failed";
}

function parseRulesInput(raw: string): string[] {
  return raw
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

function findToolIndex(entries: Entry[], callId: string, name: string): number {
  if (callId.trim()) {
    for (let i = entries.length - 1; i >= 0; i -= 1) {
      const entry = entries[i];
      if (entry.kind === "tool" && entry.callId === callId) {
        return i;
      }
    }
  }

  for (let i = entries.length - 1; i >= 0; i -= 1) {
    const entry = entries[i];
    if (entry.kind === "tool" && !entry.resolved && entry.name === name) {
      return i;
    }
  }

  return -1;
}

export function AppStoreProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<AppState>(DEFAULT_STATE);
  const stateRef = useRef(state);
  const wsRef = useRef<WebSocket | null>(null);
  const reconnectTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const reconnectDelayRef = useRef(1000);
  const disposedRef = useRef(false);
  const idCounterRef = useRef(0);
  const currentAssistantIdRef = useRef<string | null>(null);
  const activeResponseStartedAtRef = useRef<number | null>(null);
  const activeToolCallCountRef = useRef(0);
  const activeToolSuccessCountRef = useRef(0);
  const activeToolFailureCountRef = useRef(0);
  const activeToolDeniedCountRef = useRef(0);

  useEffect(() => {
    stateRef.current = state;
  }, [state]);

  const nextId = useCallback((): string => {
    idCounterRef.current += 1;
    return `${Date.now()}-${idCounterRef.current}`;
  }, []);

  const resetActiveResponseState = useCallback((): void => {
    activeResponseStartedAtRef.current = null;
    activeToolCallCountRef.current = 0;
    activeToolSuccessCountRef.current = 0;
    activeToolFailureCountRef.current = 0;
    activeToolDeniedCountRef.current = 0;
  }, []);

  const finishCurrentBubble = useCallback((): void => {
    currentAssistantIdRef.current = null;
  }, []);

  const sendControl = useCallback((payload: Record<string, unknown>): void => {
    const ws = wsRef.current;
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      return;
    }
    ws.send(JSON.stringify(payload));
  }, []);

  const applyThreadState = useCallback(
    (nextThreads?: ThreadRecord[], currentId?: string): void => {
      setState((prev) => ({
        ...prev,
        threads: Array.isArray(nextThreads) ? [...nextThreads] : prev.threads,
        currentSessionId: currentId ?? prev.currentSessionId,
      }));
    },
    [],
  );

  const applyPermissionsState = useCallback(
    (permissions?: PermissionsState, validationErrors: string[] = []): void => {
      setState((prev) => ({
        ...prev,
        permissionsLoaded: true,
        permissionsSaving: false,
        permissionsEnabled:
          typeof permissions?.enabled === "boolean"
            ? permissions.enabled
            : prev.permissionsEnabled,
        permissionsAllowText: Array.isArray(permissions?.tools?.allow)
          ? permissions.tools.allow.join("\n")
          : prev.permissionsAllowText,
        permissionsDenyText: Array.isArray(permissions?.tools?.deny)
          ? permissions.tools.deny.join("\n")
          : prev.permissionsDenyText,
        permissionsErrors: validationErrors,
      }));
    },
    [],
  );

  const applyMcpStatus = useCallback((status?: McpStatusSnapshot): void => {
    setState((prev) => ({
      ...prev,
      mcpEnabled:
        typeof status?.enabled === "boolean" ? status.enabled : prev.mcpEnabled,
      mcpServers: Array.isArray(status?.servers)
        ? [...status.servers]
        : prev.mcpServers,
    }));
  }, []);

  const hydrateEntriesFromHistory = useCallback(
    (history: HistoryMessage[], toolOutputsExpanded: boolean): Entry[] => {
      const rebuilt: Entry[] = [];
      const toolCallIndex = new Map<string, number>();

      for (const msg of history) {
        if (msg.role === "user") {
          rebuilt.push({
            id: nextId(),
            kind: "message",
            role: "user",
            text: msg.content,
          });
          continue;
        }

        if (msg.role === "assistant") {
          let parsed: { content?: unknown; tool_calls?: unknown } | null = null;
          try {
            const raw = JSON.parse(msg.content) as {
              content?: unknown;
              tool_calls?: unknown;
            };
            if (
              raw &&
              typeof raw === "object" &&
              Array.isArray(raw.tool_calls)
            ) {
              parsed = raw;
            }
          } catch {
            // Keep plain assistant message fallback.
          }

          if (!parsed) {
            rebuilt.push({
              id: nextId(),
              kind: "message",
              role: "assistant",
              text: msg.content,
            });
            continue;
          }

          if (
            typeof parsed.content === "string" &&
            parsed.content.trim().length > 0
          ) {
            rebuilt.push({
              id: nextId(),
              kind: "message",
              role: "assistant",
              text: parsed.content,
            });
          }

          const toolCalls = parsed.tool_calls as Array<Record<string, unknown>>;
          for (const call of toolCalls) {
            const callId =
              typeof call.id === "string" && call.id.trim()
                ? call.id
                : nextId();
            const name =
              typeof call.name === "string" ? call.name : "unknown_tool";
            const formattedArgs = formatArgs(call.arguments ?? {});

            const entry: ToolEntry = {
              id: nextId(),
              kind: "tool",
              callId,
              name,
              args: formattedArgs,
              argsPreview: makePreview(formattedArgs),
              output: "",
              status: "running",
              awaitingApproval: false,
              resolved: false,
              open: toolOutputsExpanded,
            };

            const idx = rebuilt.push(entry) - 1;
            toolCallIndex.set(callId, idx);
          }

          continue;
        }

        if (msg.role === "tool") {
          let callId = "";
          let output = msg.content;
          let status: ToolStatus = "success";

          try {
            const parsed = JSON.parse(msg.content) as {
              tool_call_id?: unknown;
              content?: unknown;
              status?: unknown;
            };
            if (typeof parsed.tool_call_id === "string") {
              callId = parsed.tool_call_id;
            }
            if (typeof parsed.content === "string") {
              output = parsed.content;
            }
            status = normalizeToolStatus(parsed.status, true);
          } catch {
            // Keep fallback output.
          }

          const idx = callId ? toolCallIndex.get(callId) : undefined;
          if (idx !== undefined) {
            const current = rebuilt[idx];
            if (current?.kind === "tool") {
              rebuilt[idx] = {
                ...current,
                output,
                status,
                awaitingApproval: false,
                approval: undefined,
                resolved: true,
                open: toolOutputsExpanded,
              };
            }
          } else {
            rebuilt.push({
              id: nextId(),
              kind: "message",
              role: "assistant",
              text: output,
            });
          }
        }
      }

      for (let i = 0; i < rebuilt.length; i += 1) {
        const entry = rebuilt[i];
        if (entry.kind === "tool" && !entry.resolved) {
          rebuilt[i] = {
            ...entry,
            resolved: true,
            status: "failed",
            awaitingApproval: false,
            approval: undefined,
            output:
              entry.output || "No stored result found for this tool call.",
            open: toolOutputsExpanded,
          };
        }
      }

      return rebuilt;
    },
    [nextId],
  );

  const hydrateCurrentThread = useCallback(
    (history: HistoryMessage[]): void => {
      const toolOutputsExpanded = stateRef.current.toolOutputsExpanded;
      setState((prev) => ({
        ...prev,
        entries: hydrateEntriesFromHistory(history, toolOutputsExpanded),
        isWaiting: false,
        killRequested: false,
      }));
      finishCurrentBubble();
      resetActiveResponseState();
    },
    [finishCurrentBubble, hydrateEntriesFromHistory, resetActiveResponseState],
  );

  const buildResponseStats = useCallback((event: DoneEvent): ResponseStats => {
    const elapsedMs =
      typeof event.elapsed_ms === "number" && event.elapsed_ms >= 0
        ? event.elapsed_ms
        : activeResponseStartedAtRef.current
          ? Date.now() - activeResponseStartedAtRef.current
          : 0;

    const toolCalls =
      typeof event.tool_call_count === "number"
        ? event.tool_call_count
        : activeToolCallCountRef.current;
    const toolSuccess =
      typeof event.tool_call_success === "number"
        ? event.tool_call_success
        : activeToolSuccessCountRef.current;
    const toolFailed =
      typeof event.tool_call_failed === "number"
        ? event.tool_call_failed
        : activeToolFailureCountRef.current;
    const toolDenied =
      typeof event.tool_call_denied === "number"
        ? event.tool_call_denied
        : activeToolDeniedCountRef.current;

    return {
      elapsedMs,
      toolCalls,
      toolSuccess,
      toolFailed,
      toolDenied,
      usage: normalizeUsage(event.usage),
    };
  }, []);

  const pushAssistantNote = useCallback(
    (text: string): void => {
      setState((prev) => ({
        ...prev,
        entries: [
          ...prev.entries,
          {
            id: nextId(),
            kind: "message",
            role: "assistant",
            text,
          } as MessageEntry,
        ],
      }));
    },
    [nextId],
  );

  const onError = useCallback(
    (message: string): void => {
      setState((prev) => ({
        ...prev,
        entries: [
          ...prev.entries,
          {
            id: nextId(),
            kind: "message",
            role: "assistant",
            text: `Error: ${message}`,
            error: true,
          } as MessageEntry,
        ],
        isWaiting: false,
        killRequested: false,
        permissionsSaving: false,
      }));
      finishCurrentBubble();
      resetActiveResponseState();
    },
    [finishCurrentBubble, nextId, resetActiveResponseState],
  );

  const onUserMessage = useCallback(
    (text: string): void => {
      if (!text.trim()) {
        return;
      }

      setState((prev) => ({
        ...prev,
        entries: [
          ...prev.entries,
          {
            id: nextId(),
            kind: "message",
            role: "user",
            text,
          } as MessageEntry,
        ],
        isWaiting: true,
      }));

      activeResponseStartedAtRef.current = Date.now();
      activeToolCallCountRef.current = 0;
      activeToolSuccessCountRef.current = 0;
      activeToolFailureCountRef.current = 0;
      activeToolDeniedCountRef.current = 0;
      finishCurrentBubble();
    },
    [finishCurrentBubble, nextId],
  );

  const onChunk = useCallback(
    (text: string): void => {
      if (!activeResponseStartedAtRef.current) {
        activeResponseStartedAtRef.current = Date.now();
      }

      setState((prev) => {
        let entries = [...prev.entries];
        let assistantId = currentAssistantIdRef.current;

        if (!assistantId) {
          assistantId = nextId();
          currentAssistantIdRef.current = assistantId;
          entries.push({
            id: assistantId,
            kind: "message",
            role: "assistant",
            text: "",
          });
        }

        entries = entries.map((entry) => {
          if (entry.kind !== "message" || entry.id !== assistantId) {
            return entry;
          }

          return {
            ...entry,
            text: entry.text + text,
          };
        });

        return {
          ...prev,
          entries,
          isWaiting: true,
        };
      });
    },
    [nextId],
  );

  const onToolCallStart = useCallback(
    (callId: string, name: string, args: unknown): void => {
      if (!activeResponseStartedAtRef.current) {
        activeResponseStartedAtRef.current = Date.now();
      }
      finishCurrentBubble();
      activeToolCallCountRef.current += 1;

      const formattedArgs = formatArgs(args);
      const entry: ToolEntry = {
        id: nextId(),
        kind: "tool",
        callId: callId.trim() || nextId(),
        name: name || "unknown_tool",
        args: formattedArgs,
        argsPreview: makePreview(formattedArgs),
        output: "",
        status: "running",
        awaitingApproval: false,
        resolved: false,
        open: stateRef.current.toolOutputsExpanded,
      };

      setState((prev) => ({
        ...prev,
        entries: [...prev.entries, entry],
        isWaiting: true,
      }));
    },
    [finishCurrentBubble, nextId],
  );

  const onToolCallResult = useCallback((event: ToolCallResultEvent): void => {
    if (!activeResponseStartedAtRef.current) {
      activeResponseStartedAtRef.current = Date.now();
    }

    const status = normalizeToolStatus(event.status, Boolean(event.success));
    const awaitingApproval = Boolean(event.awaiting_approval);

    if (!awaitingApproval) {
      if (status === "success") activeToolSuccessCountRef.current += 1;
      if (status === "denied") activeToolDeniedCountRef.current += 1;
      if (status === "failed") activeToolFailureCountRef.current += 1;
    }

    setState((prev) => {
      const idx = findToolIndex(
        prev.entries,
        event.call_id ?? "",
        event.name ?? "",
      );
      if (idx === -1) {
        return {
          ...prev,
          isWaiting: true,
        };
      }

      const updated = [...prev.entries];
      const entry = updated[idx];
      if (entry.kind !== "tool") {
        return {
          ...prev,
          isWaiting: true,
        };
      }

      updated[idx] = {
        ...entry,
        output: event.output ?? "",
        status,
        awaitingApproval,
        resolved: !awaitingApproval,
        approval: awaitingApproval ? entry.approval : undefined,
        open: entry.open || prev.toolOutputsExpanded,
      };

      return {
        ...prev,
        entries: updated,
        isWaiting: true,
      };
    });
  }, []);

  const onToolApprovalRequired = useCallback(
    (event: ToolApprovalRequiredEvent): void => {
      const callId = event.call_id ?? "";
      const name = event.name ?? "";
      const requestId = event.request_id ?? "";
      if (!requestId) {
        return;
      }

      setState((prev) => {
        const idx = findToolIndex(prev.entries, callId, name);
        if (idx === -1) {
          return prev;
        }

        const updated = [...prev.entries];
        const entry = updated[idx];
        if (entry.kind !== "tool") {
          return prev;
        }

        const suggested = (event.suggested_allow_rule ?? "").trim();
        updated[idx] = {
          ...entry,
          status: "denied",
          awaitingApproval: true,
          resolved: false,
          output: event.deny_reason ?? entry.output,
          approval: {
            requestId,
            suggestedAllowRule: suggested,
            allowRuleInput: suggested,
            submitting: false,
          },
          open: true,
        };

        return {
          ...prev,
          entries: updated,
        };
      });
    },
    [],
  );

  const onDone = useCallback(
    (event: DoneEvent): void => {
      const stats = buildResponseStats(event);
      const fullResponse = event.full_response ?? "";

      setState((prev) => {
        if (currentAssistantIdRef.current) {
          return {
            ...prev,
            entries: prev.entries.map((entry) => {
              if (
                entry.kind !== "message" ||
                entry.id !== currentAssistantIdRef.current
              ) {
                return entry;
              }
              return {
                ...entry,
                text: fullResponse,
                stats,
              };
            }),
            isWaiting: false,
            killRequested: false,
          };
        }

        if (fullResponse) {
          return {
            ...prev,
            entries: [
              ...prev.entries,
              {
                id: nextId(),
                kind: "message",
                role: "assistant",
                text: fullResponse,
                stats,
              } as MessageEntry,
            ],
            isWaiting: false,
            killRequested: false,
          };
        }

        return {
          ...prev,
          isWaiting: false,
          killRequested: false,
        };
      });

      finishCurrentBubble();
      resetActiveResponseState();
    },
    [buildResponseStats, finishCurrentBubble, nextId, resetActiveResponseState],
  );

  const onStopped = useCallback(
    (event: StoppedEvent): void => {
      setState((prev) => ({
        ...prev,
        entries: prev.entries.map((entry) => {
          if (entry.kind !== "tool" || entry.resolved) {
            return entry;
          }

          return {
            ...entry,
            resolved: true,
            status: "failed",
            awaitingApproval: false,
            approval: undefined,
            output: entry.output || "Stopped by user.",
            open: entry.open || prev.toolOutputsExpanded,
          };
        }),
        isWaiting: false,
        killRequested: false,
      }));

      finishCurrentBubble();
      resetActiveResponseState();

      if (event.reason === "internal_cancel") {
        onError("Run stopped due to an internal cancellation.");
      }
    },
    [finishCurrentBubble, onError, resetActiveResponseState],
  );

  const handleEvent = useCallback(
    (event: ServerEvent): void => {
      switch (event.type) {
        case "user_message":
          onUserMessage(event.content ?? "");
          return;
        case "chunk":
          onChunk(event.content ?? "");
          return;
        case "tool_call_start":
          onToolCallStart(
            event.call_id ?? "",
            event.name ?? "",
            event.args ?? "",
          );
          return;
        case "tool_call_result":
          onToolCallResult(event);
          return;
        case "tool_approval_required":
          onToolApprovalRequired(event);
          return;
        case "done":
          onDone(event);
          return;
        case "stopped":
          onStopped(event);
          return;
        case "thread_list":
          applyThreadState(event.sessions, event.current_session_id);
          return;
        case "thread_created":
        case "thread_switched":
        case "thread_cleared":
        case "thread_deleted": {
          const fallbackSessionId =
            "session_id" in event ? event.session_id : undefined;
          applyThreadState(
            event.sessions,
            event.current_session_id ?? fallbackSessionId,
          );
          hydrateCurrentThread(event.history ?? []);
          return;
        }
        case "thread_renamed":
          applyThreadState(event.sessions, event.current_session_id);
          return;
        case "permissions_state":
          applyPermissionsState(
            event.permissions,
            event.validation_errors ?? [],
          );
          return;
        case "permissions_updated":
          applyPermissionsState(event.permissions, []);
          setState((prev) => ({ ...prev, permissionsSavedAt: Date.now() }));
          return;
        case "mcp_status":
          applyMcpStatus(event.mcp);
          return;
        case "error":
          onError(event.message ?? "Unknown error");
      }
    },
    [
      applyMcpStatus,
      applyPermissionsState,
      applyThreadState,
      hydrateCurrentThread,
      onChunk,
      onDone,
      onError,
      onStopped,
      onToolApprovalRequired,
      onToolCallResult,
      onToolCallStart,
      onUserMessage,
    ],
  );

  const scheduleReconnect = useCallback((): void => {
    if (disposedRef.current || reconnectTimerRef.current) {
      return;
    }

    setState((prev) => ({ ...prev, showReconnectOverlay: true }));

    reconnectTimerRef.current = setTimeout(() => {
      reconnectTimerRef.current = null;
      connect();
      reconnectDelayRef.current = Math.min(
        Math.round(reconnectDelayRef.current * 1.5),
        10000,
      );
    }, reconnectDelayRef.current);
  }, []);

  const connect = useCallback((): void => {
    if (disposedRef.current) {
      return;
    }

    setState((prev) => ({ ...prev, connectionState: "connecting" }));

    const proto = window.location.protocol === "https:" ? "wss" : "ws";
    const socket = new WebSocket(`${proto}://${window.location.host}/ws`);
    wsRef.current = socket;

    socket.onopen = () => {
      setState((prev) => ({
        ...prev,
        connectionState: "connected",
        showReconnectOverlay: false,
      }));

      reconnectDelayRef.current = 1000;

      if (reconnectTimerRef.current) {
        clearTimeout(reconnectTimerRef.current);
        reconnectTimerRef.current = null;
      }

      sendControl({ type: "thread_list" });
      sendControl({ type: "permissions_get" });
    };

    socket.onclose = () => {
      if (disposedRef.current) {
        return;
      }
      setState((prev) => ({ ...prev, connectionState: "disconnected" }));
      scheduleReconnect();
    };

    socket.onerror = () => {
      socket.close();
    };

    socket.onmessage = (messageEvent: MessageEvent<string>) => {
      try {
        const event = JSON.parse(messageEvent.data) as ServerEvent;
        handleEvent(event);
      } catch {
        // Ignore malformed server event.
      }
    };
  }, [handleEvent, scheduleReconnect, sendControl]);

  useEffect(() => {
    setState((prev) => ({
      ...prev,
      toolOutputsExpanded: readBooleanPreference(TOOL_PREFS_STORAGE_KEY, true),
      showToolCalls: readBooleanPreference(TOOL_VISIBILITY_STORAGE_KEY, true),
    }));

    connect();

    return () => {
      disposedRef.current = true;
      if (reconnectTimerRef.current) {
        clearTimeout(reconnectTimerRef.current);
      }
      wsRef.current?.close();
    };
  }, [connect]);

  const setToolOutputsExpanded = useCallback((value: boolean): void => {
    writeBooleanPreference(TOOL_PREFS_STORAGE_KEY, value);

    setState((prev) => ({
      ...prev,
      toolOutputsExpanded: value,
      entries: prev.entries.map((entry) =>
        entry.kind === "tool" ? { ...entry, open: value } : entry,
      ),
    }));
  }, []);

  const setShowToolCalls = useCallback((value: boolean): void => {
    writeBooleanPreference(TOOL_VISIBILITY_STORAGE_KEY, value);
    setState((prev) => ({ ...prev, showToolCalls: value }));
  }, []);

  const toggleToolOpen = useCallback((entryId: string): void => {
    setState((prev) => ({
      ...prev,
      entries: prev.entries.map((entry) => {
        if (entry.kind !== "tool" || entry.id !== entryId) {
          return entry;
        }
        return { ...entry, open: !entry.open };
      }),
    }));
  }, []);

  const updateApprovalRule = useCallback(
    (entryId: string, value: string): void => {
      setState((prev) => ({
        ...prev,
        entries: prev.entries.map((entry) => {
          if (
            entry.kind !== "tool" ||
            entry.id !== entryId ||
            !entry.approval
          ) {
            return entry;
          }
          return {
            ...entry,
            approval: {
              ...entry.approval,
              allowRuleInput: value,
            },
          };
        }),
      }));
    },
    [],
  );

  const submitToolApproval = useCallback(
    (entryId: string, decision: ToolApprovalDecision): void => {
      if (stateRef.current.connectionState !== "connected") {
        return;
      }

      const idx = stateRef.current.entries.findIndex(
        (entry) => entry.kind === "tool" && entry.id === entryId,
      );
      if (idx === -1) {
        return;
      }

      const current = stateRef.current.entries[idx];
      if (
        current.kind !== "tool" ||
        !current.approval ||
        current.approval.submitting
      ) {
        return;
      }

      const allowRule = current.approval.allowRuleInput.trim();
      if (decision === "allow_persist" && !allowRule) {
        onError("Allow rule is required for persistent approval.");
        return;
      }

      setState((prev) => ({
        ...prev,
        entries: prev.entries.map((entry, index) => {
          if (index !== idx || entry.kind !== "tool" || !entry.approval) {
            return entry;
          }
          return {
            ...entry,
            approval: {
              ...entry.approval,
              submitting: true,
            },
          };
        }),
      }));

      sendControl({
        type: "tool_approval_decision",
        request_id: current.approval.requestId,
        decision,
        allow_rule: allowRule,
      });
    },
    [onError, sendControl],
  );

  const requestKillSwitch = useCallback((): void => {
    const snapshot = stateRef.current;
    if (
      snapshot.connectionState !== "connected" ||
      !snapshot.isWaiting ||
      snapshot.killRequested
    ) {
      return;
    }

    setState((prev) => ({ ...prev, killRequested: true }));
    sendControl({ type: "kill_switch" });
  }, [sendControl]);

  const switchThread = useCallback(
    (sessionId: string): void => {
      const snapshot = stateRef.current;
      if (snapshot.isWaiting || sessionId === snapshot.currentSessionId) {
        return;
      }

      setState((prev) => ({
        ...prev,
        isWaiting: false,
        killRequested: false,
      }));
      resetActiveResponseState();
      sendControl({ type: "thread_switch", session_id: sessionId });
    },
    [resetActiveResponseState, sendControl],
  );

  const createThread = useCallback(
    (displayName?: string): void => {
      sendControl({
        type: "thread_create",
        ...(displayName ? { display_name: displayName } : {}),
      });
    },
    [sendControl],
  );

  const renameThread = useCallback(
    (sessionId: string, displayName: string): void => {
      if (!sessionId || !displayName.trim()) {
        return;
      }
      sendControl({
        type: "thread_rename",
        session_id: sessionId,
        display_name: displayName,
      });
    },
    [sendControl],
  );

  const clearCurrentThread = useCallback((): void => {
    if (!stateRef.current.currentSessionId) {
      return;
    }
    setState((prev) => ({
      ...prev,
      isWaiting: false,
      killRequested: false,
    }));
    resetActiveResponseState();
    sendControl({ type: "thread_clear" });
  }, [resetActiveResponseState, sendControl]);

  const deleteThread = useCallback(
    (sessionId: string): void => {
      if (!sessionId) {
        return;
      }
      setState((prev) => ({
        ...prev,
        isWaiting: false,
        killRequested: false,
      }));
      resetActiveResponseState();
      sendControl({ type: "thread_delete", session_id: sessionId });
    },
    [resetActiveResponseState, sendControl],
  );

  const refreshPermissions = useCallback((): void => {
    sendControl({ type: "permissions_get" });
  }, [sendControl]);

  const refreshThreads = useCallback((): void => {
    sendControl({ type: "thread_list" });
  }, [sendControl]);

  const savePermissions = useCallback((): void => {
    const snapshot = stateRef.current;
    if (
      snapshot.connectionState !== "connected" ||
      snapshot.permissionsSaving
    ) {
      return;
    }

    setState((prev) => ({
      ...prev,
      permissionsSaving: true,
      permissionsErrors: [],
    }));

    sendControl({
      type: "permissions_set",
      enabled: snapshot.permissionsEnabled,
      allow: parseRulesInput(snapshot.permissionsAllowText),
      deny: parseRulesInput(snapshot.permissionsDenyText),
    });
  }, [sendControl]);

  const updatePermissionsField = useCallback(
    (field: "allow" | "deny", value: string): void => {
      setState((prev) => ({
        ...prev,
        permissionsAllowText:
          field === "allow" ? value : prev.permissionsAllowText,
        permissionsDenyText:
          field === "deny" ? value : prev.permissionsDenyText,
      }));
    },
    [],
  );

  const updatePermissionsEnabled = useCallback((value: boolean): void => {
    setState((prev) => ({ ...prev, permissionsEnabled: value }));
  }, []);

  const runSlashCommand = useCallback(
    (raw: string): boolean => {
      if (!raw.startsWith("/")) {
        return false;
      }

      const body = raw.slice(1).trim();
      if (!body) {
        onError("Empty slash command");
        return true;
      }

      const firstSpace = body.indexOf(" ");
      const cmd = (
        firstSpace === -1 ? body : body.slice(0, firstSpace)
      ).toLowerCase();
      const arg = (firstSpace === -1 ? "" : body.slice(firstSpace + 1)).trim();

      if (cmd === "help") {
        pushAssistantNote(
          "Session commands: `/new [name]`, `/rename <name>`, `/clear`, `/delete`, `/stop`\nTool view: `/tools <collapse|expand|hide|show>`",
        );
        return true;
      }

      if (cmd === "stop") {
        requestKillSwitch();
        return true;
      }

      if (cmd === "tools") {
        const mode = arg.toLowerCase();
        if (mode === "collapse") {
          setToolOutputsExpanded(false);
          pushAssistantNote("Collapsed all tool outputs.");
          return true;
        }
        if (mode === "expand") {
          setToolOutputsExpanded(true);
          pushAssistantNote("Expanded all tool outputs.");
          return true;
        }
        if (mode === "hide") {
          setShowToolCalls(false);
          pushAssistantNote("Tool call blocks are now hidden.");
          return true;
        }
        if (mode === "show") {
          setShowToolCalls(true);
          pushAssistantNote("Tool call blocks are now visible.");
          return true;
        }
        onError("Usage: /tools <collapse|expand|hide|show>");
        return true;
      }

      if (cmd === "new") {
        createThread(arg || undefined);
        return true;
      }

      if (cmd === "rename") {
        if (!stateRef.current.currentSessionId) {
          onError("No active session to rename");
          return true;
        }
        if (!arg) {
          onError("Usage: /rename <display name>");
          return true;
        }
        renameThread(stateRef.current.currentSessionId, arg);
        return true;
      }

      if (cmd === "clear") {
        if (!stateRef.current.currentSessionId) {
          onError("No active session to clear");
          return true;
        }
        clearCurrentThread();
        return true;
      }

      if (cmd === "delete") {
        if (!stateRef.current.currentSessionId) {
          onError("No active session to delete");
          return true;
        }
        deleteThread(stateRef.current.currentSessionId);
        return true;
      }

      onError(`Unknown command: /${cmd}`);
      return true;
    },
    [
      clearCurrentThread,
      createThread,
      deleteThread,
      onError,
      pushAssistantNote,
      renameThread,
      requestKillSwitch,
      setShowToolCalls,
      setToolOutputsExpanded,
    ],
  );

  const sendMessage = useCallback(
    (rawText: string): void => {
      const text = rawText.trim();
      const ws = wsRef.current;
      if (
        !text ||
        stateRef.current.isWaiting ||
        !ws ||
        ws.readyState !== WebSocket.OPEN
      ) {
        return;
      }

      if (runSlashCommand(text)) {
        resetActiveResponseState();
        return;
      }

      ws.send(
        JSON.stringify({
          type: "message",
          content: text,
        }),
      );
    },
    [resetActiveResponseState, runSlashCommand],
  );

  const value = useMemo<AppStore>(
    () => ({
      state,
      sendMessage,
      runSlashCommand,
      requestKillSwitch,
      switchThread,
      createThread,
      renameThread,
      clearCurrentThread,
      deleteThread,
      toggleToolOpen,
      setToolOutputsExpanded,
      setShowToolCalls,
      updateApprovalRule,
      submitToolApproval,
      savePermissions,
      updatePermissionsField,
      updatePermissionsEnabled,
      refreshPermissions,
      refreshThreads,
    }),
    [
      clearCurrentThread,
      createThread,
      deleteThread,
      refreshPermissions,
      refreshThreads,
      renameThread,
      requestKillSwitch,
      runSlashCommand,
      savePermissions,
      sendMessage,
      setShowToolCalls,
      setToolOutputsExpanded,
      state,
      submitToolApproval,
      switchThread,
      toggleToolOpen,
      updateApprovalRule,
      updatePermissionsEnabled,
      updatePermissionsField,
    ],
  );

  return (
    <AppStoreContext.Provider value={value}>
      {children}
    </AppStoreContext.Provider>
  );
}

export function useAppStore(): AppStore {
  const context = useContext(AppStoreContext);
  if (!context) {
    throw new Error("useAppStore must be used inside AppStoreProvider");
  }
  return context;
}
