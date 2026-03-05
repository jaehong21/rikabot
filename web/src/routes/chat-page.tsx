import { useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowUp,
  ChevronDown,
  ChevronRight,
  Check,
  Copy,
  Square,
  X,
} from "lucide-react";
import { useNavigate, useSearch } from "@tanstack/react-router";

import { useAppStore } from "@/context/app-store";
import { renderMarkdown } from "@/lib/markdown";
import type { MessageEntry, ThreadRecord, ToolEntry } from "@/types/app";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Popover,
  PopoverAnchor,
  PopoverContent,
} from "@/components/ui/popover";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) {
    return false;
  }

  const tagName = target.tagName.toLowerCase();
  if (tagName === "input" || tagName === "textarea" || tagName === "select") {
    return true;
  }

  if (target.isContentEditable) {
    return true;
  }

  return target.closest('[contenteditable="true"]') !== null;
}

function toolStatusText(status: ToolEntry["status"]): string {
  if (status === "running") return "Running";
  if (status === "success") return "Success";
  if (status === "denied") return "Denied";
  return "Failed";
}

function toolStatusBadgeClass(status: ToolEntry["status"]): string {
  if (status === "failed") {
    return "border border-red-200 bg-red-100 text-red-700";
  }
  if (status === "denied") {
    return "border border-amber-200 bg-amber-100 text-amber-700";
  }
  return "bg-foreground/10 text-foreground/80";
}

type SlashSuggestion = {
  completion: string;
  label: string;
  hint: string;
};

const SLASH_COMMANDS: Array<{ command: string; hint: string }> = [
  {
    command: "help",
    hint: "Show session and tool command usage",
  },
  {
    command: "new",
    hint: "Create a new chat session (optional title)",
  },
  {
    command: "rename",
    hint: "Rename the current chat session",
  },
  {
    command: "clear",
    hint: "Clear current session messages",
  },
  {
    command: "delete",
    hint: "Delete current session",
  },
  {
    command: "stop",
    hint: "Cancel active generation",
  },
  {
    command: "tools",
    hint: "Control tool output visibility",
  },
];

const TOOL_SUBCOMMANDS: Array<{ value: string; hint: string }> = [
  { value: "collapse", hint: "Collapse all tool outputs" },
  { value: "expand", hint: "Expand all tool outputs" },
  { value: "hide", hint: "Hide all tool output blocks" },
  { value: "show", hint: "Show all tool output blocks" },
];

function getSlashSuggestions(
  draft: string,
  threads: ThreadRecord[],
): SlashSuggestion[] {
  if (!draft.startsWith("/")) {
    return [];
  }

  const body = draft.slice(1);
  if (!body.trim()) {
    return SLASH_COMMANDS.map((command) => ({
      completion: `/${command.command}`,
      label: `/${command.command}`,
      hint: command.hint,
    }));
  }

  const normalizedBody = body.trimStart();
  const firstSpace = normalizedBody.indexOf(" ");

  if (firstSpace === -1) {
    const query = normalizedBody.toLowerCase();
    return SLASH_COMMANDS.filter((command) =>
      command.command.startsWith(query),
    ).map((command) => ({
      completion: `/${command.command}`,
      label: `/${command.command}`,
      hint: command.hint,
    }));
  }

  const command = normalizedBody.slice(0, firstSpace).toLowerCase();
  const rawArg = normalizedBody.slice(firstSpace + 1).trimStart();

  if (command === "tools") {
    const query = rawArg.toLowerCase();
    return TOOL_SUBCOMMANDS.filter((subcommand) =>
      subcommand.value.startsWith(query),
    ).map((subcommand) => ({
      completion: `/tools ${subcommand.value}`,
      label: `/tools ${subcommand.value}`,
      hint: subcommand.hint,
    }));
  }

  if (command === "rename") {
    const query = rawArg.toLowerCase();
    if (!query.trim()) {
      return [];
    }
    return threads
      .filter((thread) =>
        thread.display_name.toLowerCase().includes(query.toLowerCase()),
      )
      .map((thread) => ({
        completion: `/rename ${thread.display_name}`,
        label: `/rename ${thread.display_name}`,
        hint: "Existing session name",
      }));
  }

  return [];
}

