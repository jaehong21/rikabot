import { expect, test } from "@playwright/test";

test("keeps settings section query-state across navigation and reload", async ({
  page,
}) => {
  await page.goto("/settings?section=skills");

  await expect(page.getByText("Skill sources")).toBeVisible();

  await page
    .locator("nav")
    .getByRole("button", { name: "MCP Servers" })
    .click();
  await expect(page).toHaveURL(/\/settings\?section=mcp/);

  await page.reload();

  await expect(page).toHaveURL(/\/settings\?section=mcp/);
  await expect(page.getByText("MCP server status")).toBeVisible();
});

test("falls back to general section for invalid query-state", async ({
  page,
}) => {
  await page.goto("/settings?section=unknown");

  await expect(page.getByText("Show tool call cards")).toBeVisible();
});

test("navigates back to chat from settings header action", async ({ page }) => {
  await page.goto("/settings?section=permissions");

  await page.getByRole("button", { name: "Back to chat" }).click();
  await expect(page).toHaveURL("/");
  await expect(page.getByText("Welcome to Rika")).toBeVisible();
});
