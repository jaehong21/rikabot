import { expect, test } from "@playwright/test";

import { openCommandPalette, runSlash } from "./helpers";

test("navigates settings sections from command palette", async ({ page }) => {
  await page.goto("/");

  await openCommandPalette(page);
  await page.getByText("Go to permissions").click();
  await expect(page).toHaveURL(/\/settings\?section=permissions/);
  await expect(page.getByText("Tool permissions")).toBeVisible();

  await openCommandPalette(page);
  await page.getByText("Go to skills").click();
  await expect(page).toHaveURL(/\/settings\?section=skills/);
  await expect(page.getByText("Skill sources")).toBeVisible();

  await openCommandPalette(page);
  await page.getByText("Go to MCP servers").click();
  await expect(page).toHaveURL(/\/settings\?section=mcp/);
  await expect(page.getByText("MCP server status")).toBeVisible();
});

test("switches active session from command palette", async ({ page }) => {
  const suffix = String(Date.now()).slice(-6);
  const firstThread = `cp-first-${suffix}`;
  const secondThread = `cp-second-${suffix}`;

  await page.goto("/");

  await runSlash(page, `/new ${firstThread}`);
  await runSlash(page, `/new ${secondThread}`);

  await openCommandPalette(page);
  await page
    .getByPlaceholder("Search sessions and routes...")
    .fill(firstThread);
  await page
    .getByRole("dialog")
    .getByText(firstThread, { exact: true })
    .first()
    .click();

  await runSlash(page, `/rename ${firstThread}-active`);

  await expect(
    page.getByRole("button", { name: `${firstThread}-active` }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: secondThread })).toBeVisible();
});

test("opens command palette with keyboard shortcut", async ({ page }) => {
  await page.goto("/");

  await page.keyboard.press("Control+K");
  await expect(
    page.getByPlaceholder("Search sessions and routes..."),
  ).toBeVisible();
});

test("shows empty state when command palette has no matches", async ({
  page,
}) => {
  await page.goto("/");

  await openCommandPalette(page);
  await page
    .getByPlaceholder("Search sessions and routes...")
    .fill("no-results-query-e2e");
  await expect(page.getByText("No results found.")).toBeVisible();
});
