import {
  createRootRoute,
  createRoute,
  createRouter,
} from "@tanstack/react-router";

import { AppShell } from "@/components/app-shell";
import { ChatPage } from "@/routes/chat-page";
import { SettingsPage } from "@/routes/settings-page";
import { ThreadsPage } from "@/routes/threads-page";

export const SETTINGS_SECTION_IDS = [
  "general",
  "permissions",
  "skills",
  "mcp",
] as const;

export type SettingsSectionId = (typeof SETTINGS_SECTION_IDS)[number];

export type SettingsSearch = {
  section?: SettingsSectionId;
};

function normalizeSettingsSearch(
  search: Record<string, unknown>,
): SettingsSearch {
  const section = search.section;
  if (
    typeof section === "string" &&
    SETTINGS_SECTION_IDS.includes(section as SettingsSectionId)
  ) {
    return { section: section as SettingsSectionId };
  }
  return {};
}

const rootRoute = createRootRoute({
  component: AppShell,
});

const chatRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: ChatPage,
});

const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings",
  validateSearch: normalizeSettingsSearch,
  component: SettingsPage,
});

const threadsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/threads",
  component: ThreadsPage,
});

const routeTree = rootRoute.addChildren([
  chatRoute,
  settingsRoute,
  threadsRoute,
]);

export const router = createRouter({
  routeTree,
  defaultPreload: "intent",
  scrollRestoration: true,
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
