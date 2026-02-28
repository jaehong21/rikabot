<script lang="ts">
  import { onDestroy, onMount, tick } from 'svelte';

  type ConnectionState = 'connecting' | 'connected' | 'disconnected';
  type MainView = 'chat' | 'settings';
  type SettingsSection = 'permissions';

  type ThreadRecord = {
    id: string;
    display_name: string;
    created_at: string;
    updated_at: string;
    message_count: number;
  };

  type HistoryMessage = {
    role: string;
    content: string;
  };

  type MessageEntry = {
    id: string;
    kind: 'message';
    role: 'user' | 'assistant';
    text: string;
    error?: boolean;
    stats?: ResponseStats;
  };

  type ToolEntry = {
    id: string;
    kind: 'tool';
    name: string;
    args: string;
    argsPreview: string;
    output: string;
    status: 'running' | 'success' | 'failure';
    resolved: boolean;
    open: boolean;
  };

  type Entry = MessageEntry | ToolEntry;

  type TokenUsage = {
    prompt_tokens: number;
    completion_tokens: number;
    total_tokens: number;
  };

  type ResponseStats = {
    elapsedMs: number;
    toolCalls: number;
    toolSuccess: number;
    toolFailed: number;
    usage?: TokenUsage;
  };

  type DoneEvent = {
    type: 'done';
    full_response?: string;
    elapsed_ms?: number;
    tool_call_count?: number;
    tool_call_success?: number;
    tool_call_failed?: number;
    usage?: Partial<TokenUsage>;
  };

  type StoppedEvent = {
    type: 'stopped';
    reason?: string;
    session_id?: string;
  };

  type PermissionsState = {
    enabled?: boolean;
    tools?: {
      allow?: string[];
      deny?: string[];
    };
  };

  type McpServerState = 'pending' | 'connecting' | 'ready' | 'failed' | 'disabled';

  type McpServerStatus = {
    name: string;
    state: McpServerState;
    tool_count?: number;
    error?: string | null;
  };

  type McpStatusSnapshot = {
    enabled?: boolean;
    servers?: McpServerStatus[];
  };

  type ServerEvent =
    | { type: 'user_message'; content?: string }
    | { type: 'chunk'; content?: string }
    | { type: 'tool_call_start'; name?: string; args?: unknown }
    | { type: 'tool_call_result'; name?: string; output?: string; success?: boolean }
    | DoneEvent
    | StoppedEvent
    | { type: 'error'; message?: string }
    | {
        type: 'thread_list';
        current_session_id?: string;
        sessions?: ThreadRecord[];
      }
    | {
        type: 'thread_created';
        current_session_id?: string;
        sessions?: ThreadRecord[];
        history?: HistoryMessage[];
      }
    | {
        type: 'thread_renamed';
        current_session_id?: string;
        sessions?: ThreadRecord[];
      }
    | {
        type: 'thread_switched';
        session_id?: string;
        current_session_id?: string;
        sessions?: ThreadRecord[];
        history?: HistoryMessage[];
      }
    | {
        type: 'thread_cleared';
        session_id?: string;
        current_session_id?: string;
        sessions?: ThreadRecord[];
        history?: HistoryMessage[];
      }
    | {
        type: 'thread_deleted';
        deleted_session_id?: string;
        current_session_id?: string;
        sessions?: ThreadRecord[];
        history?: HistoryMessage[];
      }
    | {
        type: 'permissions_state';
        permissions?: PermissionsState;
        validation_errors?: string[];
      }
    | {
        type: 'permissions_updated';
        permissions?: PermissionsState;
      }
    | {
        type: 'mcp_status';
        mcp?: McpStatusSnapshot;
      };

  type SlashArgOption = {
    value: string;
    description: string;
  };

  type SlashCommandDef = {
    name: string;
    description: string;
    argsHint?: string;
    argOptions?: SlashArgOption[];
  };

  type SlashSuggestion = {
    kind: 'command' | 'arg';
    label: string;
    completion: string;
    description: string;
  };

  type CommandBarItem =
    | {
        kind: 'view';
        id: string;
        title: string;
        meta: string;
        view: MainView;
        section?: SettingsSection;
      }
    | {
        kind: 'session';
        id: string;
        title: string;
        meta: string;
        session: ThreadRecord;
      };

  let entries: Entry[] = [];
  let threads: ThreadRecord[] = [];
  let currentSessionId: string | null = null;
  let activeView: MainView = 'chat';
  let activeSettingsSection: SettingsSection = 'permissions';

  let inputValue = '';
  let isWaiting = false;
  let killRequested = false;
  let connectionState: ConnectionState = 'connecting';
  let showReconnectOverlay = false;
  let toolOutputsExpanded = true;
  let showToolCalls = true;

  let ws: WebSocket | null = null;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let reconnectDelay = 1000;
  let currentAssistantId: string | null = null;
  let disposed = false;
  let idCounter = 0;
  let activeResponseStartedAt: number | null = null;
  let activeToolCallCount = 0;
  let activeToolSuccessCount = 0;
  let activeToolFailureCount = 0;

  let messagesEl: HTMLElement | null = null;
  let inputEl: HTMLTextAreaElement | null = null;
  let commandBarInputEl: HTMLInputElement | null = null;

  const TOOL_PREFS_STORAGE_KEY = 'rika.toolOutputsExpanded';
  const TOOL_VISIBILITY_STORAGE_KEY = 'rika.showToolCalls';
  const SLASH_COMMANDS: SlashCommandDef[] = [
    { name: 'help', description: 'Show the available slash commands.' },
    { name: 'new', description: 'Create a new session.', argsHint: '[name]' },
    { name: 'rename', description: 'Rename the current session.', argsHint: '<name>' },
    { name: 'clear', description: 'Clear the current session.' },
    { name: 'delete', description: 'Delete the current session.' },
    { name: 'stop', description: 'Stop the active run.' },
    {
      name: 'tools',
      description: 'Tool panel visibility controls.',
      argsHint: '<collapse|expand|hide|show>',
      argOptions: [
        { value: 'collapse', description: 'Collapse every tool output block.' },
        { value: 'expand', description: 'Expand every tool output block.' },
        { value: 'hide', description: 'Hide tool call blocks from chat.' },
        { value: 'show', description: 'Show tool call blocks in chat.' }
      ]
    }
  ];
  const SLASH_COMMAND_MAP = new Map(SLASH_COMMANDS.map((cmd) => [cmd.name, cmd]));

  let commandBarOpen = false;
  let commandBarQuery = '';
  let commandBarSelectionIndex = 0;
  let commandBarItems: CommandBarItem[] = [];

  let permissionsLoaded = false;
  let permissionsSaving = false;
  let permissionsEnabled = true;
  let permissionsAllowText = '';
  let permissionsDenyText = '';
  let permissionsErrors: string[] = [];
  let permissionsSavedAt: number | null = null;
  let mcpEnabled = true;
  let mcpServers: McpServerStatus[] = [];

  let slashSuggestions: SlashSuggestion[] = [];
  let slashSelectionIndex = 0;
  let slashSuggestionSignature = '';

  $: canSend =
    !isWaiting &&
    connectionState === 'connected' &&
    inputValue.trim().length > 0;

  $: canKill = isWaiting && connectionState === 'connected' && !killRequested;

  $: statusLabel =
    connectionState === 'connected'
      ? 'Connected'
      : connectionState === 'connecting'
        ? 'Connecting'
        : 'Disconnected';

  $: mcpReadyCount = mcpServers.filter((server) => server.state === 'ready').length;
  $: mcpConnectingCount = mcpServers.filter((server) => server.state === 'connecting').length;
  $: mcpFailedCount = mcpServers.filter((server) => server.state === 'failed').length;

  $: {
    const query = commandBarQuery.trim().toLowerCase();
    const views: CommandBarItem[] = [
      {
        kind: 'view',
        id: 'view-chat',
        title: 'Open chat',
        meta: 'Main conversation view',
        view: 'chat'
      },
      {
        kind: 'view',
        id: 'view-settings-permissions',
        title: 'Open settings · Permissions',
        meta: 'Guardrail rules and tool permissions',
        view: 'settings',
        section: 'permissions'
      }
    ];

    const sessions: CommandBarItem[] = threads.map((thread) => ({
      kind: 'session',
      id: `session-${thread.id}`,
      title: thread.display_name,
      meta: `${thread.message_count} msgs`,
      session: thread
    }));

    const merged = [...views, ...sessions];
    commandBarItems = merged.filter((item) => {
      if (!query) {
        return true;
      }
      const haystack = `${item.title} ${item.meta}`.toLowerCase();
      return haystack.includes(query);
    });

    if (commandBarSelectionIndex >= commandBarItems.length) {
      commandBarSelectionIndex = Math.max(commandBarItems.length - 1, 0);
    }
  }

  $: {
    const nextSuggestions = buildSlashSuggestions(inputValue);
    const signature = nextSuggestions.map((item) => item.completion).join('|');
    if (signature !== slashSuggestionSignature) {
      slashSelectionIndex = 0;
      slashSuggestionSignature = signature;
    }
    slashSuggestions = nextSuggestions;
  }

  onMount(() => {
    loadUiPreferences();
    connect();
    autoResize();
    window.addEventListener('keydown', onWindowKeydown);
  });

  onDestroy(() => {
    disposed = true;
    if (reconnectTimer) {
      clearTimeout(reconnectTimer);
      reconnectTimer = null;
    }
    window.removeEventListener('keydown', onWindowKeydown);
    ws?.close();
  });

  function nextId(): string {
    idCounter += 1;
    return `${Date.now()}-${idCounter}`;
  }

  function loadUiPreferences(): void {
    toolOutputsExpanded = readBooleanPreference(TOOL_PREFS_STORAGE_KEY, true);
    showToolCalls = readBooleanPreference(TOOL_VISIBILITY_STORAGE_KEY, true);
  }

  function readBooleanPreference(key: string, fallback: boolean): boolean {
    try {
      const raw = window.localStorage.getItem(key);
      if (raw === 'true') return true;
      if (raw === 'false') return false;
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

  function setToolOutputsExpanded(value: boolean): void {
    toolOutputsExpanded = value;
    writeBooleanPreference(TOOL_PREFS_STORAGE_KEY, value);
    entries = entries.map((entry) => {
      if (entry.kind !== 'tool') {
        return entry;
      }
      return { ...entry, open: value };
    });
  }

  function setToolCallsVisible(value: boolean): void {
    showToolCalls = value;
    writeBooleanPreference(TOOL_VISIBILITY_STORAGE_KEY, value);
  }

  function isCommandBarShortcut(event: KeyboardEvent): boolean {
    return (
      !event.shiftKey &&
      !event.altKey &&
      (event.metaKey || event.ctrlKey) &&
      event.key.toLowerCase() === 'k'
    );
  }

  function openCommandBar(): void {
    commandBarOpen = true;
    commandBarQuery = '';
    const sessionEntryId = currentSessionId ? `session-${currentSessionId}` : null;
    const currentIndex = sessionEntryId
      ? commandBarItems.findIndex((item) => item.kind === 'session' && item.id === sessionEntryId)
      : -1;
    commandBarSelectionIndex = currentIndex >= 0 ? currentIndex : 0;
    tick().then(() => commandBarInputEl?.focus());
  }

  function closeCommandBar(): void {
    commandBarOpen = false;
    commandBarQuery = '';
    tick().then(() => inputEl?.focus());
  }

  function moveCommandBarSelection(step: number): void {
    if (!commandBarItems.length) {
      return;
    }

    commandBarSelectionIndex =
      (commandBarSelectionIndex + step + commandBarItems.length) % commandBarItems.length;
  }

  function chooseCommandBarItem(item: CommandBarItem): void {
    closeCommandBar();

    if (item.kind === 'session') {
      openChatView();
      switchThread(item.session.id);
      return;
    }

    if (item.view === 'settings') {
      openSettingsSection(item.section ?? 'permissions');
      return;
    }

    openChatView();
  }

  function onWindowKeydown(event: KeyboardEvent): void {
    if (event.defaultPrevented) {
      return;
    }

    if (isCommandBarShortcut(event)) {
      event.preventDefault();
      if (commandBarOpen) {
        closeCommandBar();
      } else {
        openCommandBar();
      }
      return;
    }

    if (commandBarOpen && event.key === 'Escape') {
      event.preventDefault();
      closeCommandBar();
    }
  }

  function onCommandBarKeydown(event: KeyboardEvent): void {
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      moveCommandBarSelection(1);
      return;
    }

    if (event.key === 'ArrowUp') {
      event.preventDefault();
      moveCommandBarSelection(-1);
      return;
    }

    if (event.key === 'Enter') {
      event.preventDefault();
      const selected = commandBarItems[commandBarSelectionIndex];
      if (selected) {
        chooseCommandBarItem(selected);
      }
      return;
    }

    if (event.key === 'Escape') {
      event.preventDefault();
      closeCommandBar();
    }
  }

  function connect(): void {
    if (disposed) return;

    connectionState = 'connecting';

    const proto = window.location.protocol === 'https:' ? 'wss' : 'ws';
    const socket = new WebSocket(`${proto}://${window.location.host}/ws`);
    ws = socket;

    socket.onopen = () => {
      connectionState = 'connected';
      showReconnectOverlay = false;
      reconnectDelay = 1000;

      if (reconnectTimer) {
        clearTimeout(reconnectTimer);
        reconnectTimer = null;
      }

      sendControl({ type: 'thread_list' });
      sendControl({ type: 'permissions_get' });
    };

    socket.onclose = () => {
      if (disposed) return;
      connectionState = 'disconnected';
      scheduleReconnect();
    };

    socket.onerror = () => {
      socket.close();
    };

    socket.onmessage = (event: MessageEvent<string>) => {
      try {
        const data = JSON.parse(event.data) as ServerEvent;
        handleEvent(data);
      } catch {
        // Ignore malformed server events.
      }
    };
  }

  function scheduleReconnect(): void {
    if (disposed || reconnectTimer) return;

    showReconnectOverlay = true;
    reconnectTimer = setTimeout(() => {
      reconnectTimer = null;
      connect();
      reconnectDelay = Math.min(Math.round(reconnectDelay * 1.5), 10000);
    }, reconnectDelay);
  }

  function handleEvent(event: ServerEvent): void {
    switch (event.type) {
      case 'user_message':
        onUserMessage(event.content ?? '');
        return;
      case 'chunk':
        onChunk(event.content ?? '');
        return;
      case 'tool_call_start':
        onToolCallStart(event.name ?? '', event.args ?? '');
        return;
      case 'tool_call_result':
        onToolCallResult(event.name ?? '', event.output ?? '', Boolean(event.success));
        return;
      case 'done':
        onDone(event);
        return;
      case 'stopped':
        onStopped(event);
        return;
      case 'thread_list':
        applyThreadState(event.sessions, event.current_session_id);
        return;
      case 'thread_created':
      case 'thread_switched':
      case 'thread_cleared':
      case 'thread_deleted': {
        const fallbackSessionId = 'session_id' in event ? event.session_id : undefined;
        applyThreadState(event.sessions, event.current_session_id ?? fallbackSessionId);
        hydrateCurrentThread(event.history ?? []);
        return;
      }
      case 'thread_renamed':
        applyThreadState(event.sessions, event.current_session_id);
        return;
      case 'permissions_state':
        applyPermissionsState(event.permissions, event.validation_errors ?? []);
        return;
      case 'permissions_updated':
        applyPermissionsState(event.permissions, []);
        permissionsSavedAt = Date.now();
        return;
      case 'mcp_status':
        applyMcpStatus(event.mcp);
        return;
      case 'error':
        onError(event.message ?? 'Unknown error');
    }
  }

  function openChatView(): void {
    activeView = 'chat';
  }

  function openSettingsSection(section: SettingsSection): void {
    activeView = 'settings';
    activeSettingsSection = section;
  }

  function applyThreadState(nextThreads?: ThreadRecord[], currentId?: string): void {
    if (Array.isArray(nextThreads)) {
      threads = [...nextThreads];
    }
    if (currentId) {
      currentSessionId = currentId;
    }
  }

  function applyPermissionsState(permissions?: PermissionsState, validationErrors: string[] = []): void {
    const enabled = permissions?.enabled;
    const allow = permissions?.tools?.allow;
    const deny = permissions?.tools?.deny;

    if (typeof enabled === 'boolean') {
      permissionsEnabled = enabled;
    }
    if (Array.isArray(allow)) {
      permissionsAllowText = allow.join('\n');
    }
    if (Array.isArray(deny)) {
      permissionsDenyText = deny.join('\n');
    }

    permissionsErrors = validationErrors;
    permissionsSaving = false;
    permissionsLoaded = true;
  }

  function applyMcpStatus(status?: McpStatusSnapshot): void {
    if (typeof status?.enabled === 'boolean') {
      mcpEnabled = status.enabled;
    }
    if (Array.isArray(status?.servers)) {
      mcpServers = [...status.servers];
    }
  }

  function hydrateCurrentThread(history: HistoryMessage[]): void {
    entries = hydrateEntriesFromHistory(history);
    resetActiveResponseState();
    finishCurrentBubble();
    setWaiting(false);
    scrollToBottom();
  }

  function hydrateEntriesFromHistory(history: HistoryMessage[]): Entry[] {
    const rebuilt: Entry[] = [];
    const toolCallIndex = new Map<string, number>();

    for (const msg of history) {
      if (msg.role === 'user') {
        rebuilt.push({
          id: nextId(),
          kind: 'message',
          role: 'user',
          text: msg.content
        });
        continue;
      }

      if (msg.role === 'assistant') {
        let parsed: { content?: unknown; tool_calls?: unknown } | null = null;
        try {
          const raw = JSON.parse(msg.content) as { content?: unknown; tool_calls?: unknown };
          if (raw && typeof raw === 'object' && Array.isArray(raw.tool_calls)) {
            parsed = raw;
          }
        } catch {
          // Not structured tool-call JSON; treat as plain assistant text.
        }

        if (!parsed) {
          rebuilt.push({
            id: nextId(),
            kind: 'message',
            role: 'assistant',
            text: msg.content
          });
          continue;
        }

        if (typeof parsed.content === 'string' && parsed.content.trim().length > 0) {
          rebuilt.push({
            id: nextId(),
            kind: 'message',
            role: 'assistant',
            text: parsed.content
          });
        }

        const toolCalls = parsed.tool_calls as Array<Record<string, unknown>>;
        for (const call of toolCalls) {
          const callId = typeof call.id === 'string' && call.id.trim() ? call.id : nextId();
          const name = typeof call.name === 'string' ? call.name : 'unknown_tool';
          const formattedArgs = formatArgs(call.arguments ?? {});

          const entry: ToolEntry = {
            id: nextId(),
            kind: 'tool',
            name,
            args: formattedArgs,
            argsPreview: makePreview(formattedArgs),
            output: '',
            status: 'running',
            resolved: false,
            open: toolOutputsExpanded
          };
          const idx = rebuilt.push(entry) - 1;
          toolCallIndex.set(callId, idx);
        }

        continue;
      }

      if (msg.role === 'tool') {
        let callId = '';
        let output = msg.content;

        try {
          const parsed = JSON.parse(msg.content) as { tool_call_id?: unknown; content?: unknown };
          if (typeof parsed.tool_call_id === 'string') {
            callId = parsed.tool_call_id;
          }
          if (typeof parsed.content === 'string') {
            output = parsed.content;
          }
        } catch {
          // Keep fallback output.
        }

        const idx = callId ? toolCallIndex.get(callId) : undefined;
        if (idx !== undefined) {
          const current = rebuilt[idx];
          if (current && current.kind === 'tool') {
            rebuilt[idx] = {
              ...current,
              output,
              status: 'success',
              resolved: true,
              open: toolOutputsExpanded
            };
          }
        } else {
          rebuilt.push({
            id: nextId(),
            kind: 'message',
            role: 'assistant',
            text: output
          });
        }
      }
    }

    for (let i = 0; i < rebuilt.length; i += 1) {
      const entry = rebuilt[i];
      if (entry.kind === 'tool' && !entry.resolved) {
        rebuilt[i] = {
          ...entry,
          resolved: true,
          status: 'failure',
          output: entry.output || 'No stored result found for this tool call.',
          open: toolOutputsExpanded
        };
      }
    }

    return rebuilt;
  }

  function onChunk(text: string): void {
    if (!isWaiting) {
      setWaiting(true);
    }
    if (!activeResponseStartedAt) {
      activeResponseStartedAt = Date.now();
    }

    if (!currentAssistantId) {
      const entry: MessageEntry = {
        id: nextId(),
        kind: 'message',
        role: 'assistant',
        text: ''
      };
      entries = [...entries, entry];
      currentAssistantId = entry.id;
    }

    entries = entries.map((entry) => {
      if (entry.kind !== 'message' || entry.id !== currentAssistantId) {
        return entry;
      }

      return {
        ...entry,
        text: entry.text + text
      };
    });

    scrollToBottom();
  }

  function onToolCallStart(name: string, args: unknown): void {
    if (!isWaiting) {
      setWaiting(true);
    }
    if (!activeResponseStartedAt) {
      activeResponseStartedAt = Date.now();
    }

    finishCurrentBubble();
    activeToolCallCount += 1;

    const formattedArgs = formatArgs(args);
    const entry: ToolEntry = {
      id: nextId(),
      kind: 'tool',
      name: name || 'unknown_tool',
      args: formattedArgs,
      argsPreview: makePreview(formattedArgs),
      output: '',
      status: 'running',
      resolved: false,
      open: toolOutputsExpanded
    };

    entries = [...entries, entry];
    scrollToBottom();
  }

  function onToolCallResult(name: string, output: string, success: boolean): void {
    if (!isWaiting) {
      setWaiting(true);
    }
    if (!activeResponseStartedAt) {
      activeResponseStartedAt = Date.now();
    }

    if (success) {
      activeToolSuccessCount += 1;
    } else {
      activeToolFailureCount += 1;
    }

    const idx = findLatestUnresolvedToolIndex(name);
    if (idx === -1) {
      scrollToBottom();
      return;
    }

    const updated = [...entries];
    const entry = updated[idx];

    if (entry.kind === 'tool') {
      updated[idx] = {
        ...entry,
        output,
        status: success ? 'success' : 'failure',
        resolved: true,
        open: entry.open || toolOutputsExpanded
      };
      entries = updated;
    }

    scrollToBottom();
  }

  function onDone(event: DoneEvent): void {
    const stats = buildResponseStats(event);
    const fullResponse = event.full_response ?? '';

    if (currentAssistantId) {
      entries = entries.map((entry) => {
        if (entry.kind !== 'message' || entry.id !== currentAssistantId) {
          return entry;
        }

        return {
          ...entry,
          text: fullResponse,
          stats
        };
      });
      finishCurrentBubble();
    } else if (fullResponse) {
      const entry: MessageEntry = {
        id: nextId(),
        kind: 'message',
        role: 'assistant',
        text: fullResponse,
        stats
      };
      entries = [...entries, entry];
    }

    resetActiveResponseState();
    setWaiting(false);
    scrollToBottom();
  }

  function onStopped(event: StoppedEvent): void {
    finishCurrentBubble();
    resolvePendingToolsAsStopped();

    if (event.reason === 'internal_cancel') {
      onError('Run stopped due to an internal cancellation.');
      return;
    }

    resetActiveResponseState();
    setWaiting(false);
    scrollToBottom();
  }

  function onError(message: string): void {
    if (permissionsSaving && message.toLowerCase().includes('permission')) {
      permissionsErrors = [message];
      permissionsSaving = false;
    }

    finishCurrentBubble();

    const entry: MessageEntry = {
      id: nextId(),
      kind: 'message',
      role: 'assistant',
      text: `Error: ${message}`,
      error: true
    };

    entries = [...entries, entry];

    resetActiveResponseState();
    setWaiting(false);
    scrollToBottom();
  }

  function onUserMessage(text: string): void {
    if (!text.trim()) {
      return;
    }

    entries = [
      ...entries,
      {
        id: nextId(),
        kind: 'message',
        role: 'user',
        text
      }
    ];

    activeResponseStartedAt = Date.now();
    activeToolCallCount = 0;
    activeToolSuccessCount = 0;
    activeToolFailureCount = 0;
    setWaiting(true);
    finishCurrentBubble();
    scrollToBottom();
  }

  function findLatestUnresolvedToolIndex(name: string): number {
    for (let i = entries.length - 1; i >= 0; i -= 1) {
      const entry = entries[i];
      if (entry.kind === 'tool' && !entry.resolved && entry.name === name) {
        return i;
      }
    }

    return -1;
  }

  function finishCurrentBubble(): void {
    currentAssistantId = null;
  }

  function setWaiting(value: boolean): void {
    isWaiting = value;
    if (!value) {
      killRequested = false;
      tick().then(() => inputEl?.focus());
    }
  }

  function resolvePendingToolsAsStopped(): void {
    entries = entries.map((entry) => {
      if (entry.kind !== 'tool' || entry.resolved) {
        return entry;
      }

      return {
        ...entry,
        resolved: true,
        status: 'failure',
        output: entry.output || 'Stopped by user.',
        open: entry.open || toolOutputsExpanded
      };
    });
  }

  function resetActiveResponseState(): void {
    activeResponseStartedAt = null;
    activeToolCallCount = 0;
    activeToolSuccessCount = 0;
    activeToolFailureCount = 0;
  }

  function buildResponseStats(event: DoneEvent): ResponseStats {
    const elapsedMs =
      typeof event.elapsed_ms === 'number' && event.elapsed_ms >= 0
        ? event.elapsed_ms
        : activeResponseStartedAt
          ? Date.now() - activeResponseStartedAt
          : 0;

    const usage = normalizeUsage(event.usage);
    const toolCalls =
      typeof event.tool_call_count === 'number' ? event.tool_call_count : activeToolCallCount;
    const toolSuccess =
      typeof event.tool_call_success === 'number'
        ? event.tool_call_success
        : activeToolSuccessCount;
    const toolFailed =
      typeof event.tool_call_failed === 'number' ? event.tool_call_failed : activeToolFailureCount;

    return {
      elapsedMs,
      toolCalls,
      toolSuccess,
      toolFailed,
      usage
    };
  }

  function normalizeUsage(usage?: Partial<TokenUsage>): TokenUsage | undefined {
    if (!usage) {
      return undefined;
    }

    const promptTokens = Math.max(0, Math.round(Number(usage.prompt_tokens ?? 0)));
    const completionTokens = Math.max(0, Math.round(Number(usage.completion_tokens ?? 0)));
    const providedTotal = Math.max(0, Math.round(Number(usage.total_tokens ?? 0)));
    const totalTokens = providedTotal > 0 ? providedTotal : promptTokens + completionTokens;

    if (promptTokens === 0 && completionTokens === 0 && totalTokens === 0) {
      return undefined;
    }

    return {
      prompt_tokens: promptTokens,
      completion_tokens: completionTokens,
      total_tokens: totalTokens
    };
  }

  function formatElapsed(ms: number): string {
    const seconds = ms / 1000;
    if (seconds >= 10) {
      return `${seconds.toFixed(0)}s`;
    }
    return `${seconds.toFixed(1)}s`;
  }

  function formatTokenUsage(stats: ResponseStats): string {
    if (!stats.usage) {
      return 'tokens n/a';
    }

    return `tokens ${stats.usage.total_tokens} (in ${stats.usage.prompt_tokens} / out ${stats.usage.completion_tokens})`;
  }

  function formatArgs(args: unknown): string {
    if (typeof args === 'object' && args !== null) {
      return JSON.stringify(args, null, 2);
    }

    if (typeof args === 'string') {
      try {
        return JSON.stringify(JSON.parse(args), null, 2);
      } catch {
        return args;
      }
    }

    return String(args);
  }

  function makePreview(content: string): string {
    const compact = content.replace(/\s+/g, ' ').trim();
    if (!compact) {
      return '(no arguments)';
    }

    if (compact.length <= 120) {
      return compact;
    }

    return `${compact.slice(0, 120)}...`;
  }

  function statusText(status: ToolEntry['status']): string {
    if (status === 'running') return 'Running';
    if (status === 'success') return 'Success';
    return 'Failed';
  }

  function mcpStateLabel(state: McpServerState): string {
    if (state === 'ready') return 'Ready';
    if (state === 'connecting') return 'Connecting';
    if (state === 'failed') return 'Failed';
    if (state === 'disabled') return 'Disabled';
    return 'Pending';
  }

  function toggleTool(entryId: string): void {
    entries = entries.map((entry) => {
      if (entry.kind !== 'tool' || entry.id !== entryId) {
        return entry;
      }
      return {
        ...entry,
        open: !entry.open
      };
    });
  }

  function sendControl(payload: Record<string, unknown>): void {
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      return;
    }
    ws.send(JSON.stringify(payload));
  }

  function requestKillSwitch(): void {
    if (connectionState !== 'connected' || !isWaiting || killRequested) {
      return;
    }

    killRequested = true;
    sendControl({ type: 'kill_switch' });
  }

  function switchThread(sessionId: string): void {
    if (isWaiting || sessionId === currentSessionId) {
      return;
    }
    setWaiting(false);
    resetActiveResponseState();
    sendControl({ type: 'thread_switch', session_id: sessionId });
  }

  function parseRulesInput(raw: string): string[] {
    return raw
      .split('\n')
      .map((line) => line.trim())
      .filter((line) => line.length > 0);
  }

  function savePermissions(): void {
    if (connectionState !== 'connected' || permissionsSaving) {
      return;
    }

    permissionsSaving = true;
    permissionsErrors = [];
    sendControl({
      type: 'permissions_set',
      enabled: permissionsEnabled,
      allow: parseRulesInput(permissionsAllowText),
      deny: parseRulesInput(permissionsDenyText)
    });
  }

  function pushAssistantNote(text: string): void {
    entries = [
      ...entries,
      {
        id: nextId(),
        kind: 'message',
        role: 'assistant',
        text
      }
    ];
    scrollToBottom();
  }

  function commandCompletion(command: SlashCommandDef): string {
    return command.argsHint ? `/${command.name} ` : `/${command.name}`;
  }

  function buildSlashSuggestions(rawInput: string): SlashSuggestion[] {
    if (!rawInput.startsWith('/')) {
      return [];
    }

    const body = rawInput.slice(1);
    const firstSpace = body.indexOf(' ');

    if (firstSpace === -1) {
      const query = body.trim().toLowerCase();
      return SLASH_COMMANDS.filter((command) => command.name.startsWith(query)).map((command) => ({
        kind: 'command',
        label: `/${command.name}${command.argsHint ? ` ${command.argsHint}` : ''}`,
        completion: commandCompletion(command),
        description: command.description
      }));
    }

    const commandName = body.slice(0, firstSpace).trim().toLowerCase();
    const command = SLASH_COMMAND_MAP.get(commandName);
    if (!command || !command.argOptions || command.argOptions.length === 0) {
      return [];
    }

    const argInput = body.slice(firstSpace + 1).trim().toLowerCase();
    return command.argOptions
      .filter((option) => !argInput || option.value.startsWith(argInput))
      .map((option) => ({
        kind: 'arg',
        label: `/${command.name} ${option.value}`,
        completion: `/${command.name} ${option.value}`,
        description: option.description
      }));
  }

  function moveSlashSelection(step: number): void {
    if (!slashSuggestions.length) {
      return;
    }
    slashSelectionIndex = (slashSelectionIndex + step + slashSuggestions.length) % slashSuggestions.length;
  }

  function applySlashSuggestionAt(index: number): void {
    const selected = slashSuggestions[index];
    if (!selected) {
      return;
    }

    inputValue = selected.completion;
    autoResize();
    tick().then(() => {
      if (!inputEl) return;
      inputEl.focus();
      const pos = inputValue.length;
      inputEl.setSelectionRange(pos, pos);
    });
  }

  function handleSlashCommand(raw: string): boolean {
    if (!raw.startsWith('/')) {
      return false;
    }

    const body = raw.slice(1).trim();
    if (!body) {
      onError('Empty slash command');
      return true;
    }

    const firstSpace = body.indexOf(' ');
    const cmd = (firstSpace === -1 ? body : body.slice(0, firstSpace)).toLowerCase();
    const arg = (firstSpace === -1 ? '' : body.slice(firstSpace + 1)).trim();

    if (cmd === 'help') {
      pushAssistantNote(
        'Session commands: `/new [name]`, `/rename <name>`, `/clear`, `/delete`, `/stop`\nTool view: `/tools <collapse|expand|hide|show>`\nNavigate chat/settings with `Cmd/Ctrl+K`'
      );
      return true;
    }

    if (cmd === 'stop') {
      requestKillSwitch();
      return true;
    }

    if (cmd === 'tools') {
      const mode = arg.toLowerCase();
      if (mode === 'collapse') {
        setToolOutputsExpanded(false);
        pushAssistantNote('Collapsed all tool outputs.');
        return true;
      }
      if (mode === 'expand') {
        setToolOutputsExpanded(true);
        pushAssistantNote('Expanded all tool outputs.');
        return true;
      }
      if (mode === 'hide') {
        setToolCallsVisible(false);
        pushAssistantNote('Tool call blocks are now hidden.');
        return true;
      }
      if (mode === 'show') {
        setToolCallsVisible(true);
        pushAssistantNote('Tool call blocks are now visible.');
        return true;
      }
      onError('Usage: /tools <collapse|expand|hide|show>');
      return true;
    }

    if (cmd === 'new') {
      sendControl({
        type: 'thread_create',
        ...(arg ? { display_name: arg } : {})
      });
      return true;
    }

    if (cmd === 'rename') {
      if (!currentSessionId) {
        onError('No active session to rename');
        return true;
      }
      if (!arg) {
        onError('Usage: /rename <display name>');
        return true;
      }
      sendControl({
        type: 'thread_rename',
        session_id: currentSessionId,
        display_name: arg
      });
      return true;
    }

    if (cmd === 'clear') {
      if (!currentSessionId) {
        onError('No active session to clear');
        return true;
      }
      if (!window.confirm('Clear this thread context?')) {
        return true;
      }
      setWaiting(false);
      resetActiveResponseState();
      sendControl({ type: 'thread_clear' });
      return true;
    }

    if (cmd === 'delete') {
      if (!currentSessionId) {
        onError('No active session to delete');
        return true;
      }
      if (!window.confirm('Delete this thread?')) {
        return true;
      }
      setWaiting(false);
      resetActiveResponseState();
      sendControl({ type: 'thread_delete', session_id: currentSessionId });
      return true;
    }

    onError(`Unknown command: /${cmd}`);
    return true;
  }

  function sendMessage(): void {
    const text = inputValue.trim();
    if (!text || isWaiting || !ws || ws.readyState !== WebSocket.OPEN) {
      return;
    }

    inputValue = '';
    autoResize();

    if (handleSlashCommand(text)) {
      resetActiveResponseState();
      return;
    }

    ws.send(
      JSON.stringify({
        type: 'message',
        content: text
      })
    );
  }

  function onComposerKeydown(event: KeyboardEvent): void {
    if (slashSuggestions.length > 0 && event.key === 'ArrowDown') {
      event.preventDefault();
      moveSlashSelection(1);
      return;
    }

    if (slashSuggestions.length > 0 && event.key === 'ArrowUp') {
      event.preventDefault();
      moveSlashSelection(-1);
      return;
    }

    if (slashSuggestions.length > 0 && event.key === 'Tab') {
      event.preventDefault();
      applySlashSuggestionAt(slashSelectionIndex);
      return;
    }

    if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault();
      sendMessage();
    }
  }

  function autoResize(): void {
    if (!inputEl) return;
    inputEl.style.height = 'auto';
    inputEl.style.height = `${Math.min(inputEl.scrollHeight, 180)}px`;
  }

  function scrollToBottom(): void {
    tick().then(() => {
      if (!messagesEl) return;
      messagesEl.scrollTop = messagesEl.scrollHeight;
    });
  }

  function escapeHtml(raw: string): string {
    return raw
      .replaceAll('&', '&amp;')
      .replaceAll('<', '&lt;')
      .replaceAll('>', '&gt;')
      .replaceAll('"', '&quot;')
      .replaceAll("'", '&#39;');
  }

  function renderMarkdown(text: string): string {
    let html = escapeHtml(text);

    html = html.replace(/```(\w*)\n([\s\S]*?)```/g, (_match, lang, code) => {
      const languageClass = lang ? ` class=\"lang-${lang}\"` : '';
      return `<pre><code${languageClass}>${code}</code></pre>`;
    });

    html = html.replace(/`([^`\n]+)`/g, '<code>$1</code>');
    html = html.replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>');
    html = html.replace(/(?<!\*)\*(?!\*)(.+?)(?<!\*)\*(?!\*)/g, '<em>$1</em>');

    return html;
  }
</script>

<div class="shell">
  <header class="topbar">
    <div>
      <p class="eyebrow">Rika assistant</p>
      <h1>Rika</h1>
    </div>

    <div class="status" data-state={connectionState}>
      <span class="dot"></span>
      <span>{statusLabel}</span>
    </div>
  </header>

  <div class="workspace">
    <aside class="sidebar">
      <div class="sidebar-tabs" role="tablist" aria-label="Main panels">
        <button
          type="button"
          class={`sidebar-tab ${activeView === 'chat' ? 'active' : ''}`}
          on:click={openChatView}
        >
          Chat
        </button>
        <button
          type="button"
          class={`sidebar-tab ${activeView === 'settings' ? 'active' : ''}`}
          on:click={() => openSettingsSection('permissions')}
        >
          Settings
        </button>
      </div>

      <p class="sidebar-label">Sessions</p>
      <div class="thread-list" role="tablist" aria-label="Threads">
        {#each threads as thread (thread.id)}
          <button
            type="button"
            class={`thread-chip ${thread.id === currentSessionId ? 'active' : ''}`}
            on:click={() => switchThread(thread.id)}
            title={thread.display_name}
            disabled={isWaiting}
          >
            <span>{thread.display_name}</span>
          </button>
        {/each}
      </div>
      <div class="slash-hint">
        <p>Slash commands</p>
        <code>/new [name]</code>
        <code>/rename &lt;name&gt;</code>
        <code>/clear</code>
        <code>/delete</code>
        <code>/stop</code>
        <code>/tools &lt;collapse|expand|hide|show&gt;</code>
        <code>Cmd/Ctrl+K navigate</code>
      </div>
      <div class="tool-controls">
        <button
          type="button"
          class={`tool-toggle ${toolOutputsExpanded ? '' : 'active'}`}
          on:click={() => setToolOutputsExpanded(!toolOutputsExpanded)}
        >
          {toolOutputsExpanded ? 'Collapse all tool output' : 'Expand all tool output'}
        </button>
        <button
          type="button"
          class={`tool-toggle ${showToolCalls ? '' : 'active'}`}
          on:click={() => setToolCallsVisible(!showToolCalls)}
        >
          {showToolCalls ? 'Hide tool calls' : 'Show tool calls'}
        </button>
        <button
          type="button"
          class={`tool-toggle ${activeView === 'settings' ? 'active' : ''}`}
          on:click={() => openSettingsSection('permissions')}
        >
          Open permissions settings
        </button>
      </div>

      <section class="mcp-panel">
        <p class="sidebar-label">MCP Servers</p>
        {#if !mcpEnabled}
          <p class="mcp-empty">MCP is disabled in config.</p>
        {:else if mcpServers.length === 0}
          <p class="mcp-empty">No MCP servers configured.</p>
        {:else}
          <p class="mcp-summary">
            Ready {mcpReadyCount}/{mcpServers.length}
            {#if mcpConnectingCount > 0}
              · connecting {mcpConnectingCount}
            {/if}
            {#if mcpFailedCount > 0}
              · failed {mcpFailedCount}
            {/if}
          </p>
          <div class="mcp-list" role="list" aria-label="MCP server statuses">
            {#each mcpServers as server (`mcp-${server.name}`)}
              <article class="mcp-item" role="listitem" data-state={server.state}>
                <div class="mcp-item-head">
                  <span class="mcp-dot"></span>
                  <span class="mcp-name">{server.name}</span>
                  <span class="mcp-state">{mcpStateLabel(server.state)}</span>
                </div>
                {#if server.state === 'ready'}
                  <p class="mcp-meta">Tools {server.tool_count ?? 0}</p>
                {/if}
                {#if server.error && server.state === 'failed'}
                  <p class="mcp-error">{server.error}</p>
                {/if}
              </article>
            {/each}
          </div>
        {/if}
      </section>
    </aside>

    {#if activeView === 'chat'}
      <main class="chat-pane">
      <section class="messages" bind:this={messagesEl}>
        {#if entries.length === 0}
          <article class="welcome">
            <h2>Ready when you are</h2>
            <p>
              Ask about your repo, run tools, and iterate quickly. Rika streams responses and shows
              tool activity inline.
            </p>
          </article>
        {/if}

        {#each entries as entry (entry.id)}
          {#if entry.kind === 'message'}
            <article class={`msg ${entry.role} ${entry.error ? 'error' : ''}`}>
              <div class="content">{@html renderMarkdown(entry.text)}</div>
              {#if entry.role === 'assistant' && entry.stats}
                <p class="msg-meta">
                  {formatElapsed(entry.stats.elapsedMs)} · tools {entry.stats.toolCalls}
                  (success {entry.stats.toolSuccess} / failed {entry.stats.toolFailed}) ·
                  {formatTokenUsage(entry.stats)}
                </p>
              {/if}
            </article>
          {:else if showToolCalls}
            <article class="tool-block" data-status={entry.status}>
              <button class="tool-head" type="button" on:click={() => toggleTool(entry.id)}>
                <div class="tool-head-main">
                  <span class="tool-label">Tool</span>
                  <span class="name">{entry.name}</span>
                </div>

                <div class="tool-head-right">
                  <span class={`result ${entry.status}`}>{statusText(entry.status)}</span>
                  <span class={`caret ${entry.open ? 'open' : ''}`}>▸</span>
                </div>
              </button>

              {#if !entry.open}
                <p class="tool-preview">{entry.argsPreview}</p>
              {/if}

              {#if entry.open}
                <div class="tool-body">
                  <section>
                    <p class="label">Arguments</p>
                    <div class="tool-scroll">
                      <pre>{entry.args}</pre>
                    </div>
                  </section>

                  {#if entry.output}
                    <section>
                      <p class="label">Output</p>
                      <div class="tool-scroll">
                        <pre>{entry.output}</pre>
                      </div>
                    </section>
                  {/if}
                </div>
              {/if}
            </article>
          {/if}
        {/each}

        {#if isWaiting}
          <div class="typing-indicator" aria-live="polite" aria-label="Assistant is working">
            <span></span>
            <span></span>
            <span></span>
          </div>
        {/if}
      </section>

      <footer class="composer-wrap">
        {#if slashSuggestions.length > 0}
          <div class="slash-autocomplete" role="listbox" aria-label="Slash suggestions">
            {#each slashSuggestions as suggestion, index (`${suggestion.kind}-${suggestion.label}`)}
              <button
                type="button"
                class={`slash-option ${index === slashSelectionIndex ? 'active' : ''}`}
                on:mousedown|preventDefault={() => applySlashSuggestionAt(index)}
                on:mouseenter={() => (slashSelectionIndex = index)}
              >
                <span class="slash-option-label">{suggestion.label}</span>
                <span class="slash-option-description">{suggestion.description}</span>
              </button>
            {/each}
          </div>
        {/if}

        <div class="composer">
          <textarea
            bind:this={inputEl}
            bind:value={inputValue}
            rows="1"
            placeholder="Message Rika... (try /help or Cmd/Ctrl+K)"
            on:input={autoResize}
            on:keydown={onComposerKeydown}
            disabled={isWaiting}
          ></textarea>

          <button type="button" class="send" on:click={sendMessage} disabled={!canSend} aria-label="Send message">
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path d="M2.2 21.8L23 12 2.2 2.2 2 9.8l14.5 2.2L2 14.2z"></path>
            </svg>
          </button>

          <button
            type="button"
            class="kill-switch"
            on:click={requestKillSwitch}
            disabled={!canKill}
            aria-label="Stop response"
          >
            Stop
          </button>
        </div>
      </footer>
      </main>
    {:else}
      <main class="settings-pane">
        {#if activeSettingsSection === 'permissions'}
          <section class="settings-card">
          <p class="sidebar-label">Settings</p>
          <h2>Permissions</h2>
          <p class="settings-description">
            Configure tool permissions. Exact tool names only. Use `*` wildcard in argument patterns.
          </p>

          <label class="settings-toggle">
            <input type="checkbox" bind:checked={permissionsEnabled} />
            <span>Enable permissions policy</span>
          </label>

          <div class="settings-grid">
            <label class="settings-field">
              <span>Allow rules (default deny when empty)</span>
              <textarea
                rows="10"
                bind:value={permissionsAllowText}
                placeholder="shell(command:git commit *)"
                disabled={!permissionsLoaded || permissionsSaving}
              ></textarea>
            </label>

            <label class="settings-field">
              <span>Deny rules</span>
              <textarea
                rows="10"
                bind:value={permissionsDenyText}
                placeholder="shell(command:git push *)"
                disabled={!permissionsLoaded || permissionsSaving}
              ></textarea>
            </label>
          </div>

          {#if permissionsErrors.length > 0}
            <div class="settings-errors">
              {#each permissionsErrors as error (`perm-error-${error}`)}
                <p>{error}</p>
              {/each}
            </div>
          {/if}

          <div class="settings-actions">
            <button
              type="button"
              class="tool-toggle"
              on:click={savePermissions}
              disabled={!permissionsLoaded || permissionsSaving}
            >
              {permissionsSaving ? 'Saving…' : 'Save permissions'}
            </button>
            {#if permissionsSavedAt}
              <span class="settings-saved">Saved at {new Date(permissionsSavedAt).toLocaleTimeString()}</span>
            {/if}
          </div>
          </section>
        {/if}
      </main>
    {/if}
  </div>
</div>

{#if commandBarOpen}
  <aside class="commandbar-overlay" role="presentation">
    <button
      type="button"
      class="commandbar-backdrop"
      aria-label="Close session switcher"
      on:click={closeCommandBar}
    ></button>
    <div
      class="commandbar"
      role="dialog"
      aria-modal="true"
      aria-label="Session switcher"
      tabindex="-1"
    >
      <input
        bind:this={commandBarInputEl}
        bind:value={commandBarQuery}
        class="commandbar-input"
        type="text"
        placeholder="Jump to session..."
        on:keydown={onCommandBarKeydown}
      />

      <div class="commandbar-list" role="listbox" aria-label="Navigation items">
        {#if commandBarItems.length === 0}
          <p class="commandbar-empty">No matching items.</p>
        {:else}
          {#each commandBarItems as item, index (item.id)}
            <button
              type="button"
              class={`commandbar-item ${index === commandBarSelectionIndex ? 'active' : ''}`}
              on:mouseenter={() => (commandBarSelectionIndex = index)}
              on:click={() => chooseCommandBarItem(item)}
            >
              <span class="commandbar-item-title">{item.title}</span>
              <span class="commandbar-item-meta">
                {item.meta}
                {#if item.kind === 'session' && item.session.id === currentSessionId}
                  · current
                {/if}
              </span>
            </button>
          {/each}
        {/if}
      </div>
    </div>
  </aside>
{/if}

{#if showReconnectOverlay}
  <aside class="reconnect-overlay" role="status" aria-live="polite">
    <div class="spinner"></div>
    <p>Reconnecting…</p>
  </aside>
{/if}
