import { expect, test } from "@playwright/test";

import { sendFromComposer } from "./helpers";

test("stops an in-flight response with kill switch", async ({ page }) => {
  const prompt = `e2e-slow:${Date.now()}`;

  await page.goto("/");
  await sendFromComposer(page, prompt);

  const stopButton = page.getByRole("button", { name: "Stop response" });
  await expect(stopButton).toBeVisible();

  await stopButton.click();
  await expect(stopButton).toHaveCount(0);

  await page.waitForTimeout(400);
  await expect(page.getByText(`mock-e2e: ${prompt}`)).toHaveCount(0);
});
