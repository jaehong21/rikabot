import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import { Puzzle, Server, Settings2, ShieldCheck } from "lucide-react";

import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from "@/components/ui/command";
import { useAppStore } from "@/context/app-store";
import { cn } from "@/lib/utils";
import type { SettingsSectionId } from "@/router";

type CommandPaletteProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
};

export function CommandPalette({ open, onOpenChange }: CommandPaletteProps) {
  const navigate = useNavigate();
  const { state } = useAppStore();
  const [search, setSearch] = useState("");

  const handleOpenChange = (nextOpen: boolean): void => {
    onOpenChange(nextOpen);
    if (!nextOpen) {
      setSearch("");
    }
  };

  const closeAndReset = (): void => {
    handleOpenChange(false);
  };

  const navigateSettingsSection = (section?: SettingsSectionId): void => {
    closeAndReset();
    if (section) {
      navigate({ to: "/settings", search: { section } });
      return;
    }
    navigate({ to: "/settings" });
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
              placeholder="Search sessions and routes..."
            />
            <CommandList className="scroll-soft max-h-[min(60vh,520px)]">
              <CommandEmpty>No results found.</CommandEmpty>

              <CommandGroup heading="Navigation">
                <CommandItem
                  value="go settings"
                  keywords={["preferences", "config", "options"]}
                  onSelect={() => {
                    navigateSettingsSection();
                  }}
                >
                  <Settings2 className="h-4 w-4 shrink-0" />
                  <span>Go to settings</span>
                </CommandItem>
                <CommandItem
                  value="go permissions"
                  keywords={["permissions", "policies", "tooling"]}
                  onSelect={() => {
                    navigateSettingsSection("permissions");
                  }}
                >
                  <ShieldCheck className="h-4 w-4 shrink-0" />
                  <span>Go to permissions</span>
                </CommandItem>
                <CommandItem
                  value="go skills"
                  keywords={["skills", "extensions", "plugins", "functions"]}
                  onSelect={() => {
                    navigateSettingsSection("skills");
                  }}
                >
                  <Puzzle className="h-4 w-4 shrink-0" />
                  <span>Go to skills</span>
                </CommandItem>
                <CommandItem
                  value="go mcp servers"
                  keywords={["mcp", "servers", "connectors", "tools"]}
                  onSelect={() => {
                    navigateSettingsSection("mcp");
                  }}
                >
                  <Server className="h-4 w-4 shrink-0" />
                  <span>Go to MCP servers</span>
                </CommandItem>
              </CommandGroup>

              <CommandSeparator />

              <CommandGroup heading="Sessions">
                {state.threads.map((thread, index) => {
                  const active = thread.id === state.currentSessionId;
                  return (
                    <CommandItem
                      key={thread.id}
                      onSelect={() => {
                        navigate({ to: "/", search: { session: thread.id } });
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
