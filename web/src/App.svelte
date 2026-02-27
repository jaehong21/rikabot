<script lang="ts">
  import { onDestroy, onMount, tick } from 'svelte';

  type ConnectionState = 'connecting' | 'connected' | 'disconnected';

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
    | { type: 'error'; message?: string };

  let entries: Entry[] = [];
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
      case 'error':
        onError(event.message ?? 'Unknown error');
    }
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

  function sendMessage(): void {
    const text = inputValue.trim();
    if (!text || isWaiting || !ws || ws.readyState !== WebSocket.OPEN) {
      return;
    }

    const userEntry: MessageEntry = {
      id: nextId(),
      kind: 'message',
      role: 'user',
      text
    };
    entries = [...entries, userEntry];

    inputValue = '';
    autoResize();

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
        placeholder="Message Rika..."
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
</div>

{#if showReconnectOverlay}
  <aside class="reconnect-overlay" role="status" aria-live="polite">
    <div class="spinner"></div>
    <p>Reconnecting…</p>
  </aside>
{/if}
