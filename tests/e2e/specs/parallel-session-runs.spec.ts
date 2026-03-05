import { expect, test } from "@playwright/test";

import {
  openCommandPalette,
  runSlash,
  sendFromComposer,
  waitForApiResponse,
} from "./helpers";

test("runs sessions in parallel and keeps navigation plus slash commands active", async ({
  page,
}) => {
  const suffix = String(Date.now());
  const firstThread = `parallel-a-${suffix}`;
  const secondThread = `parallel-b-${suffix}`;
  const slowPrompt = `e2e-nav-slow:${suffix}`;
  const fastPrompt = `parallel-fast:${suffix}`;

  await page.goto("/");

  await Promise.all([
    waitForApiResponse(page, "POST", "/api/threads"),
    runSlash(page, `/new ${firstThread}`),
  ]);
  await Promise.all([
    waitForApiResponse(page, "POST", "/api/threads"),
    runSlash(page, `/new ${secondThread}`),
  ]);
  await expect(page.getByRole("button", { name: secondThread })).toBeVisible();
  await page.getByRole("button", { name: secondThread }).click();
  await expect(
    page.getByRole("banner").getByText(secondThread, { exact: true }),
  ).toBeVisible();

  await sendFromComposer(page, slowPrompt);
  await expect(page.getByText(slowPrompt, { exact: true })).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Stop response" }),
  ).toBeVisible();

  await page.getByRole("button", { name: firstThread }).click();
  await expect(page).toHaveURL(/\/\?session=/);

  await runSlash(page, "/help");
  await expect(page.getByText("Session commands:")).toBeVisible();

  await sendFromComposer(page, fastPrompt);
  await expect(page.getByText(`mock-e2e: ${fastPrompt}`)).toBeVisible();

  await page.getByRole("button", { name: secondThread }).click();

  await expect(page.getByText(`mock-e2e: ${slowPrompt}`)).toBeVisible();
});

test("allows command palette and thread explorer navigation while another session runs", async ({
  page,
}) => {
  const suffix = String(Date.now());
  const firstThread = `parallel-nav-a-${suffix}`;
  const secondThread = `parallel-nav-b-${suffix}`;
  const slowPrompt = `e2e-slow:${suffix}`;

  await page.goto("/");

  await Promise.all([
    waitForApiResponse(page, "POST", "/api/threads"),
    runSlash(page, `/new ${firstThread}`),
  ]);
  await Promise.all([
    waitForApiResponse(page, "POST", "/api/threads"),
    runSlash(page, `/new ${secondThread}`),
  ]);
  const secondSessionId = new URL(page.url()).searchParams.get("session");
  expect(secondSessionId).toBeTruthy();

  await sendFromComposer(page, slowPrompt);
  await expect(
    page.getByRole("button", { name: "Stop response" }),
  ).toBeVisible();

  await openCommandPalette(page);
  await page
    .getByPlaceholder("Search sessions and routes...")
    .fill(firstThread);
  await page
    .getByRole("dialog")
    .getByText(firstThread, { exact: true })
    .first()
    .click();
  await expect(
    page.getByRole("banner").getByText(firstThread, { exact: true }),
  ).toBeVisible();

  await page.goto("/threads");
  await page.getByRole("button", { name: "Open Thread" }).first().click();
  await expect(page).toHaveURL(/\/\?session=/);

  await page.goto(`/?session=${secondSessionId ?? ""}`);
  await expect(page).toHaveURL(
    new RegExp(`\\/\\?session=${secondSessionId ?? ""}`),
  );

  const stopButton = page.getByRole("button", { name: "Stop response" });
  if (await stopButton.isVisible()) {
    await stopButton.click();
  }
});

test("queues same-session input and auto-runs queued prompt after done", async ({
  page,
}) => {
  const suffix = String(Date.now());
  const firstPrompt = `e2e-queue-slow:${suffix}`;
  const secondPrompt = `queue-after-done:${suffix}`;

  await page.goto("/");

  await sendFromComposer(page, firstPrompt);
  await sendFromComposer(page, secondPrompt);

  await expect(page.getByText("Queued messages (1/5)")).toBeVisible();
  await expect(page.getByText(secondPrompt, { exact: true })).toBeVisible();

  await expect(page.getByText(`mock-e2e: ${firstPrompt}`)).toBeVisible();
  await expect(page.getByText(`mock-e2e: ${secondPrompt}`)).toBeVisible();
  await expect(page.getByText("Queued messages (1/5)")).toHaveCount(0);
});

