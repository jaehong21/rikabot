import { useMemo, useState } from 'react';
import { Link, useNavigate } from '@tanstack/react-router';
import { Plus, Search, Settings2, Sparkles, Waypoints } from 'lucide-react';

import { useAppStore } from '@/context/app-store';
import { groupThreads } from '@/lib/utils';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { ScrollArea } from '@/components/ui/scroll-area';
import { Separator } from '@/components/ui/separator';
import { cn } from '@/lib/utils';

type LeftRailProps = {
  onNavigate?: () => void;
};

export function LeftRail({ onNavigate }: LeftRailProps) {
  const navigate = useNavigate();
  const { state, createThread, switchThread } = useAppStore();
  const [search, setSearch] = useState('');

  const filteredThreads = useMemo(() => {
    const needle = search.trim().toLowerCase();
    if (!needle) {
      return state.threads;
    }
    return state.threads.filter((thread) => thread.display_name.toLowerCase().includes(needle));
  }, [search, state.threads]);

  const groupedThreads = useMemo(() => groupThreads(filteredThreads), [filteredThreads]);

  const readyServers = state.mcpServers.filter((server) => server.state === 'ready').length;

  return (
    <div className="flex h-full flex-col bg-white/20 p-3 md:p-4">
      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <p className="text-xs font-semibold uppercase tracking-[0.22em] text-muted-foreground">Workspace</p>
          <Badge variant="secondary" className="rounded-sm bg-black/5 text-[10px]">
            Rika
          </Badge>
        </div>

        <Button
          variant="default"
          className="w-full justify-start rounded-lg"
          onClick={() => {
            createThread();
            onNavigate?.();
            navigate({ to: '/' });
          }}
        >
          <Plus className="h-4 w-4" />
          New Thread
        </Button>

        <div className="relative">
          <Search className="pointer-events-none absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
          <Input
            value={search}
            onChange={(event) => setSearch(event.currentTarget.value)}
            className="pl-8"
            placeholder="Search threads"
            aria-label="Search threads"
          />
        </div>
      </div>

      <Separator className="my-3" />

      <ScrollArea className="min-h-0 flex-1 pr-2">
        <div className="space-y-4 pb-4">
          {groupedThreads.length === 0 && (
            <p className="rounded-lg border border-dashed border-border/80 bg-card/70 p-3 text-xs text-muted-foreground">
              No threads found.
            </p>
          )}

          {groupedThreads.map((group) => (
            <section key={group.label} className="space-y-2">
              <h3 className="text-[11px] font-semibold uppercase tracking-[0.18em] text-muted-foreground">
                {group.label}
              </h3>
              <div className="space-y-1.5">
                {group.threads.map((thread) => (
                  <button
                    type="button"
                    key={thread.id}
                    onClick={() => {
                      switchThread(thread.id);
                      onNavigate?.();
                      navigate({ to: '/' });
                    }}
                    className={cn(
                      'w-full rounded-lg border px-3 py-2 text-left text-sm transition-colors',
                      'hover:border-primary/40 hover:bg-primary/5',
                      thread.id === state.currentSessionId
                        ? 'border-primary/50 bg-primary/10 text-foreground shadow-sm'
                        : 'border-border/80 bg-card/75 text-muted-foreground',
                    )}
                    disabled={state.isWaiting}
                    title={thread.display_name}
                  >
                    <div className="line-clamp-1 font-medium text-[13px]">{thread.display_name}</div>
                    <div className="mt-1 text-[11px] uppercase tracking-[0.08em] text-muted-foreground/90">
                      {thread.message_count} msgs
                    </div>
                  </button>
                ))}
              </div>
            </section>
          ))}
        </div>
      </ScrollArea>

      <Separator className="my-3" />

      <div className="space-y-2">
        <Button asChild variant="ghost" className="w-full justify-start">
          <Link to="/" onClick={onNavigate}>
            <Sparkles className="h-4 w-4" />
            Chat
          </Link>
        </Button>
        <Button asChild variant="ghost" className="w-full justify-start">
          <Link to="/threads" onClick={onNavigate}>
            <Waypoints className="h-4 w-4" />
            Threads
          </Link>
        </Button>
        <Button asChild variant="ghost" className="w-full justify-start">
          <Link to="/settings" onClick={onNavigate}>
            <Settings2 className="h-4 w-4" />
            Settings
          </Link>
        </Button>
      </div>

      <p className="mt-4 text-[11px] text-muted-foreground">
        MCP ready {readyServers}/{state.mcpServers.length || 0}
      </p>
    </div>
  );
}
