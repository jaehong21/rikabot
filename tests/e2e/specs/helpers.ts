import { expect, type Page } from "@playwright/test";

export function composer(page: Page) {
  return page.locator("textarea").first();
}

export async function sendFromComposer(
  page: Page,
  text: string,
): Promise<void> {
  const input = composer(page);
  const sendButton = page.getByRole("button", { name: "Send message" });

  await input.fill(text);
  await expect(sendButton).toBeEnabled();
  await sendButton.click();
}

export async function runSlash(page: Page, command: string): Promise<void> {
  await sendFromComposer(page, command);
}

export async function openCommandPalette(page: Page): Promise<void> {
  await page.getByRole("button", { name: "Search" }).first().click();
  await expect(
    page.getByPlaceholder("Search sessions and routes..."),
  ).toBeVisible();
}
