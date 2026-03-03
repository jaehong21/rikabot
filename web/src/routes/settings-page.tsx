import { ChevronLeft, ChevronRight, RefreshCw } from "lucide-react";
import { useNavigate, useSearch } from "@tanstack/react-router";
import { useEffect, useMemo, useState } from "react";

import type { SettingsSectionId } from "@/router";
import { useAppStore } from "@/context/app-store";
import { formatTime } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";

type SettingsSection = {
  id: SettingsSectionId;
  label: string;
};

const SETTINGS_SECTIONS: SettingsSection[] = [
  { id: "general", label: "General" },
  { id: "permissions", label: "Permissions" },
  { id: "skills", label: "Skills" },
  { id: "mcp", label: "MCP Servers" },
];

const SETTINGS_BUTTON_RADIUS_CLASS = "rounded-[0.55rem]";
const SETTINGS_SECONDARY_BUTTON_CLASS = [
  SETTINGS_BUTTON_RADIUS_CLASS,
  "h-9 border border-border bg-input px-4 text-foreground hover:bg-background",
].join(" ");

type SectionHeaderProps = {
  title: string;
};

function SectionHeader({ title }: SectionHeaderProps) {
  return <h2 className="text-lg font-semibold tracking-tight">{title}</h2>;
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
  if (state === "ready") return "Ready";
  if (state === "connecting") return "Connecting";
  if (state === "failed") return "Failed";
  if (state === "disabled") return "Disabled";
  return "Pending";
}

