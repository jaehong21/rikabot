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
import axios from "axios";

import type {
  CreateThreadResponse,
  DeleteThreadResponse,
  DoneEvent,
  Entry,
  HistoryMessage,
  McpServerStatus,
  McpStatusSnapshot,
  MessageEntry,
  PermissionsState,
  PermissionsResponse,
  QueuedInput,
  RenameThreadResponse,
  ResponseStats,
  SkillContentResponse,
  ServerEvent,
  SkillStatus,
  SkillsResponse,
  SkillsStatusSnapshot,
  StoppedEvent,
  ThreadMessagesResponse,
  ThreadRecord,
  ThreadsResponse,
  TokenUsage,
  ToolApprovalDecision,
  ToolApprovalRequiredEvent,
  ToolCallResultEvent,
  ToolEntry,
  UpdateSkillContentResponse,
  ToolStatus,
} from "@/types/app";
import { axiosInstance } from "@/lib/api/axios";
import { isDevelopmentMode, resolveBackendHostPort } from "@/lib/runtime-env";

const TOOL_PREFS_STORAGE_KEY = "rika.toolOutputsExpanded";
const TOOL_VISIBILITY_STORAGE_KEY = "rika.showToolCalls";

type AppState = {
  entries: Entry[];
  threads: ThreadRecord[];
  currentSessionId: string | null;
  runningSessionIds: string[];
  connectionState: "connecting" | "connected" | "disconnected";
  isWaiting: boolean;
  waitingStartedAtMs: number | null;
  killRequested: boolean;
  showReconnectOverlay: boolean;
  toolOutputsExpanded: boolean;
  showToolCalls: boolean;
  queuedInputs: QueuedInput[];
  permissionsLoaded: boolean;
  permissionsSaving: boolean;
  permissionsEnabled: boolean;
  permissionsAllowText: string;
  permissionsDenyText: string;
  permissionsErrors: string[];
  permissionsSavedAt: number | null;
  mcpEnabled: boolean;
  mcpServers: McpServerStatus[];
  skillsLoaded: boolean;
  skillsSaving: boolean;
  skillsEnabled: boolean;
  skills: SkillStatus[];
  skillsErrors: string[];
  skillContentByPath: Record<string, string>;
  skillContentLoadingPath: string | null;
};

type AppStore = {
  state: AppState;
  sendMessage: (text: string) => void;
  runSlashCommand: (raw: string) => boolean;
  requestKillSwitch: () => void;
  cancelQueuedInput: (queueItemId?: string) => void;
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
  refreshSkills: () => void;
  loadSkillContent: (path: string) => void;
  saveSkill: (path: string, content: string) => void;
  refreshThreads: () => void;
};

