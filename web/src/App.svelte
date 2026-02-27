<script lang="ts">
  import { onDestroy, onMount, tick } from 'svelte';

  type ConnectionState = 'connecting' | 'connected' | 'disconnected';

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

  type ServerEvent =
    | { type: 'chunk'; content?: string }
    | { type: 'tool_call_start'; name?: string; args?: unknown }
    | { type: 'tool_call_result'; name?: string; output?: string; success?: boolean }
    | { type: 'done'; full_response?: string }
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
      };

  let entries: Entry[] = [];
  let threads: ThreadRecord[] = [];
  let currentSessionId: string | null = null;

  let inputValue = '';
  let isWaiting = false;
  let isTyping = false;
  let connectionState: ConnectionState = 'connecting';
  let showReconnectOverlay = false;

  let ws: WebSocket | null = null;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let reconnectDelay = 1000;
  let currentAssistantId: string | null = null;
  let disposed = false;
  let idCounter = 0;

  let messagesEl: HTMLElement | null = null;
  let inputEl: HTMLTextAreaElement | null = null;

  $: canSend =
    !isWaiting &&
    connectionState === 'connected' &&
    inputValue.trim().length > 0;

  $: statusLabel =
    connectionState === 'connected'
      ? 'Connected'
      : connectionState === 'connecting'
        ? 'Connecting'
        : 'Disconnected';

  onMount(() => {
    connect();
    autoResize();
  });

  onDestroy(() => {
    disposed = true;
    if (reconnectTimer) {
      clearTimeout(reconnectTimer);
      reconnectTimer = null;
    }
    ws?.close();
  });

  function nextId(): string {
    idCounter += 1;
    return `${Date.now()}-${idCounter}`;
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
        onDone(event.full_response ?? '');
        return;
      case 'thread_list':
        applyThreadState(event.sessions, event.current_session_id);
        return;
      case 'thread_created':
      case 'thread_switched':
      case 'thread_cleared':
      case 'thread_deleted':
        applyThreadState(event.sessions, event.current_session_id ?? event.session_id);
        hydrateCurrentThread(event.history ?? []);
        return;
      case 'thread_renamed':
        applyThreadState(event.sessions, event.current_session_id);
        return;
      case 'error':
        onError(event.message ?? 'Unknown error');
    }
  }

  function applyThreadState(nextThreads?: ThreadRecord[], currentId?: string): void {
    if (Array.isArray(nextThreads)) {
      threads = [...nextThreads];
    }
    if (currentId) {
      currentSessionId = currentId;
    }
  }

  function hydrateCurrentThread(history: HistoryMessage[]): void {
    entries = hydrateEntriesFromHistory(history);
    finishCurrentBubble();
    isTyping = false;
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
            open: false
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
              open: false
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
          open: false
        };
      }
    }

    return rebuilt;
  }

  function onChunk(text: string): void {
    isTyping = false;

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
    isTyping = false;
    finishCurrentBubble();

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
      open: true
    };

    entries = [...entries, entry];
    scrollToBottom();
  }

  function onToolCallResult(name: string, output: string, success: boolean): void {
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
        open: entry.open || !success
      };
      entries = updated;
    }

    scrollToBottom();
  }

  function onDone(fullResponse: string): void {
    isTyping = false;

    if (currentAssistantId) {
      entries = entries.map((entry) => {
        if (entry.kind !== 'message' || entry.id !== currentAssistantId) {
          return entry;
        }

        return {
          ...entry,
          text: fullResponse
        };
      });
      finishCurrentBubble();
    } else if (fullResponse) {
      const entry: MessageEntry = {
        id: nextId(),
        kind: 'message',
        role: 'assistant',
        text: fullResponse
      };
      entries = [...entries, entry];
    }

    setWaiting(false);
    scrollToBottom();
  }

  function onError(message: string): void {
    isTyping = false;
    finishCurrentBubble();

    const entry: MessageEntry = {
      id: nextId(),
      kind: 'message',
      role: 'assistant',
      text: `Error: ${message}`,
      error: true
    };

    entries = [...entries, entry];

    setWaiting(false);
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
      tick().then(() => inputEl?.focus());
    }
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

  function switchThread(sessionId: string): void {
    if (sessionId === currentSessionId) {
      return;
    }
    setWaiting(false);
    isTyping = false;
    sendControl({ type: 'thread_switch', session_id: sessionId });
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
        'Session commands: `/new [name]`, `/rename <name>`, `/clear`, `/delete`'
      );
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
      isTyping = false;
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
      isTyping = false;
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
      return;
    }

    const userEntry: MessageEntry = {
      id: nextId(),
      kind: 'message',
      role: 'user',
      text
    };
    entries = [...entries, userEntry];

    ws.send(
      JSON.stringify({
        type: 'message',
        content: text
      })
    );

    setWaiting(true);
    isTyping = true;
    finishCurrentBubble();
    scrollToBottom();
  }

  function onComposerKeydown(event: KeyboardEvent): void {
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
      <p class="sidebar-label">Sessions</p>
      <div class="thread-list" role="tablist" aria-label="Threads">
        {#each threads as thread (thread.id)}
          <button
            type="button"
            class={`thread-chip ${thread.id === currentSessionId ? 'active' : ''}`}
            on:click={() => switchThread(thread.id)}
            title={thread.display_name}
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
      </div>
    </aside>

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
            </article>
          {:else}
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

        {#if isTyping}
          <div class="typing-indicator" aria-live="polite" aria-label="Assistant is typing">
            <span></span>
            <span></span>
            <span></span>
          </div>
        {/if}
      </section>

      <footer class="composer-wrap">
        <div class="composer">
          <textarea
            bind:this={inputEl}
            bind:value={inputValue}
            rows="1"
            placeholder="Message Rika... (try /help)"
            on:input={autoResize}
            on:keydown={onComposerKeydown}
            disabled={isWaiting}
          ></textarea>

          <button type="button" class="send" on:click={sendMessage} disabled={!canSend} aria-label="Send message">
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path d="M2.2 21.8L23 12 2.2 2.2 2 9.8l14.5 2.2L2 14.2z"></path>
            </svg>
          </button>
        </div>
      </footer>
    </main>
  </div>
</div>

{#if showReconnectOverlay}
  <aside class="reconnect-overlay" role="status" aria-live="polite">
    <div class="spinner"></div>
    <p>Reconnecting…</p>
  </aside>
{/if}
