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
