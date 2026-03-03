import { expect, test } from "@playwright/test";

import { sendFromComposer } from "./helpers";

test("renders assistant response from real backend agent flow", async ({
  page,
}) => {
  const userPrompt = `e2e-message-${Date.now()}`;
  const expectedAssistant = `mock-e2e: ${userPrompt}`;

  await page.goto("/");

  await sendFromComposer(page, userPrompt);

  await expect(page.getByText(userPrompt, { exact: true })).toBeVisible();
  await expect(page.getByText(expectedAssistant)).toBeVisible();
});
