import { expect, test } from "@playwright/test";

import { sendFromComposer, waitForApiResponse } from "./helpers";

test("handles thread lifecycle slash commands through REST CRUD endpoints", async ({
  page,
}) => {
  const suffix = String(Date.now()).slice(-6);
  const createdThread = `e2e-${suffix}`;
  const renamedThread = `ren-${suffix}`;

  await page.goto("/");

  await Promise.all([
    waitForApiResponse(page, "POST", "/api/threads"),
    sendFromComposer(page, `/new ${createdThread}`),
  ]);
  await expect(page.getByRole("button", { name: createdThread })).toBeVisible();

  await Promise.all([
    waitForApiResponse(page, "PATCH", "/api/threads/"),
    sendFromComposer(page, `/rename ${renamedThread}`),
  ]);
  await expect(page.getByRole("button", { name: renamedThread })).toBeVisible();

  await Promise.all([
    waitForApiResponse(page, "DELETE", "/api/threads/"),
    sendFromComposer(page, "/clear"),
  ]);
  await expect(page.getByText("Welcome to Rika")).toBeVisible();

  await Promise.all([
    waitForApiResponse(page, "DELETE", "/api/threads/"),
    sendFromComposer(page, "/delete"),
  ]);
  await expect(page.getByRole("button", { name: renamedThread })).toHaveCount(
    0,
  );
});

test("creates and switches to a new thread from left rail action", async ({
  page,
}) => {
  const prompt = `left-rail-${Date.now()}`;

  await page.goto("/");
  await sendFromComposer(page, prompt);
  await expect(page.getByText(prompt, { exact: true })).toBeVisible();

  await Promise.all([
    waitForApiResponse(page, "POST", "/api/threads"),
    page.getByRole("button", { name: "New chat" }).click(),
  ]);
  await expect(page.getByText("Welcome to Rika")).toBeVisible();
  await expect(page.getByText(prompt, { exact: true })).toHaveCount(0);
});
