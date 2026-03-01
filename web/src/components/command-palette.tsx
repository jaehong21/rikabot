import { useMemo, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import {
  BookOpenText,
  CirclePlus,
  Eraser,
  Eye,
  EyeOff,
  PanelBottomOpen,
  PanelTopOpen,
  Settings2,
  Square,
  Trash2,
} from "lucide-react";

import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
  CommandShortcut,
} from "@/components/ui/command";
import { useAppStore } from "@/context/app-store";
import { cn } from "@/lib/utils";

type CommandPaletteProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
};

export function CommandPalette({ open, onOpenChange }: CommandPaletteProps) {
  const navigate = useNavigate();
  const { state, runSlashCommand, switchThread } = useAppStore();
  const [search, setSearch] = useState("");

  const activeThread = useMemo(
    () => state.threads.find((thread) => thread.id === state.currentSessionId),
    [state.currentSessionId, state.threads],
  );
  const trimmedSearch = search.trim();
  const lowerSearch = trimmedSearch.toLowerCase();
  const looksLikeCommandQuery =
    /^(new|chat|create|rename|clear|delete|remove|stop|settings|tool|collapse|expand|show|hide|help|session|thread|switch|open|go|coll|del|ren)\b/.test(
      lowerSearch,
    );
  const renamePrefixedTarget = lowerSearch.startsWith("rename ")
    ? trimmedSearch.slice(7).trim()
    : "";
  const newPrefixedName = lowerSearch.startsWith("new ")
    ? trimmedSearch.slice(4).trim()
    : lowerSearch.startsWith("chat ")
      ? trimmedSearch.slice(5).trim()
      : lowerSearch.startsWith("create ")
        ? trimmedSearch.slice(7).trim()
        : "";
  const freeTextTarget =
    trimmedSearch.length > 0 && !looksLikeCommandQuery ? trimmedSearch : "";
  const newChatName = newPrefixedName || freeTextTarget;
  const renameTarget = renamePrefixedTarget || freeTextTarget;

  const handleOpenChange = (nextOpen: boolean): void => {
    onOpenChange(nextOpen);
    if (!nextOpen) {
      setSearch("");
    }
  };

  const closeAndReset = (): void => {
    handleOpenChange(false);
  };

  const runSlash = (
    command: string,
    options?: { navigateTo?: "/" | "/settings" | "/threads" },
  ): void => {
    runSlashCommand(command);
    if (options?.navigateTo) {
      navigate({ to: options.navigateTo });
    }
    closeAndReset();
  };

  return (
    <DialogPrimitive.Root open={open} onOpenChange={handleOpenChange}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="fixed inset-0 z-50 bg-foreground/45 backdrop-blur-[1px]" />
        <DialogPrimitive.Content
          className={cn(
            "fixed left-1/2 top-[14%] z-50 w-[92vw] max-w-[760px] -translate-x-1/2 overflow-hidden rounded-xl border border-border bg-background p-0 shadow-halo outline-none",
          )}
        >
          <DialogPrimitive.Title className="sr-only">
            Command Palette
          </DialogPrimitive.Title>
          <Command shouldFilter className="border-0">
            <CommandInput
              value={search}
              onValueChange={setSearch}
              placeholder="Search commands, sessions, and routes..."
            />
            <CommandList className="scroll-soft max-h-[min(60vh,520px)]">
              <CommandEmpty>No results found.</CommandEmpty>

              <CommandGroup heading="Quick actions">
                <CommandItem
                  value="new chat"
                  keywords={["create", "thread", "session", "start"]}
                  forceMount={Boolean(newChatName)}
                  onSelect={() =>
                    runSlash(newChatName ? `/new ${newChatName}` : "/new", {
                      navigateTo: "/",
                    })
                  }
                >
                  <CirclePlus className="h-4 w-4 shrink-0" />
                  <span className="truncate">
                    {newChatName ? `New chat "${newChatName}"` : "New chat"}
                  </span>
                  <CommandShortcut>Enter</CommandShortcut>
                </CommandItem>

                <CommandItem
                  value="go settings"
                  keywords={["preferences", "config", "options"]}
                  onSelect={() => {
                    navigate({ to: "/settings" });
                    closeAndReset();
                  }}
                >
                  <Settings2 className="h-4 w-4 shrink-0" />
                  <span>Go to settings</span>
                </CommandItem>

                <CommandItem
                  value="slash help"
                  keywords={["commands", "usage"]}
                  onSelect={() => runSlash("/help", { navigateTo: "/" })}
                >
                  <BookOpenText className="h-4 w-4 shrink-0" />
                  <span>Show slash command help</span>
                </CommandItem>
              </CommandGroup>

              <CommandSeparator />

              <CommandGroup heading="Session actions">
                <CommandItem
                  value="rename current session"
                  keywords={["title", "name", "thread"]}
                  forceMount={Boolean(renameTarget)}
                  disabled={!activeThread || !renameTarget}
                  onSelect={() => runSlash(`/rename ${renameTarget}`)}
                >
                  <PanelTopOpen className="h-4 w-4 shrink-0" />
                  <span className="truncate">
                    {renameTarget
                      ? `Rename current session "${renameTarget}"`
                      : "Rename current session"}
                  </span>
                </CommandItem>

                <CommandItem
                  value="clear current context"
                  keywords={["reset", "wipe", "thread"]}
                  disabled={!activeThread}
                  onSelect={() => runSlash("/clear", { navigateTo: "/" })}
                >
                  <Eraser className="h-4 w-4 shrink-0" />
                  <span>Clear current context</span>
                </CommandItem>

                <CommandItem
                  value="delete current session"
                  keywords={["remove", "thread", "destroy"]}
                  disabled={!activeThread}
                  onSelect={() => runSlash("/delete", { navigateTo: "/" })}
                >
                  <Trash2 className="h-4 w-4 shrink-0" />
                  <span>Delete current session</span>
                </CommandItem>

                <CommandItem
                  value="stop response"
                  keywords={["cancel", "interrupt"]}
                  disabled={!state.isWaiting}
                  onSelect={() => runSlash("/stop")}
                >
                  <Square className="h-4 w-4 shrink-0" />
                  <span>Stop active response</span>
                </CommandItem>
              </CommandGroup>

              <CommandSeparator />

              <CommandGroup heading="Tool view">
                <CommandItem
                  value="expand tool outputs"
                  keywords={["tools", "open", "show"]}
                  onSelect={() => runSlash("/tools expand")}
                >
                  <PanelBottomOpen className="h-4 w-4 shrink-0" />
                  <span>Expand all tool outputs</span>
                </CommandItem>
                <CommandItem
                  value="collapse tool outputs"
                  keywords={["tools", "close", "hide"]}
                  onSelect={() => runSlash("/tools collapse")}
                >
                  <PanelTopOpen className="h-4 w-4 shrink-0" />
                  <span>Collapse all tool outputs</span>
                </CommandItem>
                <CommandItem
                  value="show tool call blocks"
                  keywords={["tools", "visible"]}
                  onSelect={() => runSlash("/tools show")}
                >
                  <Eye className="h-4 w-4 shrink-0" />
                  <span>Show tool call blocks</span>
                </CommandItem>
                <CommandItem
                  value="hide tool call blocks"
                  keywords={["tools", "invisible"]}
                  onSelect={() => runSlash("/tools hide")}
                >
                  <EyeOff className="h-4 w-4 shrink-0" />
                  <span>Hide tool call blocks</span>
                </CommandItem>
              </CommandGroup>

              <CommandSeparator />

              <CommandGroup heading="Sessions">
                {state.threads.map((thread, index) => {
                  const active = thread.id === state.currentSessionId;
                  return (
                    <CommandItem
                      key={thread.id}
                      disabled={state.isWaiting}
                      onSelect={() => {
                        switchThread(thread.id);
                        navigate({ to: "/" });
                        closeAndReset();
                      }}
                      value={`${thread.display_name} ${thread.id} ${index + 1}`}
                    >
                      <span className="min-w-0 flex-1 truncate">
                        {thread.display_name}
                      </span>
                      <span className="text-xs text-muted-foreground">
                        {active ? "Current" : `${thread.message_count} msgs`}
                      </span>
                    </CommandItem>
                  );
                })}
              </CommandGroup>
            </CommandList>
          </Command>
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  );
}
