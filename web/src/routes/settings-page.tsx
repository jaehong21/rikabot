import { ShieldCheck, Server, SlidersHorizontal, Sparkles } from 'lucide-react';

import { useAppStore } from '@/context/app-store';
import { formatTime } from '@/lib/utils';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { ScrollArea } from '@/components/ui/scroll-area';
import { Switch } from '@/components/ui/switch';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs';
import { Textarea } from '@/components/ui/textarea';

function mcpStateLabel(state: string): string {
  if (state === 'ready') return 'Ready';
  if (state === 'connecting') return 'Connecting';
  if (state === 'failed') return 'Failed';
  if (state === 'disabled') return 'Disabled';
  return 'Pending';
}

export function SettingsPage() {
  const {
    state,
    setShowToolCalls,
    setToolOutputsExpanded,
    savePermissions,
    updatePermissionsField,
    updatePermissionsEnabled,
  } = useAppStore();

  const readyCount = state.mcpServers.filter((server) => server.state === 'ready').length;

  return (
    <ScrollArea className="h-full">
      <div className="mx-auto w-full max-w-5xl space-y-4 px-3 py-4 md:px-6 md:py-6">
        <Tabs defaultValue="general" className="space-y-4">
          <TabsList className="w-full justify-start overflow-auto">
            <TabsTrigger value="general" className="gap-1.5">
              <SlidersHorizontal className="h-4 w-4" />
              General
            </TabsTrigger>
            <TabsTrigger value="permissions" className="gap-1.5">
              <ShieldCheck className="h-4 w-4" />
              Permissions
            </TabsTrigger>
            <TabsTrigger value="skills" className="gap-1.5">
              <Sparkles className="h-4 w-4" />
              Skills
            </TabsTrigger>
            <TabsTrigger value="mcp" className="gap-1.5">
              <Server className="h-4 w-4" />
              MCP Servers
            </TabsTrigger>
          </TabsList>

          <TabsContent value="general" className="space-y-4">
            <Card>
              <CardHeader>
                <CardTitle>Experience</CardTitle>
                <CardDescription>Control transcript rendering and workspace behavior.</CardDescription>
              </CardHeader>
              <CardContent className="space-y-3">
                <label className="flex items-center justify-between gap-3 rounded-lg border border-border/70 bg-background/70 p-3">
                  <div>
                    <p className="font-medium">Show tool call cards</p>
                    <p className="text-sm text-muted-foreground">Display structured tool activity inline in chat.</p>
                  </div>
                  <Switch checked={state.showToolCalls} onCheckedChange={setShowToolCalls} />
                </label>

                <label className="flex items-center justify-between gap-3 rounded-lg border border-border/70 bg-background/70 p-3">
                  <div>
                    <p className="font-medium">Expand tool outputs by default</p>
                    <p className="text-sm text-muted-foreground">Open each tool card with full arguments and output.</p>
                  </div>
                  <Switch checked={state.toolOutputsExpanded} onCheckedChange={setToolOutputsExpanded} />
                </label>
              </CardContent>
            </Card>
          </TabsContent>

          <TabsContent value="permissions" className="space-y-4">
            <Card>
              <CardHeader>
                <CardTitle>Tool Permissions</CardTitle>
                <CardDescription>
                  Configure allow and deny patterns. Wildcards (`*`) are supported for tool names and argument patterns.
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-3">
                <label className="flex items-center justify-between gap-3 rounded-lg border border-border/70 bg-background/70 p-3">
                  <div>
                    <p className="font-medium">Enable permissions policy</p>
                    <p className="text-sm text-muted-foreground">When disabled, tools run without explicit rule checks.</p>
                  </div>
                  <Switch checked={state.permissionsEnabled} onCheckedChange={updatePermissionsEnabled} />
                </label>

                <div className="grid gap-3 lg:grid-cols-2">
                  <label className="space-y-2">
                    <span className="text-sm font-medium">Allow rules</span>
                    <Textarea
                      rows={10}
                      value={state.permissionsAllowText}
                      onChange={(event) => updatePermissionsField('allow', event.currentTarget.value)}
                      disabled={!state.permissionsLoaded || state.permissionsSaving}
                      placeholder="shell(command:git status *)"
                    />
                  </label>

                  <label className="space-y-2">
                    <span className="text-sm font-medium">Deny rules</span>
                    <Textarea
                      rows={10}
                      value={state.permissionsDenyText}
                      onChange={(event) => updatePermissionsField('deny', event.currentTarget.value)}
                      disabled={!state.permissionsLoaded || state.permissionsSaving}
                      placeholder="shell(command:git push *)"
                    />
                  </label>
                </div>

                {state.permissionsErrors.length > 0 && (
                  <div className="space-y-1 rounded-lg border border-destructive/35 bg-destructive/10 p-3 text-sm text-destructive">
                    {state.permissionsErrors.map((error) => (
                      <p key={error}>{error}</p>
                    ))}
                  </div>
                )}

                <div className="flex flex-wrap items-center gap-3">
                  <Button onClick={savePermissions} disabled={!state.permissionsLoaded || state.permissionsSaving}>
                    {state.permissionsSaving ? 'Saving...' : 'Save permissions'}
                  </Button>
                  {state.permissionsSavedAt && (
                    <span className="text-sm text-muted-foreground">Saved at {formatTime(state.permissionsSavedAt)}</span>
                  )}
                </div>
              </CardContent>
            </Card>
          </TabsContent>

          <TabsContent value="skills" className="space-y-4">
            <Card>
              <CardHeader>
                <CardTitle>Skill Sources</CardTitle>
                <CardDescription>
                  Skills are resolved from workspace `AGENTS.md` and skill directories. This view summarizes available surfaces.
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-3 text-sm">
                <div className="rounded-lg border border-border/70 bg-background/70 p-3">
                  <p className="font-medium">Workspace instructions</p>
                  <p className="text-muted-foreground">`AGENTS.md` defines local guidance and trigger rules for skill usage.</p>
                </div>
                <div className="rounded-lg border border-border/70 bg-background/70 p-3">
                  <p className="font-medium">Installed skill packs</p>
                  <p className="text-muted-foreground">
                    Frontend and behavior skills can be loaded per-turn without backend changes.
                  </p>
                </div>
              </CardContent>
            </Card>
          </TabsContent>

          <TabsContent value="mcp" className="space-y-4">
            <Card>
              <CardHeader>
                <CardTitle>MCP Server Status</CardTitle>
                <CardDescription>
                  Ready {readyCount}/{state.mcpServers.length}. Inspect server reachability, tool counts, and errors.
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-2">
                {!state.mcpEnabled && (
                  <p className="rounded-lg border border-border/70 bg-background/70 p-3 text-sm text-muted-foreground">
                    MCP is disabled in config.
                  </p>
                )}

                {state.mcpEnabled && state.mcpServers.length === 0 && (
                  <p className="rounded-lg border border-border/70 bg-background/70 p-3 text-sm text-muted-foreground">
                    No MCP servers configured.
                  </p>
                )}

                {state.mcpEnabled &&
                  state.mcpServers.map((server) => (
                    <div
                      key={server.name}
                      className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-border/70 bg-background/70 p-3"
                    >
                      <div>
                        <p className="font-medium">{server.name}</p>
                        <p className="text-sm text-muted-foreground">
                          {server.state === 'ready' ? `Tools ${server.tool_count ?? 0}` : server.error || 'No details'}
                        </p>
                      </div>
                      <Badge variant={server.state === 'failed' ? 'destructive' : 'outline'}>{mcpStateLabel(server.state)}</Badge>
                    </div>
                  ))}
              </CardContent>
            </Card>
          </TabsContent>
        </Tabs>
      </div>
    </ScrollArea>
  );
}
