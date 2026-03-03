import { expect, type Page } from "@playwright/test";

export async function sendFromComposer(
  page: Page,
  text: string,
): Promise<void> {
  const composer = page.locator("textarea").first();
  const sendButton = page.getByRole("button", { name: "Send message" });

  await composer.fill(text);
  await expect(sendButton).toBeEnabled();
  await sendButton.click();
}
