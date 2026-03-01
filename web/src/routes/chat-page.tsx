import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowUp, ChevronDown, ChevronRight, Square } from "lucide-react";

import { useAppStore } from "@/context/app-store";
import { renderMarkdown } from "@/lib/markdown";
import type { MessageEntry, ToolEntry } from "@/types/app";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
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
  const {
    state,
    sendMessage,
    requestKillSwitch,
    toggleToolOpen,
    updateApprovalRule,
    submitToolApproval,
  } = useAppStore();

  const [draft, setDraft] = useState("");
  const [openToolGroups, setOpenToolGroups] = useState<Record<string, boolean>>(
    {},
  );
  const transcriptRef = useRef<HTMLDivElement | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  const canSend =
    !state.isWaiting &&
    state.connectionState === "connected" &&
    draft.trim().length > 0;
  const canKill =
    state.isWaiting &&
    state.connectionState === "connected" &&
    !state.killRequested;
  const isEmpty = state.entries.length === 0;

  useEffect(() => {
    const container = transcriptRef.current;
    if (!container) {
      return;
    }

    const viewport = container.closest("[data-radix-scroll-area-viewport]");
    const scrollNode = viewport instanceof HTMLElement ? viewport : container;

    const frame = requestAnimationFrame(() => {
      scrollNode.scrollTo({ top: scrollNode.scrollHeight, behavior: "smooth" });
    });
    const settle = window.setTimeout(() => {
      scrollNode.scrollTop = scrollNode.scrollHeight;
    }, 220);

    return () => {
      cancelAnimationFrame(frame);
      window.clearTimeout(settle);
    };
  }, [state.entries, state.isWaiting, state.currentSessionId]);

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
                    <div key={entry.id} className="flex justify-end">
                      <article
                        className={cn(
                          "inline-block w-fit max-w-[85%] rounded-2xl bg-userbubble px-4 py-3",
                          entry.error && "border border-primary/50",
                        )}
                      >
                        <div
                          className="message-prose text-sm leading-5 [&_br]:leading-4"
                          dangerouslySetInnerHTML={{
                            __html: renderMarkdown(entry.text),
                          }}
                        />
                      </article>
                    </div>
                  );
                }

                return (
                  <article
                    key={entry.id}
                    className={cn(
                      "message-prose max-w-none text-[15px] leading-6 text-foreground [&_br]:leading-4",
                      entry.error && "text-primary",
                    )}
                  >
                    <div
                      dangerouslySetInnerHTML={{
                        __html: renderMarkdown(entry.text),
                      }}
                    />
                  </article>
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
            <Textarea
              ref={textareaRef}
              value={draft}
              onChange={(event) => {
                setDraft(event.currentTarget.value);
                autoResize();
              }}
              rows={1}
              className="max-h-[180px] min-h-[36px] resize-none border-0 bg-transparent px-1 py-2 text-sm shadow-none focus-visible:ring-0"
              placeholder={isEmpty ? "How can I help you today?" : "Reply..."}
              onKeyDown={(event) => {
                if (event.key === "Enter" && !event.shiftKey) {
                  event.preventDefault();
                  submit();
                }
              }}
              disabled={state.isWaiting}
            />
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
