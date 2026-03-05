import { expect, test } from "@playwright/test";

import { composer, runSlash, sendFromComposer } from "./helpers";

test("supports slash suggestions and tab completion", async ({ page }) => {
  await page.goto("/");

  const input = composer(page);
  await input.fill("/to");

  await expect(
    page.getByRole("listbox", { name: "Slash command suggestions" }),
  ).toBeVisible();

  await page.keyboard.press("Tab");
  await expect(input).toHaveValue("/tools");
});

test("handles slash command help and error paths", async ({ page }) => {
  await page.goto("/");

  await runSlash(page, "/help");
  await expect(page.getByText("Session commands:")).toBeVisible();

  await runSlash(page, "/rename");
  await expect(
    page.getByText("Error: Usage: /rename <display name>"),
  ).toBeVisible();

  await runSlash(page, "/tools invalid-mode");
  await expect(
    page.getByText("Error: Usage: /tools <collapse|expand|hide|show>"),
  ).toBeVisible();

  await runSlash(page, "/does-not-exist");
  await expect(
    page.getByText("Error: Unknown command: /does-not-exist"),
  ).toBeVisible();
});

test("focuses composer on global slash and completes by Enter before sending", async ({
  page,
}) => {
  await page.goto("/");

  const input = composer(page);
  await page.keyboard.press("/");
  await expect(input).toBeFocused();

  await input.fill("/to");
  await page.keyboard.press("Enter");

  await expect(input).toHaveValue("/tools");
  await page.waitForTimeout(200);
  await expect(page.getByText("mock-e2e: /tools")).toHaveCount(0);
});

test("shows rename suggestions sourced from backend thread state", async ({
  page,
}) => {
  const suffix = String(Date.now()).slice(-6);
  const threadName = `rename-target-${suffix}`;

  await page.goto("/");
  await runSlash(page, `/new ${threadName}`);

  const input = composer(page);
  await input.fill("/rename rename-target");

  await expect(
    page.getByRole("listbox", { name: "Slash command suggestions" }),
  ).toBeVisible();
  await expect(
    page.getByRole("option", { name: new RegExp(`^/rename ${threadName}`) }),
  ).toBeVisible();
});

test("applies /tools modes to chat tool card visibility and expansion", async ({
  page,
}) => {
  const prompt = `e2e-tool-approval:${Date.now()}`;
  const main = page.locator("main");

  await page.goto("/");
  await sendFromComposer(page, prompt);
  await expect(page.getByText("Approval required")).toBeVisible();

  await page.getByRole("button", { name: "Allow once" }).click();
  await expect(page.getByText("Success", { exact: true })).toBeVisible();
  await expect(main.getByText("Input", { exact: true })).toBeVisible();

  await runSlash(page, "/tools hide");
  await expect(main.getByText("Input", { exact: true })).toHaveCount(0);

  await runSlash(page, "/tools show");
  await expect(main.getByText("Input", { exact: true })).toBeVisible();

  await runSlash(page, "/tools collapse");
  await expect(main.getByText("Input", { exact: true })).toHaveCount(0);

  await runSlash(page, "/tools expand");
  await expect(main.getByText("Input", { exact: true })).toBeVisible();
});
