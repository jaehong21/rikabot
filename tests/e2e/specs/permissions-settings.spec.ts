import { expect, test } from "@playwright/test";
import { waitForApiResponse } from "./helpers";

test("persists permissions rules through frontend and backend", async ({
  page,
}) => {
  const allowRule = `shell(command:echo e2e-${Date.now()} *)`;

  await page.goto("/settings?section=permissions");

  const allowTextarea = page.getByPlaceholder("shell(command:git status *)");
  const denyTextarea = page.getByPlaceholder("shell(command:git push *)");

  await expect(allowTextarea).toBeEnabled();

  await allowTextarea.fill(allowRule);
  await denyTextarea.fill("");

  await Promise.all([
    waitForApiResponse(page, "PUT", "/api/settings/permissions"),
    page.getByRole("button", { name: "Save permissions" }).click(),
  ]);
  await expect(page.getByText("Saved at")).toBeVisible();

  await page.reload();

  await expect(
    page.getByPlaceholder("shell(command:git status *)"),
  ).toHaveValue(allowRule);
});

test("shows validation errors for malformed permission rules", async ({
  page,
}) => {
  await page.goto("/settings?section=permissions");

  const allowTextarea = page.getByPlaceholder("shell(command:git status *)");

  await allowTextarea.fill("not-a-valid-rule");
  await Promise.all([
    waitForApiResponse(page, "PUT", "/api/settings/permissions"),
    page.getByRole("button", { name: "Save permissions" }).click(),
  ]);
  await expect(page.getByText(/invalid rule/i)).toBeVisible();

  await allowTextarea.fill("shell(command:echo fixed *)");
  await Promise.all([
    waitForApiResponse(page, "PUT", "/api/settings/permissions"),
    page.getByRole("button", { name: "Save permissions" }).click(),
  ]);
  await expect(page.getByText("Saved at")).toBeVisible();
  await expect(page.getByText(/invalid rule/i)).toHaveCount(0);
});

test("persists permissions enabled switch and restores original value", async ({
  page,
}) => {
  await page.goto("/settings?section=permissions");

  const enabledSwitch = page.getByRole("switch").first();
  const original = await enabledSwitch.getAttribute("aria-checked");
  const target = original === "true" ? "false" : "true";

  if (target !== original) {
    await enabledSwitch.click();
  }
  await Promise.all([
    waitForApiResponse(page, "PUT", "/api/settings/permissions"),
    page.getByRole("button", { name: "Save permissions" }).click(),
  ]);
  await expect(page.getByText("Saved at")).toBeVisible();
  await expect(enabledSwitch).toHaveAttribute("aria-checked", target);

  await page.reload();
  await expect(page.getByRole("switch").first()).toHaveAttribute(
    "aria-checked",
    target,
  );

  const restoredSwitch = page.getByRole("switch").first();
  const current = await restoredSwitch.getAttribute("aria-checked");
  if (current !== original) {
    await restoredSwitch.click();
  }
  await Promise.all([
    waitForApiResponse(page, "PUT", "/api/settings/permissions"),
    page.getByRole("button", { name: "Save permissions" }).click(),
  ]);
  await expect(restoredSwitch).toHaveAttribute(
    "aria-checked",
    original ?? "true",
  );
});
