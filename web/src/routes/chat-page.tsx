import { useEffect, useRef, useState } from "react";
import { ArrowUp, Square } from "lucide-react";

import { useAppStore } from "@/context/app-store";
import { renderMarkdown } from "@/lib/markdown";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";

export function ChatPage() {
  const { state, sendMessage, requestKillSwitch, updateApprovalRule, submitToolApproval } =
    useAppStore();

  const [draft, setDraft] = useState("");
  const transcriptRef = useRef<HTMLDivElement | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  const canSend =
    !state.isWaiting && state.connectionState === "connected" && draft.trim().length > 0;
  const canKill = state.isWaiting && state.connectionState === "connected" && !state.killRequested;

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
    <div className="relative h-full min-h-0 bg-transparent">
      <ScrollArea className="h-full min-h-0">
        <div
          ref={transcriptRef}
          className="scroll-soft h-full overflow-y-auto px-4 pb-44 pt-4 md:px-6"
        >
          <div className="mx-auto w-full max-w-[760px] space-y-6">
            {state.entries.length === 0 && (
              <p className="pt-2 text-sm text-muted-foreground">Start a conversation.</p>
            )}

            {state.entries.map((entry) => {
              if (entry.kind === "message") {
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
                          className="message-prose text-sm"
                          dangerouslySetInnerHTML={{ __html: renderMarkdown(entry.text) }}
                        />
                      </article>
                    </div>
                  );
                }

                return (
                  <article
                    key={entry.id}
                    className={cn(
                      "message-prose max-w-none text-[15px] leading-7 text-foreground",
                      entry.error && "text-primary",
                    )}
                  >
                    <div dangerouslySetInnerHTML={{ __html: renderMarkdown(entry.text) }} />
                  </article>
                );
              }

              if (!entry.awaitingApproval || !entry.approval) {
                return null;
              }

              return (
                <section key={entry.id} className="rounded-2xl border border-border bg-input p-4">
                  <p className="text-sm font-semibold">Approval required</p>
                  <p className="mt-1 text-xs text-muted-foreground">
                    This tool call was blocked by permissions. Persist a rule, allow once, or deny.
                  </p>
                  <Input
                    value={entry.approval.allowRuleInput}
                    disabled={entry.approval.submitting}
                    onChange={(event) => updateApprovalRule(entry.id, event.currentTarget.value)}
                    placeholder="shell(command:git status *)"
                    className="mt-3"
                  />
                  <div className="mt-3 flex flex-wrap gap-2">
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

      <footer className="pointer-events-none absolute inset-x-0 bottom-0 px-4 pb-6 md:px-6">
        <div className="pointer-events-auto mx-auto w-full max-w-[760px]">
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
              placeholder="Reply..."
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