const DEFAULT_STATE: AppState = {
  entries: [],
  threads: [],
  currentSessionId: null,
  runningSessionIds: [],
  connectionState: "connecting",
  isWaiting: false,
  waitingStartedAtMs: null,
  killRequested: false,
  showReconnectOverlay: false,
  toolOutputsExpanded: true,
  showToolCalls: true,
  queuedInputs: [],
  permissionsLoaded: false,
  permissionsSaving: false,
  permissionsEnabled: true,
  permissionsAllowText: "",
  permissionsDenyText: "",
  permissionsErrors: [],
  permissionsSavedAt: null,
  mcpEnabled: true,
  mcpServers: [],
  skillsLoaded: false,
  skillsSaving: false,
  skillsEnabled: true,
  skills: [],
  skillsErrors: [],
  skillContentByPath: {},
  skillContentLoadingPath: null,
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

function resolveBackendWsUrl(): string {
  const hostPort = resolveBackendHostPort();
  if (hostPort) {
    return `ws://${hostPort}/ws`;
  }

  if (isDevelopmentMode()) {
    // Backend default host is 127.0.0.1; this avoids localhost/IPv6 (::1) mismatches in browsers.
    return "ws://127.0.0.1:4728/ws";
  }

  const proto = window.location.protocol === "https:" ? "wss" : "ws";
  return `${proto}://${window.location.host}/ws`;
}

function getApiErrorMessage(error: unknown): string {
  if (axios.isAxiosError(error)) {
    const message =
      (error.response?.data as { error?: { message?: string } } | undefined)
        ?.error?.message ?? error.message;
    return message || "Request failed";
  }
  if (error instanceof Error) {
    return error.message;
  }
  return "Request failed";
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
  const currentThreadSnapshotRef = useRef<{
    sessionId: string | null;
    messageCount: number;
  }>({
    sessionId: null,
    messageCount: 0,
  });
  const activeResponseStartedAtRef = useRef<number | null>(null);
  const activeToolCallCountRef = useRef(0);
  const activeToolSuccessCountRef = useRef(0);
  const activeToolFailureCountRef = useRef(0);
  const activeToolDeniedCountRef = useRef(0);
  const runningSessionsRef = useRef<Set<string>>(new Set());
  const runningSessionStartedAtRef = useRef<Record<string, number>>({});
  const killRequestedSessionsRef = useRef<Set<string>>(new Set());
  const queueBySessionRef = useRef<Record<string, QueuedInput[]>>({});

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
        runningSessionIds: Array.from(runningSessionsRef.current),
        isWaiting: ((): boolean => {
          const sid = currentId ?? prev.currentSessionId;
          return Boolean(sid && runningSessionsRef.current.has(sid));
        })(),
        waitingStartedAtMs: ((): number | null => {
          const sid = currentId ?? prev.currentSessionId;
          if (!sid || !runningSessionsRef.current.has(sid)) {
            return null;
          }
          return runningSessionStartedAtRef.current[sid] ?? null;
        })(),
        killRequested: ((): boolean => {
          const sid = currentId ?? prev.currentSessionId;
          return Boolean(sid && killRequestedSessionsRef.current.has(sid));
        })(),
        queuedInputs: ((): QueuedInput[] => {
          const sid = currentId ?? prev.currentSessionId;
          if (!sid) {
            return [];
          }
          return [...(queueBySessionRef.current[sid] ?? [])];
        })(),
      }));
    },
    [],
  );

  const updateCurrentThreadSnapshot = useCallback(
    (threads?: ThreadRecord[], currentId?: string): void => {
      const list = Array.isArray(threads) ? threads : stateRef.current.threads;
      const sid = currentId ?? stateRef.current.currentSessionId;
      if (!sid) {
        currentThreadSnapshotRef.current = {
          sessionId: null,
          messageCount: 0,
        };
        return;
      }
      const match = list.find((item) => item.id === sid);
      currentThreadSnapshotRef.current = {
        sessionId: sid,
        messageCount: match?.message_count ?? 0,
      };
    },
    [],
  );

  const applyThreadStateAndSnapshot = useCallback(
    (threads?: ThreadRecord[], currentId?: string): void => {
      applyThreadState(threads, currentId);
      updateCurrentThreadSnapshot(threads, currentId);
    },
    [applyThreadState, updateCurrentThreadSnapshot],
  );

  const applyThreadListKeepingSelection = useCallback(
    (threads: ThreadRecord[]): void => {
      const current = stateRef.current.currentSessionId;
      const nextCurrent =
        current && threads.some((thread) => thread.id === current)
          ? current
          : (threads[0]?.id ?? null);
      applyThreadStateAndSnapshot(threads, nextCurrent ?? undefined);
    },
    [applyThreadStateAndSnapshot],
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

  const applySkillsStatus = useCallback(
    (skills?: SkillsStatusSnapshot, validationErrors: string[] = []): void => {
      setState((prev) => {
        const nextSkills = Array.isArray(skills?.skills)
          ? [...skills.skills]
          : prev.skills;
        const validPaths = new Set(nextSkills.map((skill) => skill.path));
        const nextContentByPath: Record<string, string> = {};
        for (const [path, content] of Object.entries(prev.skillContentByPath)) {
          if (validPaths.has(path)) {
            nextContentByPath[path] = content;
          }
        }

        return {
          ...prev,
          skillsLoaded: true,
          skillsSaving: false,
          skillsEnabled:
            typeof skills?.enabled === "boolean"
              ? skills.enabled
              : prev.skillsEnabled,
          skills: nextSkills,
          skillsErrors: validationErrors,
          skillContentByPath: nextContentByPath,
          skillContentLoadingPath:
            prev.skillContentLoadingPath &&
            validPaths.has(prev.skillContentLoadingPath)
              ? prev.skillContentLoadingPath
              : null,
        };
      });
    },
    [],
  );

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
    (sessionId: string | null, history: HistoryMessage[]): void => {
      const toolOutputsExpanded = stateRef.current.toolOutputsExpanded;
      setState((prev) => ({
        ...prev,
        entries: hydrateEntriesFromHistory(history, toolOutputsExpanded),
        runningSessionIds: Array.from(runningSessionsRef.current),
        isWaiting: Boolean(
          sessionId && runningSessionsRef.current.has(sessionId),
        ),
        waitingStartedAtMs:
          sessionId && runningSessionsRef.current.has(sessionId)
            ? (runningSessionStartedAtRef.current[sessionId] ?? null)
            : null,
        killRequested: Boolean(
          sessionId && killRequestedSessionsRef.current.has(sessionId),
        ),
        queuedInputs: sessionId
          ? [...(queueBySessionRef.current[sessionId] ?? [])]
          : [],
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
    (message: string, sessionId?: string): void => {
      if (sessionId) {
        runningSessionsRef.current.delete(sessionId);
        delete runningSessionStartedAtRef.current[sessionId];
        killRequestedSessionsRef.current.delete(sessionId);
      }

      const currentSessionId = stateRef.current.currentSessionId;
      const shouldRender = !sessionId || sessionId === currentSessionId;

      setState((prev) => ({
        ...prev,
        runningSessionIds: Array.from(runningSessionsRef.current),
        entries: shouldRender
          ? [
              ...prev.entries,
              {
                id: nextId(),
                kind: "message",
                role: "assistant",
                text: `Error: ${message}`,
                error: true,
              } as MessageEntry,
            ]
          : prev.entries,
        isWaiting: shouldRender ? false : prev.isWaiting,
        waitingStartedAtMs: shouldRender ? null : prev.waitingStartedAtMs,
        killRequested: shouldRender ? false : prev.killRequested,
        permissionsSaving: false,
        skillsSaving: false,
        skillContentLoadingPath: null,
      }));

      if (shouldRender) {
        finishCurrentBubble();
        resetActiveResponseState();
      }
    },
    [finishCurrentBubble, nextId, resetActiveResponseState],
  );

  const onUserMessage = useCallback(
    (sessionId: string, text: string, startedAtUnixMs?: number): void => {
      const normalizedText = text.trim();
      if (!normalizedText) {
        return;
      }

      const startMs =
        typeof startedAtUnixMs === "number" &&
        Number.isFinite(startedAtUnixMs) &&
        startedAtUnixMs > 0
          ? Math.round(startedAtUnixMs)
          : Date.now();

      runningSessionsRef.current.add(sessionId);
      runningSessionStartedAtRef.current[sessionId] = startMs;
      killRequestedSessionsRef.current.delete(sessionId);

      if (sessionId !== stateRef.current.currentSessionId) {
        setState((prev) => ({
          ...prev,
          runningSessionIds: Array.from(runningSessionsRef.current),
        }));
        return;
      }

      setState((prev) => {
        const lastEntry = prev.entries[prev.entries.length - 1];
        const isDuplicateWhileWaiting =
          prev.isWaiting &&
          lastEntry?.kind === "message" &&
          lastEntry.role === "user" &&
          lastEntry.text.trim() === normalizedText;

        if (isDuplicateWhileWaiting) {
          return {
            ...prev,
            runningSessionIds: Array.from(runningSessionsRef.current),
            isWaiting: true,
            waitingStartedAtMs: startMs,
            killRequested: false,
          };
        }

        return {
          ...prev,
          runningSessionIds: Array.from(runningSessionsRef.current),
          entries: [
            ...prev.entries,
            {
              id: nextId(),
              kind: "message",
              role: "user",
              text: normalizedText,
            } as MessageEntry,
          ],
          isWaiting: true,
          waitingStartedAtMs: startMs,
          killRequested: false,
        };
      });

      activeResponseStartedAtRef.current = startMs;
      activeToolCallCountRef.current = 0;
      activeToolSuccessCountRef.current = 0;
      activeToolFailureCountRef.current = 0;
      activeToolDeniedCountRef.current = 0;
      finishCurrentBubble();
    },
    [finishCurrentBubble, nextId],
  );

  const onChunk = useCallback(
    (sessionId: string, text: string): void => {
      if (sessionId !== stateRef.current.currentSessionId) {
        return;
      }

      if (!activeResponseStartedAtRef.current) {
        activeResponseStartedAtRef.current = Date.now();
      }
      if (!runningSessionStartedAtRef.current[sessionId]) {
        runningSessionStartedAtRef.current[sessionId] =
          activeResponseStartedAtRef.current;
      }

      setState((prev) => {
        let entries = [...prev.entries];
        let assistantId = currentAssistantIdRef.current;
        const startedAtMs = activeResponseStartedAtRef.current ?? Date.now();

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
          waitingStartedAtMs: prev.waitingStartedAtMs ?? startedAtMs,
        };
      });
    },
    [nextId],
  );

  const onToolCallStart = useCallback(
    (sessionId: string, callId: string, name: string, args: unknown): void => {
      if (sessionId !== stateRef.current.currentSessionId) {
        return;
      }

      if (!activeResponseStartedAtRef.current) {
        activeResponseStartedAtRef.current = Date.now();
      }
      if (!runningSessionStartedAtRef.current[sessionId]) {
        runningSessionStartedAtRef.current[sessionId] =
          activeResponseStartedAtRef.current;
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
        waitingStartedAtMs:
          prev.waitingStartedAtMs ?? activeResponseStartedAtRef.current,
      }));
    },
    [finishCurrentBubble, nextId],
  );

  const onToolCallResult = useCallback(
    (sessionId: string, event: ToolCallResultEvent): void => {
      if (sessionId !== stateRef.current.currentSessionId) {
        return;
      }

      if (!activeResponseStartedAtRef.current) {
        activeResponseStartedAtRef.current = Date.now();
      }
      if (!runningSessionStartedAtRef.current[sessionId]) {
        runningSessionStartedAtRef.current[sessionId] =
          activeResponseStartedAtRef.current;
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
            waitingStartedAtMs:
              prev.waitingStartedAtMs ?? activeResponseStartedAtRef.current,
          };
        }

        const updated = [...prev.entries];
        const entry = updated[idx];
        if (entry.kind !== "tool") {
          return {
            ...prev,
            isWaiting: true,
            waitingStartedAtMs:
              prev.waitingStartedAtMs ?? activeResponseStartedAtRef.current,
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
          waitingStartedAtMs:
            prev.waitingStartedAtMs ?? activeResponseStartedAtRef.current,
        };
      });
    },
    [],
  );

  const onToolApprovalRequired = useCallback(
    (sessionId: string, event: ToolApprovalRequiredEvent): void => {
      if (sessionId !== stateRef.current.currentSessionId) {
        return;
      }

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

  const onSkillContent = useCallback((path: string, content: string): void => {
    if (!path.trim()) {
      return;
    }

    setState((prev) => ({
      ...prev,
      skillContentByPath: {
        ...prev.skillContentByPath,
        [path]: content,
      },
      skillContentLoadingPath:
        prev.skillContentLoadingPath === path
          ? null
          : prev.skillContentLoadingPath,
    }));
  }, []);

  const onDone = useCallback(
    (sessionId: string, event: DoneEvent): void => {
      runningSessionsRef.current.delete(sessionId);
      delete runningSessionStartedAtRef.current[sessionId];
      killRequestedSessionsRef.current.delete(sessionId);
      if (sessionId !== stateRef.current.currentSessionId) {
        setState((prev) => ({
          ...prev,
          runningSessionIds: Array.from(runningSessionsRef.current),
        }));
        return;
      }

      const stats = buildResponseStats(event);
      const fullResponse = event.full_response ?? "";

      setState((prev) => {
        if (currentAssistantIdRef.current) {
          return {
            ...prev,
            runningSessionIds: Array.from(runningSessionsRef.current),
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
            waitingStartedAtMs: null,
            killRequested: false,
          };
        }

        if (fullResponse) {
          return {
            ...prev,
            runningSessionIds: Array.from(runningSessionsRef.current),
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
            waitingStartedAtMs: null,
            killRequested: false,
          };
        }

        return {
          ...prev,
          runningSessionIds: Array.from(runningSessionsRef.current),
          isWaiting: false,
          waitingStartedAtMs: null,
          killRequested: false,
        };
      });

      finishCurrentBubble();
      resetActiveResponseState();
    },
    [buildResponseStats, finishCurrentBubble, nextId, resetActiveResponseState],
  );

  const onStopped = useCallback(
    (sessionId: string, event: StoppedEvent): void => {
      runningSessionsRef.current.delete(sessionId);
      delete runningSessionStartedAtRef.current[sessionId];
      killRequestedSessionsRef.current.delete(sessionId);
      if (sessionId !== stateRef.current.currentSessionId) {
        setState((prev) => ({
          ...prev,
          runningSessionIds: Array.from(runningSessionsRef.current),
        }));
        return;
      }

      setState((prev) => ({
        ...prev,
        runningSessionIds: Array.from(runningSessionsRef.current),
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
        waitingStartedAtMs: null,
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

  const onQueueUpdated = useCallback(
    (sessionId: string, items: QueuedInput[]): void => {
      queueBySessionRef.current[sessionId] = [...items];
      if (sessionId !== stateRef.current.currentSessionId) {
        return;
      }
      setState((prev) => ({
        ...prev,
        queuedInputs: [...items],
      }));
    },
    [],
  );

  const handleEvent = useCallback(
    (event: ServerEvent): void => {
      const rawSessionId =
        typeof (event as { session_id?: unknown }).session_id === "string"
          ? ((event as { session_id?: string }).session_id ?? "").trim()
          : "";
      const scopedSessionId =
        rawSessionId || stateRef.current.currentSessionId || "";

      switch (event.type) {
        case "user_message":
          if (scopedSessionId) {
            onUserMessage(
              scopedSessionId,
              event.content ?? "",
              event.started_at_unix_ms,
            );
          }
          return;
        case "chunk":
          if (scopedSessionId) {
            onChunk(scopedSessionId, event.content ?? "");
          }
          return;
        case "tool_call_start":
          if (scopedSessionId) {
            onToolCallStart(
              scopedSessionId,
              event.call_id ?? "",
              event.name ?? "",
              event.args ?? "",
            );
          }
          return;
        case "tool_call_result":
          if (scopedSessionId) {
            onToolCallResult(scopedSessionId, event);
          }
          return;
        case "tool_approval_required":
          if (scopedSessionId) {
            onToolApprovalRequired(scopedSessionId, event);
          }
          return;
        case "done":
          if (scopedSessionId) {
            onDone(scopedSessionId, event);
          }
          return;
        case "stopped":
          if (scopedSessionId) {
            onStopped(scopedSessionId, event);
          }
          return;
        case "queue_updated":
          if (scopedSessionId) {
            onQueueUpdated(
              scopedSessionId,
              Array.isArray(event.items) ? event.items : [],
            );
          }
          return;
        case "thread_list":
          if (Array.isArray(event.sessions)) {
            applyThreadListKeepingSelection(event.sessions);
          }
          return;
        case "permissions_updated":
          applyPermissionsState(event.permissions, []);
          setState((prev) => ({ ...prev, permissionsSavedAt: Date.now() }));
          return;
        case "mcp_status":
          applyMcpStatus(event.mcp);
          return;
        case "error":
          onError(event.message ?? "Unknown error", rawSessionId || undefined);
      }
    },
    [
      applyThreadListKeepingSelection,
      applyMcpStatus,
      applyPermissionsState,
      onChunk,
      onDone,
      onError,
      onQueueUpdated,
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

    const socket = new WebSocket(resolveBackendWsUrl());
    wsRef.current = socket;

    socket.onopen = () => {
      runningSessionsRef.current.clear();
      runningSessionStartedAtRef.current = {};
      killRequestedSessionsRef.current.clear();
      queueBySessionRef.current = {};

      setState((prev) => ({
        ...prev,
        connectionState: "connected",
        showReconnectOverlay: false,
        runningSessionIds: [],
        isWaiting: false,
        waitingStartedAtMs: null,
        killRequested: false,
        queuedInputs: [],
      }));

      reconnectDelayRef.current = 1000;

      if (reconnectTimerRef.current) {
        clearTimeout(reconnectTimerRef.current);
        reconnectTimerRef.current = null;
      }

      void (async () => {
        try {
          const [threadsRes, permissionsRes, skillsRes] = await Promise.all([
            axiosInstance.get<ThreadsResponse>("/api/threads"),
            axiosInstance.get<PermissionsResponse>("/api/settings/permissions"),
            axiosInstance.get<SkillsResponse>("/api/settings/skills"),
          ]);
          const sessions = threadsRes.data.sessions ?? [];
          const current = stateRef.current.currentSessionId;
          const nextSessionId =
            current && sessions.some((thread) => thread.id === current)
              ? current
              : (sessions[0]?.id ?? null);

          applyThreadStateAndSnapshot(sessions, nextSessionId ?? undefined);
          if (nextSessionId) {
            const historyRes = await axiosInstance.get<ThreadMessagesResponse>(
              `/api/threads/${encodeURIComponent(nextSessionId)}/messages`,
            );
            hydrateCurrentThread(nextSessionId, historyRes.data.history ?? []);
          } else {
            hydrateCurrentThread(null, []);
          }
          applyPermissionsState(permissionsRes.data.permissions, []);
          applySkillsStatus(skillsRes.data.skills, []);
        } catch (error) {
          onError(getApiErrorMessage(error));
        }
      })();
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
  }, [
    applyPermissionsState,
    applySkillsStatus,
    applyThreadStateAndSnapshot,
    handleEvent,
    hydrateCurrentThread,
    onError,
    scheduleReconnect,
  ]);

  useEffect(() => {
    // React StrictMode runs effect cleanup once in development before the real mount.
    // Reset disposed flag on each setup so the real mount can establish WebSocket.
    disposedRef.current = false;

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
    const sessionId = snapshot.currentSessionId;
    const isRunning = Boolean(
      sessionId && runningSessionsRef.current.has(sessionId),
    );
    const killRequested = Boolean(
      sessionId && killRequestedSessionsRef.current.has(sessionId),
    );
    if (
      snapshot.connectionState !== "connected" ||
      !sessionId ||
      !isRunning ||
      killRequested
    ) {
      return;
    }

    killRequestedSessionsRef.current.add(sessionId);
    setState((prev) => ({ ...prev, killRequested: true }));
    sendControl({
      type: "kill_switch",
      session_id: sessionId,
    });
  }, [sendControl]);

  const cancelQueuedInput = useCallback(
    (queueItemId?: string): void => {
      const snapshot = stateRef.current;
      const sessionId = snapshot.currentSessionId;
      if (snapshot.connectionState !== "connected" || !sessionId) {
        return;
      }

      sendControl({
        type: "queue_cancel",
        session_id: sessionId,
        ...(queueItemId ? { queue_item_id: queueItemId } : {}),
      });
    },
    [sendControl],
  );

  const switchThread = useCallback(
    (sessionId: string): void => {
      const snapshot = stateRef.current;
      if (sessionId === snapshot.currentSessionId) {
        return;
      }

      resetActiveResponseState();
      void (async () => {
        try {
          const [threadsRes, historyRes] = await Promise.all([
            axiosInstance.get<ThreadsResponse>("/api/threads"),
            axiosInstance.get<ThreadMessagesResponse>(
              `/api/threads/${encodeURIComponent(sessionId)}/messages`,
            ),
          ]);
          applyThreadStateAndSnapshot(threadsRes.data.sessions, sessionId);
          hydrateCurrentThread(sessionId, historyRes.data.history ?? []);
        } catch (error) {
          onError(getApiErrorMessage(error));
        }
      })();
    },
    [
      applyThreadStateAndSnapshot,
      hydrateCurrentThread,
      onError,
      resetActiveResponseState,
    ],
  );

  const createThread = useCallback(
    (displayName?: string): void => {
      void (async () => {
        try {
          const response = await axiosInstance.post<CreateThreadResponse>(
            "/api/threads",
            {
              ...(displayName ? { display_name: displayName } : {}),
            },
          );
          applyThreadStateAndSnapshot(
            response.data.sessions,
            response.data.session.id,
          );
          hydrateCurrentThread(response.data.session.id, []);
        } catch (error) {
          onError(getApiErrorMessage(error));
        }
      })();
    },
    [applyThreadStateAndSnapshot, hydrateCurrentThread, onError],
  );

  const renameThread = useCallback(
    (sessionId: string, displayName: string): void => {
      if (!sessionId || !displayName.trim()) {
        return;
      }
      void (async () => {
        try {
          const response = await axiosInstance.patch<RenameThreadResponse>(
            `/api/threads/${encodeURIComponent(sessionId)}`,
            {
              display_name: displayName,
            },
          );
          applyThreadListKeepingSelection(response.data.sessions);
        } catch (error) {
          onError(getApiErrorMessage(error));
        }
      })();
    },
    [applyThreadListKeepingSelection, onError],
  );

  const clearCurrentThread = useCallback((): void => {
    const sessionId = stateRef.current.currentSessionId;
    if (!sessionId) {
      return;
    }
    resetActiveResponseState();
    void (async () => {
      try {
        const [threadsRes, clearedRes] = await Promise.all([
          axiosInstance.get<ThreadsResponse>("/api/threads"),
          axiosInstance.delete<ThreadMessagesResponse>(
            `/api/threads/${encodeURIComponent(sessionId)}/messages`,
          ),
        ]);
        applyThreadStateAndSnapshot(threadsRes.data.sessions, sessionId);
        hydrateCurrentThread(sessionId, clearedRes.data.history ?? []);
      } catch (error) {
        onError(getApiErrorMessage(error));
      }
    })();
  }, [
    applyThreadStateAndSnapshot,
    hydrateCurrentThread,
    onError,
    resetActiveResponseState,
  ]);

  const deleteThread = useCallback(
    (sessionId: string): void => {
      if (!sessionId) {
        return;
      }
      resetActiveResponseState();
      void (async () => {
        try {
          const snapshot = stateRef.current;
          const response = await axiosInstance.delete<DeleteThreadResponse>(
            `/api/threads/${encodeURIComponent(sessionId)}`,
          );
          const sessions = response.data.sessions ?? [];
          const wasCurrent = snapshot.currentSessionId === sessionId;
          let nextSessionId: string | null = snapshot.currentSessionId;
          if (!nextSessionId || wasCurrent) {
            nextSessionId = response.data.fallback_session_id ?? null;
          }
          if (
            nextSessionId &&
            !sessions.some((thread) => thread.id === nextSessionId)
          ) {
            nextSessionId = sessions[0]?.id ?? null;
          }

          applyThreadStateAndSnapshot(sessions, nextSessionId ?? undefined);
          if (nextSessionId) {
            const historyRes = await axiosInstance.get<ThreadMessagesResponse>(
              `/api/threads/${encodeURIComponent(nextSessionId)}/messages`,
            );
            hydrateCurrentThread(nextSessionId, historyRes.data.history ?? []);
            return;
          }
          hydrateCurrentThread(null, []);
        } catch (error) {
          onError(getApiErrorMessage(error));
        }
      })();
    },
    [
      applyThreadStateAndSnapshot,
      hydrateCurrentThread,
      onError,
      resetActiveResponseState,
    ],
  );

  const refreshPermissions = useCallback((): void => {
    void (async () => {
      try {
        const response = await axiosInstance.get<PermissionsResponse>(
          "/api/settings/permissions",
        );
        applyPermissionsState(response.data.permissions, []);
      } catch (error) {
        onError(getApiErrorMessage(error));
      }
    })();
  }, [applyPermissionsState, onError]);

  const refreshSkills = useCallback((): void => {
    void (async () => {
      try {
        const response = await axiosInstance.get<SkillsResponse>(
          "/api/settings/skills",
        );
        applySkillsStatus(response.data.skills, []);
      } catch (error) {
        onError(getApiErrorMessage(error));
      }
    })();
  }, [applySkillsStatus, onError]);

  const loadSkillContent = useCallback(
    (path: string): void => {
      const trimmed = path.trim();
      if (!trimmed) {
        return;
      }

      setState((prev) => ({
        ...prev,
        skillContentLoadingPath: trimmed,
      }));
      void (async () => {
        try {
          const response = await axiosInstance.get<SkillContentResponse>(
            "/api/settings/skills/content",
            {
              params: { path: trimmed },
            },
          );
          onSkillContent(response.data.path, response.data.content);
        } catch (error) {
          setState((prev) => ({ ...prev, skillContentLoadingPath: null }));
          onError(getApiErrorMessage(error));
        }
      })();
    },
    [onError, onSkillContent],
  );

  const saveSkill = useCallback(
    (path: string, content: string): void => {
      const trimmedPath = path.trim();
      if (!trimmedPath) {
        return;
      }
      if (stateRef.current.skillsSaving) {
        return;
      }

      setState((prev) => ({
        ...prev,
        skillsSaving: true,
        skillsErrors: [],
      }));
      void (async () => {
        try {
          const response = await axiosInstance.put<UpdateSkillContentResponse>(
            "/api/settings/skills/content",
            {
              path: trimmedPath,
              content,
            },
          );
          applySkillsStatus(response.data.skills, []);
          onSkillContent(response.data.path, response.data.content);
        } catch (error) {
          setState((prev) => ({
            ...prev,
            skillsSaving: false,
            skillsErrors: [getApiErrorMessage(error)],
          }));
        }
      })();
    },
    [applySkillsStatus, onSkillContent],
  );

  const refreshThreads = useCallback((): void => {
    void (async () => {
      try {
        const response =
          await axiosInstance.get<ThreadsResponse>("/api/threads");
        applyThreadListKeepingSelection(response.data.sessions ?? []);
      } catch (error) {
        onError(getApiErrorMessage(error));
      }
    })();
  }, [applyThreadListKeepingSelection, onError]);

  const savePermissions = useCallback((): void => {
    const snapshot = stateRef.current;
    if (snapshot.permissionsSaving) {
      return;
    }

    setState((prev) => ({
      ...prev,
      permissionsSaving: true,
      permissionsErrors: [],
    }));

    void (async () => {
      try {
        const response = await axiosInstance.put<PermissionsResponse>(
          "/api/settings/permissions",
          {
            enabled: snapshot.permissionsEnabled,
            allow: parseRulesInput(snapshot.permissionsAllowText),
            deny: parseRulesInput(snapshot.permissionsDenyText),
          },
        );
        applyPermissionsState(response.data.permissions, []);
        setState((prev) => ({ ...prev, permissionsSavedAt: Date.now() }));
      } catch (error) {
        setState((prev) => ({
          ...prev,
          permissionsSaving: false,
          permissionsErrors: [getApiErrorMessage(error)],
        }));
      }
    })();
  }, [applyPermissionsState]);

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
          return true;
        }
        if (mode === "expand") {
          setToolOutputsExpanded(true);
          return true;
        }
        if (mode === "hide") {
          setShowToolCalls(false);
          return true;
        }
        if (mode === "show") {
          setShowToolCalls(true);
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
      const sessionId = stateRef.current.currentSessionId;
      if (!text) {
        return;
      }

      if (runSlashCommand(text)) {
        resetActiveResponseState();
        return;
      }

      if (!sessionId || !ws || ws.readyState !== WebSocket.OPEN) {
        return;
      }

      ws.send(
        JSON.stringify({
          type: "message",
          content: text,
          session_id: sessionId,
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
      cancelQueuedInput,
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
      refreshSkills,
      loadSkillContent,
      saveSkill,
      refreshThreads,
    }),
    [
      clearCurrentThread,
      createThread,
      deleteThread,
      loadSkillContent,
      cancelQueuedInput,
      refreshPermissions,
      refreshSkills,
      refreshThreads,
      renameThread,
      requestKillSwitch,
      runSlashCommand,
      savePermissions,
      saveSkill,
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