test("keeps queued input after runtime error until user cancels it", async ({
  page,
}) => {
  const suffix = String(Date.now());
  const failingPrompt = `e2e-error-slow:${suffix}`;
  const queuedPrompt = `queue-after-error:${suffix}`;

  await page.goto("/");

  await sendFromComposer(page, failingPrompt);
  await sendFromComposer(page, queuedPrompt);

  await expect(page.getByText("Queued messages (1/5)")).toBeVisible();
  await expect(page.getByText(queuedPrompt, { exact: true })).toBeVisible();
  await expect(page.getByText("Error: OpenAI API error")).toBeVisible();

  await page.waitForTimeout(400);
  await expect(page.getByText("Queued messages (1/5)")).toBeVisible();
  await expect(page.getByText(`mock-e2e: ${queuedPrompt}`)).toHaveCount(0);

  await page.getByRole("button", { name: "Clear all" }).click();
  await expect(page.getByText("Queued messages (1/5)")).toHaveCount(0);
});

test("clears queued prompts when stop is issued", async ({ page }) => {
  const suffix = String(Date.now());
  const runningPrompt = `e2e-slow:${suffix}`;
  const queuedPrompt = `queue-while-running:${suffix}`;

  await page.goto("/");

  await sendFromComposer(page, runningPrompt);
  await sendFromComposer(page, queuedPrompt);
  await expect(page.getByText("Queued messages (1/5)")).toBeVisible();

  await page.getByRole("button", { name: "Stop response" }).click();
  await expect(page.getByText("Queued messages (1/5)")).toHaveCount(0);
  await expect(page.getByText(`mock-e2e: ${queuedPrompt}`)).toHaveCount(0);
});

test("keeps persisted user/error history across refresh after a failed run", async ({
  page,
}) => {
  const prompt = `e2e-error-slow:refresh-${Date.now()}`;

  await page.goto("/");

  await sendFromComposer(page, prompt);
  await expect(page.getByText(prompt, { exact: true })).toBeVisible();

  await page.reload();
  await expect(page.getByText(prompt, { exact: true })).toBeVisible();
  await expect(page.getByText("Error: OpenAI API error")).toBeVisible();
});

test("reconnects after refresh mid-run and resyncs queued inputs", async ({
  page,
}) => {
  const suffix = String(Date.now());
  const runningPrompt = `e2e-slow:refresh-mid-${suffix}`;
  const queuedPrompt = `queued-after-refresh-${suffix}`;

  await page.goto("/");

  await sendFromComposer(page, runningPrompt);
  await sendFromComposer(page, queuedPrompt);
  await expect(page.getByText(/Queued messages \(\d\/5\)/)).toBeVisible();
  await expect(page.getByText(queuedPrompt, { exact: true })).toBeVisible();

  await page.reload();

  await expect(page.getByText(runningPrompt, { exact: true })).toBeVisible();
  await expect(page.getByText(/Queued messages \(\d\/5\)/)).toBeVisible();
  const stopButton = page.getByRole("button", { name: "Stop response" });
  await expect(stopButton).toBeVisible();
  await stopButton.click();
  await expect(page.getByText(/Queued messages \(\d\/5\)/)).toHaveCount(0);
});

test("keeps chat session query-state across navigation, back/forward, and reload", async ({
  page,
}) => {
  const suffix = String(Date.now());
  const firstThread = `query-a-${suffix}`;
  const secondThread = `query-b-${suffix}`;

  await page.goto("/");

  await runSlash(page, `/new ${firstThread}`);
  await runSlash(page, `/new ${secondThread}`);

  const secondSessionId = new URL(page.url()).searchParams.get("session");
  expect(secondSessionId).toBeTruthy();

  await page.getByRole("button", { name: firstThread }).click();
  await expect(page).toHaveURL(/\/\?session=/);
  const firstSessionId = new URL(page.url()).searchParams.get("session");
  expect(firstSessionId).toBeTruthy();
  expect(firstSessionId).not.toBe(secondSessionId);

  await page.goBack();
  await expect(page).toHaveURL(
    new RegExp(`\\/\\?session=${secondSessionId ?? ""}`),
  );

  await page.reload();
  await expect(page).toHaveURL(
    new RegExp(`\\/\\?session=${secondSessionId ?? ""}`),
  );
});
