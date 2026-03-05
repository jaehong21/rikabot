import { expect, test } from "@playwright/test";

import { sendFromComposer } from "./helpers";

function parseElapsedSeconds(raw: string | null): number | null {
  if (!raw) {
    return null;
  }

  const match = raw.match(/elapsed:\s*([0-9]+\.[0-9]{2})\s*sec/i);
  if (!match) {
    return null;
  }

  const value = Number.parseFloat(match[1] ?? "");
  return Number.isFinite(value) ? value : null;
}

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

test("shows live elapsed timer while loading and persists elapsed above assistant copy after completion", async ({
  page,
}) => {
  const prompt = `e2e-nav-slow:elapsed-${Date.now()}`;
  const expectedAssistant = `mock-e2e: ${prompt}`;

  await page.goto("/");
  await sendFromComposer(page, prompt);

  const loadingIndicator = page.getByLabel("Loading response");
  await expect(loadingIndicator).toBeVisible();
  await expect(loadingIndicator).toContainText(
    /elapsed:\s*[0-9]+\.[0-9]{2}\s*sec/i,
  );

  await expect
    .poll(
      async () =>
        parseElapsedSeconds(await loadingIndicator.textContent()) ?? -1,
      { timeout: 8_000 },
    )
    .toBeGreaterThanOrEqual(0);

  const firstLiveElapsed =
    parseElapsedSeconds(await loadingIndicator.textContent()) ?? 0;
  await page.waitForTimeout(700);
  const secondLiveElapsed =
    parseElapsedSeconds(await loadingIndicator.textContent()) ?? 0;
  expect(secondLiveElapsed).toBeGreaterThan(firstLiveElapsed);

  await expect(page.getByText(expectedAssistant)).toBeVisible();
  await expect(loadingIndicator).toHaveCount(0);

  const assistantCopyButton = page
    .getByRole("button", { name: "Copy assistant response" })
    .last();
  const assistantMeta = assistantCopyButton.locator("xpath=..");
  const persistedElapsed = assistantMeta.getByText(
    /elapsed:\s*[0-9]+\.[0-9]{2}\s*sec/i,
  );

  await expect(persistedElapsed).toBeVisible();
  await page.waitForTimeout(400);
  await expect(persistedElapsed).toBeVisible();
});
