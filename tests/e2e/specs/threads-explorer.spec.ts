import { expect, test } from "@playwright/test";

import { runSlash, waitForApiResponse } from "./helpers";

test("switches thread from thread explorer route", async ({ page }) => {
  const suffix = String(Date.now()).slice(-6);
  const firstThread = `explorer-a-${suffix}`;
  const secondThread = `explorer-b-${suffix}`;
  const renamed = `${firstThread}-selected`;

  await page.goto("/");

  await runSlash(page, `/new ${firstThread}`);
  await runSlash(page, `/new ${secondThread}`);

  await page.goto("/threads");

  const targetCard = page.locator("main article, main div").filter({
    hasText: firstThread,
  });
  await Promise.all([
    waitForApiResponse(page, "GET", "/api/threads/"),
    targetCard.getByRole("button", { name: "Open Thread" }).first().click(),
  ]);

  await page.goto("/");
  await runSlash(page, `/rename ${renamed}`);

  await expect(page.getByRole("button", { name: renamed })).toBeVisible();
  await expect(page.getByRole("button", { name: secondThread })).toBeVisible();
});
