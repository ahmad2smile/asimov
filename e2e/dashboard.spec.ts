import { expect, test } from "@playwright/test";

const rows = (page: import("@playwright/test").Page) => page.locator("tbody tr");

test.beforeEach(async ({ page }) => {
  await page.goto("/");
  await expect(page.getByText("Live")).toBeVisible();
});

test("shows the AGV list of the first map", async ({ page }) => {
  await expect(rows(page)).toHaveCount(10);
  await expect(rows(page).first()).toContainText("AGV-01");
  await expect(rows(page).first()).toContainText("Berlin");
  await expect(page.getByText("1–10")).toBeVisible();
});

test("pages through the AGVs", async ({ page }) => {
  await expect(rows(page)).toHaveCount(10);
  await page.getByRole("button", { name: "Next page" }).click();
  await expect(rows(page)).toHaveCount(2);
  await expect(rows(page).first()).toContainText("AGV-11");
  await expect(page.getByRole("button", { name: "Next page" })).toBeDisabled();

  await page.getByRole("button", { name: "Previous page" }).click();
  await expect(rows(page)).toHaveCount(10);
});

test("searches AGVs by name", async ({ page }) => {
  await page.getByPlaceholder("Search AGVs").fill("acme/AGV-02");
  await expect(rows(page)).toHaveCount(1);
  await expect(rows(page).first()).toContainText("Obstacle detected");
});

test("switches map", async ({ page }) => {
  await page.getByRole("combobox").click();
  await page.getByRole("option", { name: "Paris" }).click();
  await expect(rows(page)).toHaveCount(2);
  await expect(rows(page).first()).toContainText("P-1");
});

test("opens an AGV and goes back", async ({ page }) => {
  await rows(page).filter({ hasText: "AGV-03" }).click();
  await expect(page).toHaveURL(/#\/agv\/acme%2FAGV-03$/);
  await expect(page.getByText("AGV-03")).toBeVisible();

  await page.goBack();
  await expect(rows(page)).toHaveCount(10);
});

test("keeps the previous rows while the next page loads", async ({ page }) => {
  await expect(rows(page)).toHaveCount(10);
  // Record the fewest rows the table ever shows during the page change.
  await page.evaluate(() => {
    const body = document.querySelector("tbody")!;
    const w = window as unknown as { minRows: number };
    w.minRows = body.rows.length;
    new MutationObserver(() => {
      w.minRows = Math.min(w.minRows, body.rows.length);
    }).observe(body, { childList: true, subtree: true });
  });
  await page.getByRole("button", { name: "Next page" }).click();
  await expect(rows(page)).toHaveCount(2);
  const minRows = await page.evaluate(() => (window as unknown as { minRows: number }).minRows);
  expect(minRows).toBeGreaterThan(0);
});

test("never shows the old AGV under a new AGV route", async ({ page }) => {
  await rows(page).filter({ hasText: "AGV-03" }).click();
  await expect(page.getByText("AGV-03")).toBeVisible();
  // Sample every frame: is AGV-03 still shown while the route is AGV-04?
  await page.evaluate(() => {
    const w = window as unknown as { stale: boolean };
    w.stale = false;
    const sample = () => {
      if (location.hash.endsWith("AGV-04") && document.body.innerText.includes("AGV-03")) {
        w.stale = true;
      }
      requestAnimationFrame(sample);
    };
    sample();
    location.hash = "#/agv/acme%2FAGV-04";
  });
  await expect(page.getByText("AGV-04")).toBeVisible();
  expect(await page.evaluate(() => (window as unknown as { stale: boolean }).stale)).toBe(false);
});

test("changing the search on page 2 returns to page 1", async ({ page }) => {
  await page.getByRole("button", { name: "Next page" }).click();
  await expect(rows(page)).toHaveCount(2);
  await page.getByPlaceholder("Search AGVs").fill("acme/AGV-0");
  await expect(page.getByText("1–9")).toBeVisible();
  await expect(page.getByRole("button", { name: "Previous page" })).toBeDisabled();
});
