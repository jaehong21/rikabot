import { expect, test } from "@playwright/test";

test("persists general toggle preferences across reload", async ({ page }) => {
  await page.goto("/settings?section=general");

  const showToolCallsSwitch = page.getByRole("switch").nth(0);
  const expandToolOutputsSwitch = page.getByRole("switch").nth(1);

  if ((await showToolCallsSwitch.getAttribute("aria-checked")) === "true") {
    await showToolCallsSwitch.click();
  }
  if ((await expandToolOutputsSwitch.getAttribute("aria-checked")) === "true") {
    await expandToolOutputsSwitch.click();
  }

  await expect(showToolCallsSwitch).toHaveAttribute("aria-checked", "false");
  await expect(expandToolOutputsSwitch).toHaveAttribute(
    "aria-checked",
    "false",
  );

  await page.reload();

  await expect(page.getByRole("switch").nth(0)).toHaveAttribute(
    "aria-checked",
    "false",
  );
  await expect(page.getByRole("switch").nth(1)).toHaveAttribute(
    "aria-checked",
    "false",
  );
});
