import { expect, test } from "@playwright/test";

test("renders disabled MCP state from backend config", async ({ page }) => {
  await page.goto("/settings?section=mcp");

  await expect(page.getByText("MCP server status")).toBeVisible();
  await expect(page.getByText("Ready 0/0")).toBeVisible();
  await expect(page.getByText("MCP is disabled in config.")).toBeVisible();
});
