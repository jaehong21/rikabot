import { expect, test, type Page } from "@playwright/test";

import { runSlash, sendFromComposer } from "./helpers";

async function ensurePermissionsEnabled(page: Page) {
  await page.goto("/settings?section=permissions");
  const enabledSwitch = page.getByRole("switch").first();
  await expect(enabledSwitch).toBeEnabled();

  const current = await enabledSwitch.getAttribute("aria-checked");
  if (current !== "true") {
    await enabledSwitch.click();
    await page.getByRole("button", { name: "Save permissions" }).click();
    await expect(page.getByText("Saved at")).toBeVisible();
  }

  await page.goto("/");
}

test("handles tool approval flow from denied to allow-once", async ({
  page,
}) => {
  const prompt = `e2e-tool-approval:${Date.now()}`;

  await ensurePermissionsEnabled(page);
  await runSlash(page, `/new approval-allow-${Date.now()}`);
  await sendFromComposer(page, prompt);

  await expect(page.getByText("Approval required")).toBeVisible();
  await expect(
    page.getByText("This tool call was blocked by permissions."),
  ).toBeVisible();

  await page.getByRole("button", { name: "Allow once", exact: true }).click();

  await expect(page.getByText("Success", { exact: true })).toBeVisible();
  await expect(
    page.getByText("e2e-tool-approved", { exact: true }),
  ).toBeVisible();
  await expect(page.getByText("Approval required")).toHaveCount(0);
});

test("supports denying a pending tool call", async ({ page }) => {
  const prompt = `e2e-tool-approval:${Date.now()}`;

  await ensurePermissionsEnabled(page);
  await runSlash(page, `/new approval-deny-${Date.now()}`);
  await sendFromComposer(page, prompt);

  await expect(page.getByText("Approval required")).toBeVisible();
  await page.getByRole("button", { name: "Deny", exact: true }).click();

  await expect(page.getByText("Denied", { exact: true })).toBeVisible();
  await expect(page.getByText("Approval required")).toHaveCount(0);
});
