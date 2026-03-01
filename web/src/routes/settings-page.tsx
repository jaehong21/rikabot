import { ChevronLeft } from 'lucide-react';
import { useNavigate } from '@tanstack/react-router';
import { useMemo, useState } from 'react';

import { useAppStore } from '@/context/app-store';
import { formatTime } from '@/lib/utils';
import { Button } from '@/components/ui/button';
import { ScrollArea } from '@/components/ui/scroll-area';
import { Switch } from '@/components/ui/switch';
import { Textarea } from '@/components/ui/textarea';

type SettingsSectionId = 'general' | 'permissions' | 'skills' | 'mcp';

type SettingsSection = {
  id: SettingsSectionId;
  label: string;
};

const SETTINGS_SECTIONS: SettingsSection[] = [
  { id: 'general', label: 'General' },
  { id: 'permissions', label: 'Permissions' },
  { id: 'skills', label: 'Skills' },
  { id: 'mcp', label: 'MCP Servers' },
];

type SectionHeaderProps = {
  title: string;
};

function SectionHeader({ title }: SectionHeaderProps) {
  return (
    <h2 className="text-lg font-semibold tracking-tight">{title}</h2>
  );
}

type SettingRowProps = {
  title: string;
  description: string;
  control?: React.ReactNode;
};

function SettingRow({ title, description, control }: SettingRowProps) {
  return (
    <div className="flex flex-wrap items-start justify-between gap-3 py-3">
      <div className="space-y-1">
        <p className="font-medium">{title}</p>
        <p className="text-sm text-foreground/70">{description}</p>
      </div>
      {control ? <div className="shrink-0 pt-0.5">{control}</div> : null}
    </div>
  );
}

function mcpStateLabel(state: string): string {
  if (state === 'ready') return 'Ready';
  if (state === 'connecting') return 'Connecting';
  if (state === 'failed') return 'Failed';
  if (state === 'disabled') return 'Disabled';
  return 'Pending';
}

export function SettingsPage() {
  const navigate = useNavigate();
  const {
    state,
    setShowToolCalls,
    setToolOutputsExpanded,
    savePermissions,
    updatePermissionsField,
    updatePermissionsEnabled,
  } = useAppStore();
  const [activeSection, setActiveSection] = useState<SettingsSectionId>('general');

  const readyCount = useMemo(
    () => state.mcpServers.filter((server) => server.state === 'ready').length,
    [state.mcpServers],
  );

  return (
    <ScrollArea className="h-full">
      <div className="mx-auto w-full max-w-6xl px-4 py-6 md:px-8 md:py-8">
        <div className="mb-8 flex items-center gap-2">
          <button
            type="button"
            aria-label="Back to chat"
            className="rounded-sm p-1 text-foreground/70 transition-colors hover:bg-foreground/5 hover:text-foreground"
            onClick={() => navigate({ to: '/' })}
          >
            <ChevronLeft className="h-5 w-5" />
          </button>
          <h1 className="text-4xl font-semibold leading-none tracking-tight">Settings</h1>
        </div>

        <div className="grid gap-10 md:grid-cols-[180px_minmax(0,1fr)]">
          <nav className="space-y-1">
            {SETTINGS_SECTIONS.map((section) => {
              const selected = section.id === activeSection;
              return (
                <button
                  key={section.id}
                  type="button"
                  className={[
                    'flex w-full min-w-0 items-center gap-2 overflow-hidden rounded-md px-3 py-2 text-left text-sm transition-colors',
                    'hover:bg-foreground/5',
                    selected ? 'bg-foreground/10 text-foreground' : 'text-foreground/90',
                  ].join(' ')}
                  onClick={() => setActiveSection(section.id)}
                >
                  <span className="truncate">{section.label}</span>
                </button>
              );
            })}
          </nav>

          <section className="space-y-6 pb-8">
            {activeSection === 'general' && (
              <div className="space-y-8">
                <SectionHeader title="General desktop settings" />
                <SettingRow
                  title="Show tool call cards"
                  description="Display structured tool activity inline in chat."
                  control={<Switch checked={state.showToolCalls} onCheckedChange={setShowToolCalls} />}
                />
                <SettingRow
                  title="Expand tool outputs by default"
                  description="Open each tool card with full arguments and output."
                  control={<Switch checked={state.toolOutputsExpanded} onCheckedChange={setToolOutputsExpanded} />}
                />
              </div>
            )}

            {activeSection === 'permissions' && (
              <div className="space-y-6">
                <SectionHeader title="Tool permissions" />
                <SettingRow
                  title="Enable permissions policy"
                  description="When disabled, tools run without explicit rule checks."
                  control={<Switch checked={state.permissionsEnabled} onCheckedChange={updatePermissionsEnabled} />}
                />
                <div className="grid gap-3 py-2 lg:grid-cols-2">
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
                  <div className="space-y-1 py-2 text-sm text-destructive">
                    {state.permissionsErrors.map((error) => (
                      <p key={error}>{error}</p>
                    ))}
                  </div>
                )}

                <div className="flex flex-wrap items-center gap-3 py-2">
                  <Button
                    onClick={savePermissions}
                    disabled={!state.permissionsLoaded || state.permissionsSaving}
                    className="bg-foreground text-input hover:bg-foreground/90"
                  >
                    {state.permissionsSaving ? 'Saving...' : 'Save permissions'}
                  </Button>
                  {state.permissionsSavedAt && (
                    <span className="text-sm text-foreground/70">Saved at {formatTime(state.permissionsSavedAt)}</span>
                  )}
                </div>
              </div>
            )}

            {activeSection === 'skills' && (
              <div className="space-y-6">
                <SectionHeader title="Skill sources" />
                <div className="space-y-1 py-2">
                  <p className="font-medium">Workspace instructions</p>
                  <p className="text-sm text-foreground/70">
                    `AGENTS.md` defines local guidance and trigger rules for skill usage.
                  </p>
                </div>
                <div className="space-y-1 py-2">
                  <p className="font-medium">Installed skill packs</p>
                  <p className="text-sm text-foreground/70">
                    Frontend and behavior skills can be loaded per turn without backend changes.
                  </p>
                </div>
              </div>
            )}

            {activeSection === 'mcp' && (
              <div className="space-y-5">
                <SectionHeader title="MCP server status" />
                <p className="text-sm text-foreground/70">
                  Ready {readyCount}/{state.mcpServers.length}
                </p>

                {!state.mcpEnabled && <p className="py-2 text-sm text-foreground/70">MCP is disabled in config.</p>}

                {state.mcpEnabled && state.mcpServers.length === 0 && (
                  <p className="py-2 text-sm text-foreground/70">No MCP servers configured.</p>
                )}

                {state.mcpEnabled &&
                  state.mcpServers.map((server) => (
                    <div key={server.name} className="flex flex-wrap items-center justify-between gap-2 py-2">
                      <div className="space-y-1">
                        <p className="font-medium">{server.name}</p>
                        <p className="text-sm text-foreground/70">
                          {server.state === 'ready' ? `Tools ${server.tool_count ?? 0}` : server.error || 'No details'}
                        </p>
                      </div>
                      <span className="text-sm text-foreground/70">{mcpStateLabel(server.state)}</span>
                    </div>
                  ))}
              </div>
            )}
          </section>
        </div>
      </div>
    </ScrollArea>
  );
}
