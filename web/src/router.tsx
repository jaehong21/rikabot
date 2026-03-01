import {
  createRootRoute,
  createRoute,
  createRouter,
} from "@tanstack/react-router";

import { AppShell } from "@/components/app-shell";
import { ChatPage } from "@/routes/chat-page";
import { SettingsPage } from "@/routes/settings-page";
import { ThreadsPage } from "@/routes/threads-page";

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
