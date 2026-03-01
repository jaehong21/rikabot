import { useMemo, useState } from 'react';
import { Outlet, useRouterState } from '@tanstack/react-router';
import { Menu, PanelLeft } from 'lucide-react';

import { LeftRail } from '@/components/left-rail';
import { useAppStore } from '@/context/app-store';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Sheet, SheetContent, SheetHeader, SheetTitle, SheetTrigger } from '@/components/ui/sheet';

function connectionLabel(state: 'connecting' | 'connected' | 'disconnected'): string {
  if (state === 'connected') return 'Connected';
  if (state === 'connecting') return 'Connecting';
  return 'Disconnected';
}

function connectionTone(state: 'connecting' | 'connected' | 'disconnected'): string {
  if (state === 'connected') return 'bg-primary';
  if (state === 'connecting') return 'bg-foreground/55';
  return 'bg-foreground/30';
}

export function AppShell() {
  const { state, setShowToolCalls } = useAppStore();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const [mobileOpen, setMobileOpen] = useState(false);

  const activeThread = useMemo(
    () => state.threads.find((thread) => thread.id === state.currentSessionId),
    [state.currentSessionId, state.threads],
  );

  const title =
    pathname === '/settings'
      ? 'Settings'
      : pathname === '/threads'
        ? 'Threads'
        : activeThread?.display_name || 'Rika Assistant';

  return (
    <div className="app-noise h-screen w-screen overflow-hidden">
      <div className="relative grid h-full w-full overflow-hidden bg-card/50 md:grid-cols-[290px_1fr]">
        <aside className="hidden border-r border-border/70 md:block">
          <LeftRail />
        </aside>

        <div className="grid min-h-0 grid-rows-[auto_1fr]">
          <header className="flex items-center justify-between gap-3 border-b border-border/70 bg-card/70 px-3 py-2 md:px-5 md:py-3">
            <div className="flex min-w-0 items-center gap-3">
              <Sheet open={mobileOpen} onOpenChange={setMobileOpen}>
                <SheetTrigger asChild>
                  <Button size="icon" variant="outline" className="md:hidden" aria-label="Open navigation">
                    <Menu className="h-4 w-4" />
                  </Button>
                </SheetTrigger>
                <SheetContent side="left" className="w-[86vw] max-w-[360px] p-0">
                  <SheetHeader className="border-b px-4 py-3">
                    <SheetTitle className="display-heading text-xl">Rika</SheetTitle>
                  </SheetHeader>
                  <div className="h-[calc(100vh-68px)]">
                    <LeftRail onNavigate={() => setMobileOpen(false)} />
                  </div>
                </SheetContent>
              </Sheet>

              <div className="min-w-0">
                <p className="display-heading truncate text-xl font-semibold md:text-2xl">{title}</p>
                <p className="text-xs uppercase tracking-[0.14em] text-muted-foreground">Service console</p>
              </div>
            </div>

            <div className="flex shrink-0 items-center gap-2">
              <Badge variant="outline" className="hidden items-center gap-2 rounded-full px-3 py-1 sm:flex">
                <span className={`status-dot ${connectionTone(state.connectionState)}`} />
                {connectionLabel(state.connectionState)}
              </Badge>

              <Button
                variant={state.showToolCalls ? 'secondary' : 'outline'}
                size="sm"
                onClick={() => setShowToolCalls(!state.showToolCalls)}
              >
                <PanelLeft className="h-4 w-4" />
                Tools
              </Button>
            </div>
          </header>

          <main className="min-h-0 overflow-hidden">
            <Outlet />
          </main>
        </div>

        {state.showReconnectOverlay && (
          <div className="absolute inset-0 z-30 flex items-center justify-center bg-background/70 backdrop-blur-sm">
            <div className="rounded-xl border border-border bg-card px-5 py-4 shadow-halo">
              <p className="display-heading text-lg">Reconnecting...</p>
              <p className="mt-1 text-sm text-muted-foreground">Connection dropped. Trying again.</p>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