export function SettingsPage() {
  const navigate = useNavigate();
  const search = useSearch({ from: "/settings" });
  const {
    state,
    setShowToolCalls,
    setToolOutputsExpanded,
    savePermissions,
    updatePermissionsField,
    updatePermissionsEnabled,
    refreshSkills,
    loadSkillContent,
    saveSkill,
  } = useAppStore();
  const activeSection = search.section ?? "general";
  const [editingSkillPath, setEditingSkillPath] = useState<string | null>(null);
  const [editorContent, setEditorContent] = useState("");
  const [editorDirty, setEditorDirty] = useState(false);
  const [isRefreshingSkills, setIsRefreshingSkills] = useState(false);
  const [expandedMcpServers, setExpandedMcpServers] = useState<
    Record<string, boolean>
  >({});

  const readyCount = useMemo(
    () => state.mcpServers.filter((server) => server.state === "ready").length,
    [state.mcpServers],
  );

  useEffect(() => {
    if (activeSection === "skills" && !state.skillsLoaded) {
      refreshSkills();
    }
  }, [activeSection, refreshSkills, state.skillsLoaded]);

  useEffect(() => {
    if (
      editingSkillPath &&
      !state.skills.some((skill) => skill.path === editingSkillPath)
    ) {
      setEditingSkillPath(null);
      setEditorContent("");
      setEditorDirty(false);
    }
  }, [editingSkillPath, state.skills]);

  useEffect(() => {
    if (!editingSkillPath || editorDirty) {
      return;
    }
    const content = state.skillContentByPath[editingSkillPath];
    if (typeof content === "string") {
      setEditorContent(content);
    }
  }, [editingSkillPath, editorDirty, state.skillContentByPath]);

  const setActiveSection = (section: SettingsSectionId): void => {
    navigate({ to: "/settings", search: { section } });
  };

  const toggleSkillEditor = (path: string): void => {
    if (editingSkillPath === path) {
      setEditingSkillPath(null);
      setEditorContent("");
      setEditorDirty(false);
      return;
    }

    setEditingSkillPath(path);
    setEditorDirty(false);
    const cached = state.skillContentByPath[path];
    if (typeof cached === "string") {
      setEditorContent(cached);
      return;
    }
    setEditorContent("");
    loadSkillContent(path);
  };

  const saveEditingSkill = (): void => {
    if (!editingSkillPath) {
      return;
    }
    saveSkill(editingSkillPath, editorContent);
    setEditorDirty(false);
  };

  const toggleMcpServerExpanded = (name: string): void => {
    setExpandedMcpServers((prev) => ({
      ...prev,
      [name]: !prev[name],
    }));
  };

  const triggerSkillsRefresh = (): void => {
    if (isRefreshingSkills) {
      return;
    }
    setIsRefreshingSkills(true);
    refreshSkills();
    window.setTimeout(() => {
      setIsRefreshingSkills(false);
    }, 700);
  };

  return (
    <ScrollArea className="h-full">
      <div className="mx-auto w-full max-w-6xl px-4 py-6 md:px-8 md:py-8">
        <div className="mb-8 flex items-center gap-2">
          <button
            type="button"
            aria-label="Back to chat"
            className="rounded-sm p-1 text-foreground/70 transition-colors hover:bg-foreground/5 hover:text-foreground"
            onClick={() => navigate({ to: "/" })}
          >
            <ChevronLeft className="h-5 w-5" />
          </button>
          <h1 className="text-4xl font-semibold leading-none tracking-tight">
            Settings
          </h1>
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
                    `flex w-full min-w-0 items-center gap-2 overflow-hidden ${SETTINGS_BUTTON_RADIUS_CLASS} px-3 py-2 text-left text-sm transition-colors`,
                    "hover:bg-foreground/5",
                    selected
                      ? "bg-foreground/10 text-foreground"
                      : "text-foreground/90",
                  ].join(" ")}
                  onClick={() => setActiveSection(section.id)}
                >
                  <span className="truncate">{section.label}</span>
                </button>
              );
            })}
          </nav>

          <section className="space-y-6 pb-8">
            {activeSection === "general" && (
              <div className="space-y-8">
                <SectionHeader title="General desktop settings" />
                <SettingRow
                  title="Show tool call cards"
                  description="Display structured tool activity inline in chat."
                  control={
                    <Switch
                      checked={state.showToolCalls}
                      onCheckedChange={setShowToolCalls}
                    />
                  }
                />
                <SettingRow
                  title="Expand tool outputs by default"
                  description="Open each tool card with full arguments and output."
                  control={
                    <Switch
                      checked={state.toolOutputsExpanded}
                      onCheckedChange={setToolOutputsExpanded}
                    />
                  }
                />
              </div>
            )}

            {activeSection === "permissions" && (
              <div className="space-y-6">
                <SectionHeader title="Tool permissions" />
                <SettingRow
                  title="Enable permissions policy"
                  description="When disabled, tools run without explicit rule checks."
                  control={
                    <Switch
                      checked={state.permissionsEnabled}
                      onCheckedChange={updatePermissionsEnabled}
                    />
                  }
                />
                <div className="grid gap-3 py-2 lg:grid-cols-2">
                  <label className="space-y-2">
                    <span className="text-sm font-medium">Allow rules</span>
                    <Textarea
                      rows={10}
                      value={state.permissionsAllowText}
                      onChange={(event) =>
                        updatePermissionsField(
                          "allow",
                          event.currentTarget.value,
                        )
                      }
                      disabled={
                        !state.permissionsLoaded || state.permissionsSaving
                      }
                      placeholder="shell(command:git status *)"
                    />
                  </label>

                  <label className="space-y-2">
                    <span className="text-sm font-medium">Deny rules</span>
                    <Textarea
                      rows={10}
                      value={state.permissionsDenyText}
                      onChange={(event) =>
                        updatePermissionsField(
                          "deny",
                          event.currentTarget.value,
                        )
                      }
                      disabled={
                        !state.permissionsLoaded || state.permissionsSaving
                      }
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
                    disabled={
                      !state.permissionsLoaded || state.permissionsSaving
                    }
                    className={`${SETTINGS_BUTTON_RADIUS_CLASS} bg-foreground text-input hover:bg-foreground/90`}
                  >
                    {state.permissionsSaving ? "Saving..." : "Save permissions"}
                  </Button>
                  {state.permissionsSavedAt && (
                    <span className="text-sm text-foreground/70">
                      Saved at {formatTime(state.permissionsSavedAt)}
                    </span>
                  )}
                </div>
              </div>
            )}

            {activeSection === "skills" && (
              <div className="space-y-6">
                <div className="flex flex-wrap items-center gap-2">
                  <SectionHeader title="Skill sources" />
                  <button
                    type="button"
                    aria-label="Refresh skills"
                    onClick={triggerSkillsRefresh}
                    className={`${SETTINGS_BUTTON_RADIUS_CLASS} p-1 text-foreground/70 transition-colors hover:bg-foreground/5 hover:text-foreground`}
                  >
                    <RefreshCw
                      className={[
                        "h-4 w-4",
                        isRefreshingSkills ? "animate-spin" : "",
                      ].join(" ")}
                    />
                  </button>
                </div>
                <div className="space-y-1 py-2">
                  <p className="font-medium">Workspace instructions</p>
                  <p className="text-sm text-foreground/70">
                    `AGENTS.md` defines local guidance and trigger rules for
                    skill usage.
                  </p>
                </div>

                {!state.skillsEnabled && (
                  <p className="py-2 text-sm text-foreground/70">
                    Skills are disabled in config.
                  </p>
                )}

                {state.skillsEnabled && state.skills.length === 0 && (
                  <p className="py-2 text-sm text-foreground/70">
                    No skills found in workspace.
                  </p>
                )}

                {state.skillsEnabled && state.skills.length > 0 && (
                  <div className="divide-y divide-border">
                    {state.skills.map((skill) => {
                      const isEditing = skill.path === editingSkillPath;
                      return (
                        <div key={skill.path} className="w-full py-4 text-left">
                          <div className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-4">
                            <div className="min-w-0 space-y-1">
                              <div className="flex items-center gap-2">
                                <p className="font-medium">{skill.name}</p>
                                <span className="text-xs text-foreground/50">
                                  {skill.available
                                    ? "Available"
                                    : "Unavailable"}
                                </span>
                              </div>
                              <p className="text-sm text-foreground/70">
                                {skill.description || "No description"}
                              </p>
                              <p className="text-xs text-foreground/50">
                                {skill.always ? "Always-loaded" : "On-demand"} ·{" "}
                                {skill.path}
                              </p>
                              {!skill.available &&
                                Array.isArray(skill.missing) &&
                                skill.missing.length > 0 && (
                                  <p className="text-xs text-foreground/50">
                                    Missing: {skill.missing.join(", ")}
                                  </p>
                                )}
                            </div>
                            <Button
                              onClick={() => toggleSkillEditor(skill.path)}
                              className={SETTINGS_SECONDARY_BUTTON_CLASS}
                            >
                              Edit
                            </Button>
                          </div>
                          {isEditing && (
                            <div className="mt-3 space-y-3">
                              <Textarea
                                rows={14}
                                value={editorContent}
                                onChange={(event) => {
                                  setEditorDirty(true);
                                  setEditorContent(event.currentTarget.value);
                                }}
                                placeholder={
                                  '---\nname: sample\ndescription: "..." \n---'
                                }
                              />
                              {state.skillContentLoadingPath === skill.path && (
                                <p className="text-xs text-foreground/50">
                                  Loading skill file...
                                </p>
                              )}
                              {state.skillsErrors.length > 0 && (
                                <div className="space-y-1 text-sm text-destructive">
                                  {state.skillsErrors.map((error) => (
                                    <p key={error}>{error}</p>
                                  ))}
                                </div>
                              )}
                              <div className="flex flex-wrap items-center gap-3">
                                <Button
                                  onClick={saveEditingSkill}
                                  disabled={
                                    state.skillsSaving || !editorContent.trim()
                                  }
                                  className={`${SETTINGS_BUTTON_RADIUS_CLASS} bg-foreground text-input hover:bg-foreground/90`}
                                >
                                  {state.skillsSaving
                                    ? "Saving..."
                                    : "Save skill"}
                                </Button>
                              </div>
                            </div>
                          )}
                        </div>
                      );
                    })}
                  </div>
                )}
              </div>
            )}

            {activeSection === "mcp" && (
              <div className="space-y-5">
                <SectionHeader title="MCP server status" />
                <p className="text-sm text-foreground/70">
                  Ready {readyCount}/{state.mcpServers.length}
                </p>

                {!state.mcpEnabled && (
                  <p className="py-2 text-sm text-foreground/70">
                    MCP is disabled in config.
                  </p>
                )}

                {state.mcpEnabled && state.mcpServers.length === 0 && (
                  <p className="py-2 text-sm text-foreground/70">
                    No MCP servers configured.
                  </p>
                )}

                {state.mcpEnabled &&
                  (state.mcpServers.length > 0 ? (
                    <div className="divide-y divide-border">
                      {state.mcpServers.map((server) => {
                        const isExpanded = Boolean(
                          expandedMcpServers[server.name],
                        );
                        return (
                          <div key={server.name} className="py-4">
                            <button
                              type="button"
                              aria-label={
                                isExpanded
                                  ? `Collapse ${server.name} tools`
                                  : `Expand ${server.name} tools`
                              }
                              className={`${SETTINGS_BUTTON_RADIUS_CLASS} flex w-full flex-wrap items-center justify-between gap-2 p-1 text-left`}
                              onClick={() =>
                                toggleMcpServerExpanded(server.name)
                              }
                            >
                              <div className="space-y-1">
                                <p className="font-medium">{server.name}</p>
                                <p className="text-sm text-foreground/70">
                                  Tools{" "}
                                  {server.tool_count ??
                                    server.tools?.length ??
                                    0}
                                </p>
                              </div>
                              <div className="flex items-center gap-2">
                                <span className="text-sm text-foreground/70">
                                  {mcpStateLabel(server.state)}
                                </span>
                                <ChevronRight
                                  className={[
                                    "h-4 w-4 text-foreground/70 transition-transform",
                                    isExpanded ? "rotate-90" : "",
                                  ].join(" ")}
                                />
                              </div>
                            </button>

                            {isExpanded && (
                              <div className="mt-3">
                                {server.state !== "ready" && (
                                  <p className="text-sm text-foreground/70">
                                    {server.error || "No details"}
                                  </p>
                                )}

                                {server.state === "ready" &&
                                  (server.tools?.length ? (
                                    <div className="space-y-2">
                                      {server.tools.map((tool) => (
                                        <div
                                          key={tool.name}
                                          className="space-y-0.5"
                                        >
                                          <p className="text-sm font-medium">
                                            {tool.name}
                                          </p>
                                          {tool.description && (
                                            <p className="text-xs text-foreground/50">
                                              {tool.description}
                                            </p>
                                          )}
                                        </div>
                                      ))}
                                    </div>
                                  ) : (
                                    <p className="text-sm text-foreground/70">
                                      No tools reported.
                                    </p>
                                  ))}
                              </div>
                            )}
                          </div>
                        );
                      })}
                    </div>
                  ) : null)}
              </div>
            )}
          </section>
        </div>
      </div>
    </ScrollArea>
  );
}
