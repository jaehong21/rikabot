import { expect, test, type Page } from "@playwright/test";

import { sendFromComposer } from "./helpers";

function streamLine(prefix: string, n: number): string {
  return `${prefix} line ${n} detail detail detail detail detail detail`;
}

async function openFreshChat(page: Page): Promise<void> {
  await page.goto("/");
  await page.getByRole("button", { name: "New chat" }).first().click();
  await sendFromComposer(page, "/clear");
  await expect(
    page.getByPlaceholder("How can I help you today?"),
  ).toBeVisible();
}

async function wheelTranscript(page: Page, deltaY: number): Promise<void> {
  const transcript = page.getByTestId("chat-transcript");
  await transcript.hover();
  await page.mouse.wheel(0, deltaY);
}

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

test("shows live elapsed while streaming and restores assistant meta after completion", async ({
  page,
}) => {
  const prompt = `e2e-stream-refresh:${Date.now()}`;
  const expectedAssistant = "stream-refresh done";

  await openFreshChat(page);
  await sendFromComposer(page, prompt);

  const loadingIndicator = page.getByLabel("Loading response");
  const assistantCopyButton = page.getByRole("button", {
    name: "Copy assistant response",
  });
  await expect(loadingIndicator).toBeVisible();
  await expect(loadingIndicator).toContainText(
    /elapsed:\s*[0-9]+\.[0-9]{2}\s*sec/i,
  );
  await expect(assistantCopyButton).toHaveCount(0);

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

  await expect(page.getByText(streamLine("stream-refresh", 4))).toBeVisible({
    timeout: 20_000,
  });
  await expect(assistantCopyButton).toHaveCount(0);

  await expect(page.getByText(expectedAssistant)).toBeVisible();
  await expect(loadingIndicator).toHaveCount(0);

  const finalAssistantCopyButton = assistantCopyButton.last();
  await expect(finalAssistantCopyButton).toBeVisible();
  const assistantMeta = finalAssistantCopyButton.locator("xpath=..");
  const persistedElapsed = assistantMeta.getByText(
    /elapsed:\s*[0-9]+\.[0-9]{2}\s*sec/i,
  );

  await expect(persistedElapsed).toBeVisible();
  await page.waitForTimeout(400);
  await expect(persistedElapsed).toBeVisible();
});

test("autoscroll while streaming pauses on scroll-up, resumes at bottom, and supports overlay jump button", async ({
  page,
}) => {
  const prompt = `e2e-stream-scroll:${Date.now()}`;
  const scrollToBottomButton = page.getByRole("button", {
    name: "Scroll to bottom",
  });

  await openFreshChat(page);
  await sendFromComposer(page, prompt);

  await expect(page.getByLabel("Loading response")).toBeVisible();
  await expect(page.getByText(streamLine("stream-scroll", 120))).toBeVisible({
    timeout: 20_000,
  });
  await expect(scrollToBottomButton).toHaveCount(0);

  await wheelTranscript(page, -120);
  await expect(scrollToBottomButton).toHaveCount(0);

  await wheelTranscript(page, -800);
  await expect(scrollToBottomButton).toBeVisible();

  const duringPauseLine = page
    .getByText(streamLine("stream-scroll", 240))
    .first();
  await expect(duringPauseLine).toBeAttached({ timeout: 20_000 });
  await expect(scrollToBottomButton).toBeVisible();

  await wheelTranscript(page, 5000);
  await wheelTranscript(page, 5000);
  await expect(scrollToBottomButton).toHaveCount(0);

  await expect(page.getByText(streamLine("stream-scroll", 280))).toBeVisible({
    timeout: 20_000,
  });

  await wheelTranscript(page, -800);
  await expect(scrollToBottomButton).toBeVisible();

  await scrollToBottomButton.click();
  await expect(scrollToBottomButton).toHaveCount(0);

  await expect(page.getByText(streamLine("stream-scroll", 310))).toBeVisible({
    timeout: 20_000,
  });
  await expect(page.getByText("stream-scroll done")).toBeVisible({
    timeout: 25_000,
  });
});
