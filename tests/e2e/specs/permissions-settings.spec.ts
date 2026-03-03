import { expect, test } from "@playwright/test";

test("persists permissions rules through frontend and backend", async ({ page }) => {
  const allowRule = `shell(command:echo e2e-${Date.now()} *)`;

  await page.goto("/settings?section=permissions");

  const allowTextarea = page.getByPlaceholder("shell(command:git status *)");
  const denyTextarea = page.getByPlaceholder("shell(command:git push *)");

  await expect(allowTextarea).toBeEnabled();

  await allowTextarea.fill(allowRule);
  await denyTextarea.fill("");

  await page.getByRole("button", { name: "Save permissions" }).click();
  await expect(page.getByText("Saved at")).toBeVisible();

  await page.reload();

  await expect(page.getByPlaceholder("shell(command:git status *)")).toHaveValue(allowRule);
});
