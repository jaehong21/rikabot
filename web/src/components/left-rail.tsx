import { useNavigate } from "@tanstack/react-router";
import { CirclePlus, Search, Settings2 } from "lucide-react";
import { useMemo } from "react";

import { ScrollArea } from "@/components/ui/scroll-area";
import { useAppStore } from "@/context/app-store";
import { cn } from "@/lib/utils";

type LeftRailProps = {
  onNavigate?: () => void;
};

export function LeftRail({ onNavigate }: LeftRailProps) {
  const navigate = useNavigate();
  const { state, createThread, switchThread } = useAppStore();

  const sortedThreads = useMemo(
    () =>
      [...state.threads].sort(
        (left, right) => new Date(right.updated_at).getTime() - new Date(left.updated_at).getTime(),
      ),
    [state.threads],
  );

  const sessions = useMemo(() => sortedThreads, [sortedThreads]);

  return (
    <div className="flex h-full flex-col bg-background px-3 py-3 text-foreground">
      <ScrollArea className="min-h-0 flex-1 pr-1">
        <div className="space-y-4 pb-4">
          <div className="px-2 pt-1 text-xl font-semibold">Rika</div>

          <div className="space-y-1">
            <button
              type="button"
              className="flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-left text-sm text-foreground/90 transition-colors hover:bg-foreground/5 hover:text-foreground"
              onClick={() => {
                createThread();
                onNavigate?.();
                navigate({ to: "/" });
              }}
            >
              <CirclePlus className="h-4 w-4 shrink-0" />
              <span className="truncate">New chat</span>
            </button>

            <button
              type="button"
              className="flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-left text-sm text-foreground/90 transition-colors hover:bg-foreground/5 hover:text-foreground"
            >
              <Search className="h-4 w-4 shrink-0" />
              <span className="truncate">Search</span>
            </button>
          </div>

          <section className="space-y-1">
            <p className="px-2 text-[0.75rem]" style={{ color: "#8b8a84" }}>
              Sessions
            </p>
            {sessions.length === 0 && (
              <p className="px-2 py-1 text-sm text-muted-foreground">No sessions</p>
            )}
            {sessions.map((thread) => {
              const active = thread.id === state.currentSessionId;
              return (
                <button
                  type="button"
                  key={thread.id}
                  onClick={() => {
                    switchThread(thread.id);
                    onNavigate?.();
                    navigate({ to: "/" });
                  }}
                  className={cn(
                    "flex w-full items-center justify-between gap-2 rounded-sm px-2 py-2 text-left text-sm transition-colors",
                    "hover:bg-foreground/5",
                    active ? "bg-foreground/10 text-foreground" : "text-foreground/90",
                  )}
                  disabled={state.isWaiting}
                  title={thread.display_name}
                >
                  <span className="line-clamp-1">{thread.display_name}</span>
                </button>
              );
            })}
          </section>
        </div>
      </ScrollArea>

      <button
        type="button"
        className="flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-left transition-colors hover:bg-foreground/5"
        onClick={() => {
          onNavigate?.();
          navigate({ to: "/settings" });
        }}
      >
        <Settings2 className="h-4 w-4 shrink-0 text-foreground/80" />
        <p className="truncate text-sm font-medium">Settings</p>
      </button>
    </div>
  );
}
