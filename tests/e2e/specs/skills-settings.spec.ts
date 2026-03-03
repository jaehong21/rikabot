import { expect, test } from "@playwright/test";

test("loads and saves workspace skill content", async ({ page }) => {
  const marker = `E2E skill description ${Date.now()}`;

  await page.goto("/settings?section=skills");

  await expect(page.getByText("e2e-skill", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Edit" }).first().click();

  const editor = page.locator("textarea").first();
  const original = await editor.inputValue();
  expect(original).toContain("name: e2e-skill");

  const updated = original.replace(
    "Skill seeded for frontend E2E tests",
    marker,
  );
  await editor.fill(updated);

  await page.getByRole("button", { name: "Save skill" }).click();
  await page.reload();

  await expect(page.getByText("e2e-skill", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Edit" }).first().click();
  await expect(page.locator("textarea").first()).toContainText(marker);
});

test("shows validation error on invalid skill frontmatter", async ({
  page,
}) => {
  await page.goto("/settings?section=skills");

  await expect(page.getByText("e2e-skill", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Edit" }).first().click();

  const editor = page.locator("textarea").first();
  await editor.fill(`---\nname: e2e-skill\n---\n\nBroken content`);

  await page.getByRole("button", { name: "Save skill" }).click();
  await expect(
    page.getByText("skill frontmatter field `description` is required"),
  ).toBeVisible();
});
