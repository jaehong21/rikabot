import { expect, test } from "@playwright/test";

import { sendFromComposer } from "./helpers";

test("handles thread lifecycle slash commands through backend websocket", async ({
  page,
}) => {
  const suffix = String(Date.now()).slice(-6);
  const createdThread = `e2e-${suffix}`;
  const renamedThread = `ren-${suffix}`;

  await page.goto("/");

  await sendFromComposer(page, `/new ${createdThread}`);
  await expect(page.getByRole("button", { name: createdThread })).toBeVisible();

  await sendFromComposer(page, `/rename ${renamedThread}`);
  await expect(page.getByRole("button", { name: renamedThread })).toBeVisible();

  await sendFromComposer(page, "/clear");
  await expect(page.getByText("Welcome to Rika")).toBeVisible();

  await sendFromComposer(page, "/delete");
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

  await page.getByRole("button", { name: "New chat" }).click();
  await expect(page.getByText("Welcome to Rika")).toBeVisible();
  await expect(page.getByText(prompt, { exact: true })).toHaveCount(0);
});