function summarizeToolGroup(entries: ToolEntry[]): string {
  const total = entries.length;
  const running = entries.filter((entry) => entry.status === "running").length;
  const denied = entries.filter((entry) => entry.status === "denied").length;
  const failed = entries.filter((entry) => entry.status === "failed").length;

  if (running > 0) {
    return `Running ${running} of ${total} tool call${total > 1 ? "s" : ""}`;
  }

  if (denied > 0 || failed > 0) {
    return `Ran ${total} tool call${total > 1 ? "s" : ""} (${denied} denied, ${failed} failed)`;
  }

  return `Ran ${total} tool call${total > 1 ? "s" : ""}`;
}

export function ChatPage() {
  const navigate = useNavigate();
  const search = useSearch({ from: "/" });
  const {
    state,
    sendMessage,
    requestKillSwitch,
    cancelQueuedInput,
    switchThread,
    toggleToolOpen,
    updateApprovalRule,
    submitToolApproval,
  } = useAppStore();

  const [draft, setDraft] = useState("");
  const [activeSlashSuggestionIndex, setActiveSlashSuggestionIndex] =
    useState(-1);
  const [copiedMessageId, setCopiedMessageId] = useState<string | null>(null);
  const [openToolGroups, setOpenToolGroups] = useState<Record<string, boolean>>(
    {},
  );
  const transcriptRef = useRef<HTMLDivElement | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const copyResetTimeoutRef = useRef<number | null>(null);
  const pendingQuerySessionSwitchRef = useRef<string | null>(null);
  const scrollStateRef = useRef<{
    initialized: boolean;
    sessionId: string | null;
    messageCount: number;
    toolCount: number;
  }>({
    initialized: false,
    sessionId: null,
    messageCount: 0,
    toolCount: 0,
  });

  const canSend =
    state.connectionState === "connected" && draft.trim().length > 0;
  const canKill =
    state.isWaiting &&
    state.connectionState === "connected" &&
    !state.killRequested;
  const isEmpty = state.entries.length === 0;
  const slashSuggestions = useMemo(
    () => getSlashSuggestions(draft, state.threads),
    [draft, state.threads],
  );
  const showSlashSuggestions = slashSuggestions.length > 0;

  useEffect(() => {
    if (!showSlashSuggestions) {
      setActiveSlashSuggestionIndex(-1);
      return;
    }
    setActiveSlashSuggestionIndex((previous) => {
      if (previous < 0) {
        return 0;
      }
      return previous >= slashSuggestions.length
        ? slashSuggestions.length - 1
        : previous;
    });
  }, [showSlashSuggestions, slashSuggestions]);

  useEffect(() => {
    const querySession =
      typeof search.session === "string" ? search.session.trim() : "";

    if (!querySession) {
      pendingQuerySessionSwitchRef.current = null;
      return;
    }
    if (state.currentSessionId === querySession) {
      pendingQuerySessionSwitchRef.current = null;
      return;
    }
    if (pendingQuerySessionSwitchRef.current === querySession) {
      return;
    }
    if (!state.threads.some((thread) => thread.id === querySession)) {
      pendingQuerySessionSwitchRef.current = null;
      return;
    }

    pendingQuerySessionSwitchRef.current = querySession;
    switchThread(querySession);
  }, [search.session, state.currentSessionId, state.threads, switchThread]);

  useEffect(() => {
    const querySession =
      typeof search.session === "string" ? search.session.trim() : "";
    const hasQuerySession =
      querySession.length > 0 &&
      state.threads.some((thread) => thread.id === querySession);

    if (state.threads.length === 0) {
      return;
    }

    const fallbackSessionId =
      state.currentSessionId ?? state.threads[0]?.id ?? "";
    const targetSessionId = hasQuerySession ? querySession : fallbackSessionId;
    if (!targetSessionId) {
      return;
    }

    if (querySession !== targetSessionId) {
      navigate({
        to: "/",
        search: { session: targetSessionId },
        replace: true,
      });
    }
  }, [navigate, search.session, state.currentSessionId, state.threads]);

  useEffect(() => {
    const container = transcriptRef.current;
    if (!container) {
      return;
    }

    const messageCount = state.entries.reduce(
      (count, entry) => count + (entry.kind === "message" ? 1 : 0),
      0,
    );
    const toolCount = state.entries.reduce(
      (count, entry) => count + (entry.kind === "tool" ? 1 : 0),
      0,
    );
    const previous = scrollStateRef.current;
    const sessionChanged = previous.sessionId !== state.currentSessionId;
    const shouldSmoothScroll =
      previous.initialized &&
      !sessionChanged &&
      (messageCount > previous.messageCount || toolCount > previous.toolCount);
    const shouldJumpToBottom = !previous.initialized || sessionChanged;

    const viewport = container.closest("[data-radix-scroll-area-viewport]");
    const scrollNode = viewport instanceof HTMLElement ? viewport : container;

    const frame = requestAnimationFrame(() => {
      if (shouldSmoothScroll) {
        scrollNode.scrollTo({
          top: scrollNode.scrollHeight,
          behavior: "smooth",
        });
        return;
      }

      if (shouldJumpToBottom) {
        scrollNode.scrollTop = scrollNode.scrollHeight;
      }
    });
    scrollStateRef.current = {
      initialized: true,
      sessionId: state.currentSessionId,
      messageCount,
      toolCount,
    };

    return () => {
      cancelAnimationFrame(frame);
    };
  }, [state.entries, state.currentSessionId]);

  useEffect(() => {
    const handleSlashFocus = (event: KeyboardEvent): void => {
      if (
        event.defaultPrevented ||
        event.key !== "/" ||
        event.metaKey ||
        event.ctrlKey ||
        event.altKey ||
        isEditableTarget(event.target)
      ) {
        return;
      }

      event.preventDefault();
      textareaRef.current?.focus();
    };

    window.addEventListener("keydown", handleSlashFocus);
    return () => {
      window.removeEventListener("keydown", handleSlashFocus);
    };
  }, []);

  const submit = (): void => {
    if (!canSend) {
      return;
    }
    sendMessage(draft);
    setDraft("");
    if (textareaRef.current) {
      textareaRef.current.style.height = "auto";
    }
  };

  const autoResize = (): void => {
    const node = textareaRef.current;
    if (!node) {
      return;
    }
    node.style.height = "auto";
    node.style.height = `${Math.min(node.scrollHeight, 180)}px`;
  };

  const applySlashSuggestion = (suggestion: SlashSuggestion): void => {
    setDraft(suggestion.completion);
    window.requestAnimationFrame(() => {
      autoResize();
      textareaRef.current?.focus();
    });
  };

  const copyText = async (text: string): Promise<void> => {
    try {
      await navigator.clipboard.writeText(text);
      return;
    } catch {
      const textarea = document.createElement("textarea");
      textarea.value = text;
      textarea.style.position = "fixed";
      textarea.style.opacity = "0";
      document.body.appendChild(textarea);
      textarea.select();
      document.execCommand("copy");
      document.body.removeChild(textarea);
    }
  };

  const handleCopy = (id: string, text: string): void => {
    void copyText(text).then(() => {
      if (copyResetTimeoutRef.current !== null) {
        window.clearTimeout(copyResetTimeoutRef.current);
      }
      setCopiedMessageId(id);
      copyResetTimeoutRef.current = window.setTimeout(() => {
        setCopiedMessageId((current) => (current === id ? null : current));
      }, 1200);
    });
  };

  const groupedEntries = useMemo(() => {
    const groups: Array<
      | { kind: "message"; entry: MessageEntry }
      | { kind: "tool-group"; id: string; entries: ToolEntry[] }
    > = [];

    for (let index = 0; index < state.entries.length; index += 1) {
      const entry = state.entries[index];

      if (entry.kind === "message") {
        groups.push({ kind: "message", entry });
        continue;
      }

      const toolEntries: ToolEntry[] = [entry];
      while (
        index + 1 < state.entries.length &&
        state.entries[index + 1].kind === "tool"
      ) {
        toolEntries.push(state.entries[index + 1] as ToolEntry);
        index += 1;
      }

      groups.push({
        kind: "tool-group",
        id: toolEntries[0].id,
        entries: toolEntries,
      });
    }

    return groups;
  }, [state.entries]);

  return (
    <div className="relative h-full min-h-0 bg-transparent">
      <ScrollArea className="h-full min-h-0">
        <div
          ref={transcriptRef}
          className="scroll-soft h-full overflow-y-auto px-4 pb-44 pt-4 md:px-6"
        >
          <div className="mx-auto w-full max-w-[760px] space-y-6">
            {groupedEntries.map((item) => {
              if (item.kind === "message") {
                const entry = item.entry;
                if (entry.role === "user") {
                  return (
                    <div
                      key={entry.id}
                      className="flex flex-col items-end gap-1"
                    >
                      <article
                        className={cn(
                          "inline-block w-fit max-w-[85%] rounded-2xl bg-userbubble px-4 py-3",
                          entry.error && "border border-primary/50",
                        )}
                      >
                        <div
                          className="message-prose text-sm leading-5"
                          dangerouslySetInnerHTML={{
                            __html: renderMarkdown(entry.text),
                          }}
                        />
                      </article>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        className="h-6 w-6 text-muted-foreground/60 transition-colors hover:text-foreground/90"
                        onClick={() => {
                          handleCopy(entry.id, entry.text);
                        }}
                        aria-label="Copy user message"
                      >
                        {copiedMessageId === entry.id ? (
                          <Check className="h-3.5 w-3.5" />
                        ) : (
                          <Copy className="h-3.5 w-3.5" />
                        )}
                      </Button>
                    </div>
                  );
                }

                return (
                  <div key={entry.id} className="flex flex-col gap-1">
                    <article
                      className={cn(
                        "message-prose max-w-none text-[15px] leading-6 text-foreground",
                        entry.error && "text-primary",
                      )}
                    >
                      <div
                        dangerouslySetInnerHTML={{
                          __html: renderMarkdown(entry.text),
                        }}
                      />
                    </article>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      className="h-6 w-6 text-muted-foreground/60 transition-colors hover:text-foreground/90"
                      onClick={() => {
                        handleCopy(entry.id, entry.text);
                      }}
                      aria-label="Copy assistant response"
                    >
                      {copiedMessageId === entry.id ? (
                        <Check className="h-3.5 w-3.5" />
                      ) : (
                        <Copy className="h-3.5 w-3.5" />
                      )}
                    </Button>
                  </div>
                );
              }

              const hasApproval = item.entries.some(
                (entry) => entry.awaitingApproval && entry.approval,
              );
              if (!state.showToolCalls && !hasApproval) {
                return null;
              }

              const groupOpen = openToolGroups[item.id] ?? true;

              return (
                <section key={item.id} className="space-y-2">
                  <button
                    type="button"
                    onClick={() =>
                      setOpenToolGroups((prev) => ({
                        ...prev,
                        [item.id]: !(prev[item.id] ?? true),
                      }))
                    }
                    className="inline-flex items-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground"
                  >
                    {groupOpen ? (
                      <ChevronDown className="h-3.5 w-3.5 opacity-50" />
                    ) : (
                      <ChevronRight className="h-3.5 w-3.5 opacity-50" />
                    )}
                    <span className="opacity-50">
                      {summarizeToolGroup(item.entries)}
                    </span>
                  </button>

                  {(groupOpen || hasApproval) && (
                    <div className="space-y-2 pl-2">
                      {item.entries.map((entry) => (
                        <article key={entry.id} className="py-1">
                          <button
                            type="button"
                            onClick={() => toggleToolOpen(entry.id)}
                            className="flex w-full items-start gap-2 text-left"
                          >
                            <div className="min-w-0 flex-1">
                              <div className="flex items-center gap-2">
                                <p className="truncate text-sm text-foreground opacity-50">
                                  {entry.name}
                                </p>
                                <span
                                  className={cn(
                                    "shrink-0 rounded px-1.5 py-0.5 font-mono text-[10px] leading-none",
                                    toolStatusBadgeClass(entry.status),
                                  )}
                                >
                                  {toolStatusText(entry.status)}
                                </span>
                              </div>
                              {!entry.open && (
                                <p className="mt-1 line-clamp-1 text-xs text-muted-foreground opacity-50">
                                  {entry.argsPreview || "No input"}
                                </p>
                              )}
                            </div>
                            {entry.open ? (
                              <ChevronDown className="mt-0.5 h-3.5 w-3.5 text-muted-foreground" />
                            ) : (
                              <ChevronRight className="mt-0.5 h-3.5 w-3.5 text-muted-foreground" />
                            )}
                          </button>

                          {entry.open && (
                            <div className="mt-2 space-y-2">
                              <div>
                                <p className="text-[11px] uppercase tracking-[0.12em] text-muted-foreground">
                                  Input
                                </p>
                                <pre className="mt-1 overflow-x-auto rounded-lg border border-border/60 bg-input px-3 py-2 text-xs text-foreground">
                                  {entry.args ||
                                    entry.argsPreview ||
                                    "No input"}
                                </pre>
                              </div>

                              <div>
                                <p className="text-[11px] uppercase tracking-[0.12em] text-muted-foreground">
                                  Output
                                </p>
                                <pre className="mt-1 max-h-56 overflow-auto rounded-lg border border-border/60 bg-input px-3 py-2 text-xs text-foreground">
                                  {entry.output || "No output"}
                                </pre>
                              </div>

                              {entry.awaitingApproval && entry.approval && (
                                <section className="rounded-lg border border-border bg-input p-3">
                                  <p className="text-sm font-semibold">
                                    Approval required
                                  </p>
                                  <p className="mt-1 text-xs text-muted-foreground">
                                    This tool call was blocked by permissions.
                                    Persist a rule, allow once, or deny.
                                  </p>
                                  <Input
                                    value={entry.approval.allowRuleInput}
                                    disabled={entry.approval.submitting}
                                    onChange={(event) =>
                                      updateApprovalRule(
                                        entry.id,
                                        event.currentTarget.value,
                                      )
                                    }
                                    placeholder="shell(command:git status *)"
                                    className="mt-3"
                                  />
                                  <div className="mt-3 flex flex-wrap gap-2">
                                    <Button
                                      size="sm"
                                      onClick={() =>
                                        submitToolApproval(
                                          entry.id,
                                          "allow_persist",
                                        )
                                      }
                                      disabled={entry.approval.submitting}
                                      className="rounded-[0.55rem] bg-foreground text-input hover:bg-foreground/90"
                                    >
                                      Always allow
                                    </Button>
                                    <Button
                                      size="sm"
                                      variant="outline"
                                      onClick={() =>
                                        submitToolApproval(
                                          entry.id,
                                          "allow_once",
                                        )
                                      }
                                      disabled={entry.approval.submitting}
                                      className="rounded-[0.55rem] border-border bg-transparent text-foreground hover:bg-background"
                                    >
                                      Allow once
                                    </Button>
                                    <Button
                                      size="sm"
                                      variant="outline"
                                      onClick={() =>
                                        submitToolApproval(entry.id, "deny")
                                      }
                                      disabled={entry.approval.submitting}
                                      className="rounded-[0.55rem] border-border bg-transparent text-foreground hover:bg-background"
                                    >
                                      Deny
                                    </Button>
                                  </div>
                                </section>
                              )}
                            </div>
                          )}
                        </article>
                      ))}
                    </div>
                  )}
                </section>
              );
            })}
            {state.isWaiting && (
              <div className="inline-flex items-center gap-1 py-1">
                <span className="inline-block h-1.5 w-1.5 animate-pulse rounded-full bg-foreground/70" />
                <span className="inline-block h-1.5 w-1.5 animate-pulse rounded-full bg-foreground/70 [animation-delay:120ms]" />
                <span className="inline-block h-1.5 w-1.5 animate-pulse rounded-full bg-foreground/70 [animation-delay:240ms]" />
              </div>
            )}
          </div>
        </div>
      </ScrollArea>

      <footer
        className={cn(
          "absolute inset-x-0 px-4 md:px-6",
          isEmpty
            ? "inset-y-0 flex items-center justify-center"
            : "pointer-events-none bottom-0 pb-6",
        )}
      >
        <div className="pointer-events-auto mx-auto w-full max-w-[760px]">
          {isEmpty && (
            <h1 className="mb-8 text-center text-4xl font-semibold leading-tight md:text-5xl">
              👾 Welcome to Rika
            </h1>
          )}
          <div className="rounded-[1.5rem] border border-border/70 bg-input/70 px-4 py-3 transition-all duration-150 hover:border-border hover:bg-input hover:shadow-[0_8px_18px_rgba(20,20,20,0.06)] focus-within:border-border focus-within:bg-input focus-within:shadow-[0_8px_18px_rgba(20,20,20,0.08)]">
            {state.queuedInputs.length > 0 && (
              <section className="mb-3 rounded-xl border border-border/70 bg-background/70 p-3">
                <div className="flex items-center justify-between gap-2">
                  <p className="text-xs font-medium uppercase tracking-[0.12em] text-muted-foreground">
                    Queued messages ({state.queuedInputs.length}/5)
                  </p>
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="h-7 px-2 text-xs"
                    onClick={() => cancelQueuedInput()}
                  >
                    Clear all
                  </Button>
                </div>
                <div className="mt-2 space-y-1">
                  {state.queuedInputs.map((queued) => (
                    <div
                      key={queued.id}
                      className="flex items-center gap-2 rounded-lg border border-border/60 bg-input px-2 py-1.5"
                    >
                      <p className="line-clamp-1 min-w-0 flex-1 text-xs text-foreground/90">
                        {queued.content}
                      </p>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        className="h-6 w-6"
                        onClick={() => cancelQueuedInput(queued.id)}
                        aria-label="Cancel queued message"
                      >
                        <X className="h-3.5 w-3.5" />
                      </Button>
                    </div>
                  ))}
                </div>
              </section>
            )}
            <Popover open={showSlashSuggestions} onOpenChange={() => {}}>
              <PopoverAnchor asChild>
                <div>
                  <Textarea
                    ref={textareaRef}
                    value={draft}
                    onChange={(event) => {
                      setDraft(event.currentTarget.value);
                      autoResize();
                    }}
                    rows={1}
                    className="max-h-[180px] min-h-[36px] resize-none border-0 bg-transparent px-1 py-2 text-sm shadow-none focus-visible:ring-0"
                    placeholder={
                      isEmpty ? "How can I help you today?" : "Reply..."
                    }
                    onKeyDown={(event) => {
                      if (showSlashSuggestions && slashSuggestions.length > 0) {
                        if (event.key === "ArrowDown") {
                          event.preventDefault();
                          setActiveSlashSuggestionIndex((previous) =>
                            previous < 0
                              ? 0
                              : (previous + 1) % slashSuggestions.length,
                          );
                          return;
                        }

                        if (event.key === "ArrowUp") {
                          event.preventDefault();
                          setActiveSlashSuggestionIndex((previous) =>
                            previous <= 0
                              ? slashSuggestions.length - 1
                              : previous - 1,
                          );
                          return;
                        }

                        if (
                          event.key === "Tab" &&
                          activeSlashSuggestionIndex >= 0 &&
                          slashSuggestions[activeSlashSuggestionIndex] !==
                            undefined
                        ) {
                          event.preventDefault();
                          const suggestion =
                            slashSuggestions[activeSlashSuggestionIndex] ??
                            slashSuggestions[0];
                          if (suggestion) {
                            applySlashSuggestion(suggestion);
                          }
                          return;
                        }
                      }

                      if (event.key === "Enter" && !event.shiftKey) {
                        const activeSuggestion =
                          activeSlashSuggestionIndex >= 0
                            ? slashSuggestions[activeSlashSuggestionIndex]
                            : undefined;
                        if (
                          activeSuggestion &&
                          activeSuggestion.completion !== draft.trim()
                        ) {
                          event.preventDefault();
                          applySlashSuggestion(activeSuggestion);
                          return;
                        }
                        event.preventDefault();
                        submit();
                      }
                    }}
                  />
                </div>
              </PopoverAnchor>
              <PopoverContent
                side="top"
                align="start"
                sideOffset={8}
                onOpenAutoFocus={(event) => event.preventDefault()}
                onCloseAutoFocus={(event) => {
                  event.preventDefault();
                  textareaRef.current?.focus();
                }}
                className="p-0"
              >
                <div
                  role="listbox"
                  aria-label="Slash command suggestions"
                  className="scroll-soft max-h-44 overflow-y-auto rounded-xl"
                >
                  {slashSuggestions.map((suggestion, index) => {
                    const isActive = index === activeSlashSuggestionIndex;
                    return (
                      <button
                        key={suggestion.completion}
                        type="button"
                        role="option"
                        aria-selected={isActive}
                        onMouseDown={(event) => {
                          event.preventDefault();
                          applySlashSuggestion(suggestion);
                        }}
                        onMouseEnter={() =>
                          setActiveSlashSuggestionIndex(index)
                        }
                        className={cn(
                          "w-full rounded-lg px-3 py-2 text-left text-sm transition-colors",
                          isActive
                            ? "bg-muted/70"
                            : "text-foreground/90 hover:bg-muted/45",
                          "focus-visible:outline-none focus-visible:ring-0 focus:outline-none",
                        )}
                      >
                        <div className="truncate font-mono text-foreground">
                          {suggestion.label}
                        </div>
                        <div className="truncate text-xs text-muted-foreground">
                          {suggestion.hint}
                        </div>
                      </button>
                    );
                  })}
                </div>
              </PopoverContent>
            </Popover>
            <div className="mt-3 flex items-center justify-end gap-2">
              {canKill && (
                <Button
                  size="icon"
                  variant="outline"
                  onClick={requestKillSwitch}
                  aria-label="Stop response"
                  className="h-8 w-8 rounded-full"
                >
                  <Square className="h-3.5 w-3.5" />
                </Button>
              )}
              <Button
                size="icon"
                onClick={submit}
                disabled={!canSend}
                aria-label="Send message"
                className="h-8 w-8 rounded-full"
              >
                <ArrowUp className="h-3.5 w-3.5" color="white" />
              </Button>
            </div>
          </div>
        </div>
      </footer>
    </div>
  );
}
