import { test, expect } from "@playwright/test";
const key = "playwright-admin-test-key-at-least-32-bytes";
const headers = { Authorization: `Bearer ${key}` };
test.beforeEach(async ({ page }) => {
  await page.goto("/");
  await page.getByLabel("Admin access token").fill(key);
  await page.getByRole("button", { name: /connect|open|sign in/i }).click();
  await expect(
    page.getByRole("heading", { name: "Overview", exact: true }),
  ).toBeVisible();
});
test("admin token survives reload and is cleared on disconnect", async ({
  page,
}) => {
  expect(
    await page.evaluate(() => sessionStorage.getItem("flow.adminToken")),
  ).toBe(key);
  await page.reload();
  await expect(
    page.getByRole("heading", { name: "Overview", exact: true }),
  ).toBeVisible();
  await expect(page.getByLabel("Admin access token")).toHaveCount(0);
  await page.getByRole("button", { name: /disconnect admin/i }).click();
  await expect(page.getByLabel("Admin access token")).toBeVisible();
  expect(
    await page.evaluate(() => sessionStorage.getItem("flow.adminToken")),
  ).toBeNull();
  await page.reload();
  await expect(page.getByLabel("Admin access token")).toBeVisible();
});
test("navigation, performance charts, larger fonts and mobile layout", async ({
  page,
  request,
}) => {
  for (let i = 0; i < 12; i++)
    expect(
      (await request.get("http://127.0.0.1:18081/api/products")).ok(),
    ).toBeTruthy();
  await page.getByLabel("Refresh dashboard").click();
  await expect(page.locator(".chart-canvas svg").first()).toBeVisible();
  const gradient = page.locator(".chart-canvas stop").first();
  await expect(gradient).toHaveAttribute("stop-color", /^#/);
  await expect(gradient).toHaveAttribute("stop-opacity", ".13");
  await expect(page.locator(".chart-canvas polyline").first()).toHaveAttribute(
    "fill",
    "none",
  );
  await page.locator(".chart-canvas").first().hover();
  await expect(page.locator(".chart-tooltip").first()).toBeVisible();
  expect(
    await page
      .locator(".nav-item")
      .first()
      .evaluate((el) => parseFloat(getComputedStyle(el).fontSize)),
  ).toBeGreaterThanOrEqual(14);
  for (const name of [
    "Traffic & latency",
    "Streaming",
    "Runtime & storage",
    "Log explorer",
    "Mutation audit",
    "HTTP console",
    "Access tokens",
  ]) {
    await page.getByRole("link", { name, exact: true }).click();
    await expect(
      page.getByRole("heading", { name, exact: true, level: 1 }),
    ).toBeVisible();
    if (name === "Runtime & storage") {
      await expect(page.getByText("Process RSS / host RAM")).toBeVisible();
      await expect(page.getByText("Disk logs")).toBeVisible();
      await expect(page.getByText("Dashboard log cache").first()).toBeVisible();
      await expect(page.getByText(/waiting to flush/)).toBeVisible();
      await expect(page.getByText("target VPS RAM")).toHaveCount(0);
    }
  }
  await page.setViewportSize({ width: 390, height: 844 });
  await page.getByLabel("Open navigation").click();
  await page.getByRole("link", { name: "Overview", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Overview", exact: true }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBeTruthy();
});
test("issue JWT, call endpoint and record committed mutation audit", async ({
  page,
  request,
}) => {
  await page.getByRole("link", { name: "Access tokens", exact: true }).click();
  await page.getByLabel("Subject", { exact: true }).fill("idp:u1");
  await page
    .getByRole("button", { name: "Generate application token" })
    .click();
  await page.getByRole("button", { name: "Use in console" }).click();
  await page.getByLabel("Operation", { exact: true }).selectOption("Users");
  await page.getByRole("button", { name: /send request/i }).click();
  await expect(page.locator(".response-body")).toContainText("Petra");
  await expect(page.locator(".response-body")).not.toContainText("David");
  await page
    .getByLabel("Operation", { exact: true })
    .selectOption("SendCommand");
  await page.getByLabel("Request path").fill("/api/devices/mower1/commands");
  await page.getByLabel("JSON body").fill('{"action":"START"}');
  await page.getByRole("button", { name: /send request/i }).click();
  await expect(page.locator(".response-body")).toContainText("START");
  const audit = await request.get("/api/audit?entity=DeviceCommand", {
    headers,
  });
  expect(audit.ok()).toBeTruthy();
  expect(JSON.stringify(await audit.json())).toContain("START");
  await page.getByRole("link", { name: "Mutation audit", exact: true }).click();
  await expect(page.locator("tbody")).toContainText("DeviceCommand");
});
test("filter and page real request logs and enforce admin authentication", async ({
  page,
  request,
}) => {
  for (let i = 0; i < 210; i++)
    await request.get("http://127.0.0.1:18081/api/products");
  await page.getByRole("link", { name: "Log explorer", exact: true }).click();
  await page.getByLabel("Log kind", { exact: true }).selectOption("request");
  await page.getByLabel("Search log text").fill("/api/products");
  await expect(page.locator("tbody tr")).toHaveCount(100);
  await page.getByRole("button", { name: /older/i }).click();
  await expect(page.locator("tbody tr")).toHaveCount(100);
  await page.locator("tbody tr").first().click();
  await expect(page.locator(".drawer")).toBeVisible();
  expect((await request.get("/api/overview")).status()).toBe(401);
  expect(
    (
      await request.post("/api/jwt", {
        data: { adapter: "user", subject: "idp:u1", ttl_seconds: 300 },
      })
    ).status(),
  ).toBe(401);
  expect(
    (await request.get("http://127.0.0.1:18081/api/overview")).status(),
  ).toBe(404);
});

test("audit detail exposes decoded values for historical seed records", async ({
  page,
  request,
}) => {
  const response = await request.get("/api/audit?entity=User", { headers });
  const rows = await response.json();
  const user = rows.find(
    (row: { entity_id: string }) => row.entity_id === "u1",
  );
  expect(user.after.country).toBe("CZ");
  expect(user.after.roles).toEqual([]);
  await page.getByRole("link", { name: "Mutation audit", exact: true }).click();
  await page.getByLabel("Filter audit entity").fill("User");
  await expect(page.locator("tbody tr")).toHaveCount(4);
  await page
    .locator("tbody tr")
    .filter({ has: page.locator(".table-subtitle", { hasText: /^u1$/ }) })
    .click();
  await expect(page.locator(".drawer")).toContainText('"country": "CZ"');
  await expect(page.locator(".drawer")).not.toContainText('\\"CZ\\"');
});
