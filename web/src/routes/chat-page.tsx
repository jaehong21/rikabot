import { useEffect, useMemo, useRef, useState } from "react";
import { ChevronDown, Send, Square } from "lucide-react";

import { useAppStore } from "@/context/app-store";
import { renderMarkdown } from "@/lib/markdown";
import type { ResponseStats, ToolEntry } from "@/types/app";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";

function formatElapsed(ms: number): string {
  const seconds = ms / 1000;
  return seconds >= 10 ? `${seconds.toFixed(0)}s` : `${seconds.toFixed(1)}s`;
}

function formatTokenUsage(stats: ResponseStats): string {
  if (!stats.usage) {
    return "tokens n/a";
  }

  return `tokens ${stats.usage.total_tokens} (in ${stats.usage.prompt_tokens} / out ${stats.usage.completion_tokens})`;
}

function statusVariant(
  status: ToolEntry["status"],
): "default" | "secondary" | "destructive" | "outline" {
  if (status === "success") return "default";
  if (status === "running") return "secondary";
  if (status === "denied") return "outline";
  return "destructive";
}

function statusText(status: ToolEntry["status"]): string {
  if (status === "running") return "Running";
  if (status === "success") return "Success";
  if (status === "denied") return "Denied";
  return "Failed";
}

export function ChatPage() {
  const {
    state,
    sendMessage,
    requestKillSwitch,
    toggleToolOpen,
    updateApprovalRule,
    submitToolApproval,
    clearCurrentThread,
    deleteThread,
    renameThread,
  } = useAppStore();

  const [draft, setDraft] = useState("");
  const [renameValue, setRenameValue] = useState("");
  const transcriptRef = useRef<HTMLDivElement | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  const canSend =
    !state.isWaiting && state.connectionState === "connected" && draft.trim().length > 0;
  const canKill = state.isWaiting && state.connectionState === "connected" && !state.killRequested;

  const activeThread = useMemo(
    () => state.threads.find((thread) => thread.id === state.currentSessionId),
    [state.currentSessionId, state.threads],
  );

  useEffect(() => {
    const node = transcriptRef.current;
    if (!node) {
      return;
    }
    node.scrollTop = node.scrollHeight;
  }, [state.entries, state.isWaiting]);

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

  return (
    <div className="grid h-full min-h-0 grid-rows-[auto_1fr_auto] bg-transparent">
      <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border/70 bg-card/40 px-3 py-2 md:px-4">
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <Badge variant="outline" className="rounded-sm">
            {activeThread?.message_count ?? 0} msgs
          </Badge>
          <Badge variant="outline" className="rounded-sm">
            {state.showToolCalls ? "Tools visible" : "Tools hidden"}
          </Badge>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <Input
            value={renameValue}
            onChange={(event) => setRenameValue(event.currentTarget.value)}
            placeholder="Rename thread"
            className="h-8 w-[180px]"
          />
          <Button
            size="sm"
            variant="outline"
            onClick={() => {
              if (!state.currentSessionId || !renameValue.trim()) {
                return;
              }
              renameThread(state.currentSessionId, renameValue.trim());
              setRenameValue("");
            }}
          >
            Rename
          </Button>
          <Button
            size="sm"
            variant="outline"
            onClick={clearCurrentThread}
            disabled={!state.currentSessionId}
          >
            Clear
          </Button>
          <Button
            size="sm"
            variant="destructive"
            onClick={() => state.currentSessionId && deleteThread(state.currentSessionId)}
            disabled={!state.currentSessionId}
          >
            Delete
          </Button>
        </div>
      </div>

      <ScrollArea className="min-h-0">
        <div
          ref={transcriptRef}
          className="scroll-soft h-full overflow-y-auto px-3 pb-5 pt-4 md:px-4"
        >
          <div className="mx-auto w-full max-w-4xl space-y-4">
            {state.entries.length === 0 && (
              <Card className="border-dashed bg-card/70">
                <CardHeader>
                  <CardTitle className="display-heading text-2xl">Ready when you are</CardTitle>
                  <CardDescription>
                    Ask about your repo, stream tool activity, and control permissions inline. Try
                    `/help` for commands.
                  </CardDescription>
                </CardHeader>
              </Card>
            )}

            {state.entries.map((entry) => {
              if (entry.kind === "message") {
                return (
                  <article
                    key={entry.id}
                    className={cn(
                      "rounded-xl border px-4 py-3 shadow-sm",
                      entry.role === "user"
                        ? "ml-auto max-w-[85%] border-primary/40 bg-primary/10"
                        : "max-w-[92%] border-border/80 bg-card/85",
                      entry.error && "border-destructive/60 bg-destructive/10",
                    )}
                  >
                    <div
                      className="message-prose text-sm"
                      dangerouslySetInnerHTML={{ __html: renderMarkdown(entry.text) }}
                    />
                    {entry.role === "assistant" && entry.stats && (
                      <p className="mt-2 text-xs text-muted-foreground">
                        {formatElapsed(entry.stats.elapsedMs)} · tools {entry.stats.toolCalls}{" "}
                        (success {entry.stats.toolSuccess}
                        {" / "}denied {entry.stats.toolDenied} / failed {entry.stats.toolFailed}) ·{" "}
                        {formatTokenUsage(entry.stats)}
                      </p>
                    )}
                  </article>
                );
              }

              if (!state.showToolCalls) {
                return null;
              }

              return (
                <Collapsible
                  key={entry.id}
                  open={entry.open}
                  onOpenChange={() => toggleToolOpen(entry.id)}
                  className="rounded-xl border border-border/80 bg-card/75"
                >
                  <CollapsibleTrigger asChild>
                    <button
                      type="button"
                      className="flex w-full items-center justify-between gap-3 px-4 py-3 text-left hover:bg-foreground/[0.03]"
                    >
                      <div>
                        <p className="text-xs uppercase tracking-[0.18em] text-muted-foreground">
                          Tool
                        </p>
                        <p className="font-mono text-sm">{entry.name}</p>
                        {!entry.open && (
                          <p className="mt-1 line-clamp-1 text-xs text-muted-foreground">
                            {entry.argsPreview}
                          </p>
                        )}
                      </div>
                      <div className="flex items-center gap-2">
                        <Badge variant={statusVariant(entry.status)}>
                          {statusText(entry.status)}
                        </Badge>
                        <ChevronDown
                          className={cn("h-4 w-4 transition-transform", entry.open && "rotate-180")}
                        />
                      </div>
                    </button>
                  </CollapsibleTrigger>

                  <CollapsibleContent className="space-y-3 border-t border-border/80 px-4 py-3 text-sm">
                    <div>
                      <p className="mb-1 text-xs font-semibold uppercase tracking-[0.16em] text-muted-foreground">
                        Arguments
                      </p>
                      <pre className="overflow-auto rounded-md border border-border/20 bg-input px-3 py-2 font-mono text-xs text-foreground">
                        {entry.args}
                      </pre>
                    </div>

                    {entry.output && (
                      <div>
                        <p className="mb-1 text-xs font-semibold uppercase tracking-[0.16em] text-muted-foreground">
                          Output
                        </p>
                        <pre className="max-h-56 overflow-auto rounded-md border border-border/20 bg-input px-3 py-2 font-mono text-xs text-foreground">
                          {entry.output}
                        </pre>
                      </div>
                    )}

                    {entry.awaitingApproval && entry.approval && (
                      <Card className="border-primary/35 bg-primary/5">
                        <CardHeader className="pb-2">
                          <CardTitle className="text-base">Approval required</CardTitle>
                          <CardDescription>
                            This tool call was blocked by permissions. Persist a rule, allow once,
                            or deny.
                          </CardDescription>
                        </CardHeader>
                        <CardContent className="space-y-3">
                          <Input
                            value={entry.approval.allowRuleInput}
                            disabled={entry.approval.submitting}
                            onChange={(event) =>
                              updateApprovalRule(entry.id, event.currentTarget.value)
                            }
                            placeholder="shell(command:git status *)"
                          />
                          <div className="flex flex-wrap gap-2">
                            <Button
                              size="sm"
                              onClick={() => submitToolApproval(entry.id, "allow_persist")}
                              disabled={entry.approval.submitting}
                            >
                              Always allow
                            </Button>
                            <Button
                              size="sm"
                              variant="outline"
                              onClick={() => submitToolApproval(entry.id, "allow_once")}
                              disabled={entry.approval.submitting}
                            >
                              Allow once
                            </Button>
                            <Button
                              size="sm"
                              variant="destructive"
                              onClick={() => submitToolApproval(entry.id, "deny")}
                              disabled={entry.approval.submitting}
                            >
                              Deny
                            </Button>
                          </div>
                        </CardContent>
                      </Card>
                    )}
                  </CollapsibleContent>
                </Collapsible>
              );
            })}

            {state.isWaiting && (
              <div className="inline-flex items-center gap-1 rounded-full border border-border bg-card px-4 py-2">
                <span className="inline-block h-1.5 w-1.5 animate-pulse rounded-full bg-foreground/70" />
                <span className="inline-block h-1.5 w-1.5 animate-pulse rounded-full bg-foreground/70 [animation-delay:120ms]" />
                <span className="inline-block h-1.5 w-1.5 animate-pulse rounded-full bg-foreground/70 [animation-delay:240ms]" />
              </div>
            )}
          </div>
        </div>
      </ScrollArea>

      <footer className="border-t border-border/70 bg-card/70 px-3 py-3 md:px-4">
        <div className="mx-auto flex w-full max-w-4xl items-end gap-2">
          <Textarea
            ref={textareaRef}
            value={draft}
            onChange={(event) => {
              setDraft(event.currentTarget.value);
              autoResize();
            }}
            rows={1}
            className="max-h-[180px] min-h-[44px] resize-none"
            placeholder="Message Rika... (/help for commands)"
            onKeyDown={(event) => {
              if (event.key === "Enter" && !event.shiftKey) {
                event.preventDefault();
                submit();
              }
            }}
            disabled={state.isWaiting}
          />
          <Button size="icon" onClick={submit} disabled={!canSend} aria-label="Send message">
            <Send className="h-4 w-4" />
          </Button>
          <Button
            size="icon"
            variant="outline"
            onClick={requestKillSwitch}
            disabled={!canKill}
            aria-label="Stop response"
          >
            <Square className="h-4 w-4" />
          </Button>
        </div>
      </footer>
    </div>
  );
}
