import { readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test, expect } from "@playwright/test";
const key = "playwright-admin-test-key-at-least-32-bytes";
const headers = { Authorization: `Bearer ${key}` };
const browserErrors = new WeakMap<object, string[]>();
test.afterEach(async ({ page }) => {
  expect(browserErrors.get(page) ?? []).toEqual([]);
});
test.beforeEach(async ({ page }) => {
  const errors: string[] = [];
  browserErrors.set(page, errors);
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto("/");
  await page.getByLabel("Admin access token").fill(key);
  await page.getByRole("button", { name: /connect|open|sign in/i }).click();
  await expect(
    page.getByRole("heading", { name: "Overview", exact: true }),
  ).toBeVisible();
  await expect(page.getByLabel("Active project")).toContainText("demo");
  await page.getByLabel("Active project").selectOption("demo");
  await expect(page.getByLabel("Active project")).toHaveValue("demo");
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
      (await request.get("http://127.0.0.1:18081/demo/api/products")).ok(),
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
  await page.screenshot({
    path: test.info().outputPath("overview-desktop.png"),
    fullPage: true,
  });
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
    "Database",
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
  const users = JSON.parse(await page.locator(".response-body").innerText());
  expect(users[0].id).toBe(1);
  expect(typeof users[0].id).toBe("number");
  await page
    .getByLabel("Operation", { exact: true })
    .selectOption("SendCommand");
  await page.getByLabel("Request path").fill("/api/devices/1/commands");
  await page.getByLabel("JSON body").fill('{"action":"START"}');
  await page.getByRole("button", { name: /send request/i }).click();
  await expect(page.locator(".response-body")).toContainText("START");
  const audit = await request.get(
    "/api/audit?project=demo&entity=DeviceCommand",
    {
      headers,
    },
  );
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
    await request.get("http://127.0.0.1:18081/demo/api/products");
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
      await request.post("/api/jwt?project=demo", {
        data: { adapter: "user", subject: "idp:u1", ttl_seconds: 300 },
      })
    ).status(),
  ).toBe(401);
  expect(
    (await request.get("http://127.0.0.1:18081/demo/api/overview")).status(),
  ).toBe(404);
});

test("audit detail exposes decoded values for historical seed records", async ({
  page,
  request,
}) => {
  const response = await request.get("/api/audit?project=demo&entity=User", {
    headers,
  });
  const rows = await response.json();
  const user = rows.find((row: { entity_id: number }) => row.entity_id === 1);
  expect(user.after.country).toBe("CZ");
  expect(user.after.roles).toEqual([]);
  await page.getByRole("link", { name: "Mutation audit", exact: true }).click();
  await page.getByLabel("Filter audit entity").fill("User");
  await expect(page.locator("tbody tr")).toHaveCount(4);
  await page
    .locator("tbody tr")
    .filter({ has: page.locator(".table-subtitle", { hasText: /^1$/ }) })
    .click();
  await expect(page.locator(".drawer")).toContainText('"country": "CZ"');
  await expect(page.locator(".drawer")).not.toContainText('\\"CZ\\"');
});

test("system overview aggregates projects and preserves project-specific tools", async ({
  page,
  request,
}) => {
  await request.get("http://127.0.0.1:18081/demo/api/products");
  await request.get("http://127.0.0.1:18081/second/api/products");
  await page.getByLabel("Active project").selectOption("all");
  await page.getByLabel("Refresh dashboard").click();
  const overview = await (
    await request.get("/api/overview?project=all", { headers })
  ).json();
  expect(overview.scope).toBe("all");
  expect(overview.metrics.system.count).toBeGreaterThan(0);
  expect(
    overview.endpoints.some(
      (e: { path: string }) => e.path === "/second/api/products",
    ),
  ).toBeTruthy();
  expect(overview.resources.process.rss_bytes).toBeGreaterThan(0);
  await test.info().attach("process-resources", {
    body: JSON.stringify(overview.resources, null, 2),
    contentType: "application/json",
  });
  await expect(page.locator(".chart-canvas svg").first()).toBeVisible();
  await expect(
    page.getByRole("alert").filter({ hasText: "broken" }),
  ).toBeVisible();
  await page.getByRole("link", { name: "HTTP console", exact: true }).click();
  await expect(
    page.getByText("Select a project", { exact: true }),
  ).toBeVisible();
  await page.getByLabel("Active project").selectOption("second");
  await expect(page.getByLabel("Operation", { exact: true })).toBeVisible();
  await page.getByLabel("Operation", { exact: true }).selectOption("Products");
  await page.getByRole("button", { name: /send request/i }).click();
  await expect(page.locator(".response-body")).toContainText("USB-C");
  const audit = await (
    await request.get("/api/audit?project=all", { headers })
  ).json();
  expect(new Set(audit.map((row: { project: string }) => row.project))).toEqual(
    new Set(["demo", "second"]),
  );
  expect(typeof audit[0].cursor).toBe("string");
});

test("database browser validates, searches, updates, detects conflicts and audits deletion", async ({
  page,
  request,
}) => {
  await page.getByRole("link", { name: "Database", exact: true }).click();
  await page.getByLabel("Database entity").selectOption("Product");
  await expect(page.locator("tbody tr")).toHaveCount(4);
  await page
    .getByRole("button", { name: "Create record", exact: true })
    .click();
  await page
    .getByLabel("Record JSON")
    .fill(JSON.stringify({ name: "E2E disposable", price: -1, stock: 1 }));
  await page.getByRole("button", { name: "Save record", exact: true }).click();
  await expect(page.locator(".drawer [role=alert]")).toBeVisible();
  await page
    .getByLabel("Record JSON")
    .fill(JSON.stringify({ name: "E2E disposable", price: 9, stock: 1 }));
  await page.getByRole("button", { name: "Save record", exact: true }).click();
  await expect(page.locator(".drawer")).toHaveCount(0);
  await page.getByLabel("Search database records").fill("E2E disposable");
  await expect(page.locator("tbody tr")).toHaveCount(1);
  const response = await request.get(
    "/api/data/rows?project=demo&entity=Product&search=E2E",
    { headers },
  );
  const original = (await response.json()).rows[0];
  await page
    .getByRole("button", {
      name: `Edit record ${original.record.id}`,
      exact: true,
    })
    .click();
  await page
    .getByLabel("Record JSON")
    .fill(JSON.stringify({ ...original.record, name: "E2E renamed" }));
  await page.getByRole("button", { name: "Save record", exact: true }).click();
  await expect(page.locator(".drawer")).toHaveCount(0);
  expect(
    (
      await request.post("/api/data?project=demo", {
        headers,
        data: {
          action: "UPDATE",
          entity: "Product",
          id: original.record.id,
          record: original.record,
          etag: original.etag,
        },
      })
    ).status(),
  ).toBe(409);
  const second = await (
    await request.get(
      "/api/data/rows?project=second&entity=Product&search=E2E",
      { headers },
    )
  ).json();
  expect(second.rows).toEqual([]);
  await page.getByLabel("Search database records").fill("E2E renamed");
  await expect(page.locator("tbody tr")).toHaveCount(1);
  await page
    .getByRole("button", {
      name: `Edit record ${original.record.id}`,
      exact: true,
    })
    .click();
  const current = await (
    await request.get(
      `/api/data/rows?project=demo&entity=Product&id=${original.record.id}`,
      { headers },
    )
  ).json();
  expect(
    (
      await request.post("/api/data?project=demo", {
        headers,
        data: {
          action: "UPDATE",
          entity: "Product",
          id: original.record.id,
          record: { ...current.rows[0].record, name: "E2E concurrent" },
          etag: current.rows[0].etag,
        },
      })
    ).ok(),
  ).toBeTruthy();
  await page.getByRole("button", { name: "Save record", exact: true }).click();
  await expect(page.locator(".drawer [role=alert]")).toContainText(
    "Record changed",
  );
  await page
    .getByRole("button", { name: "Reload record", exact: true })
    .click();
  await expect(page.getByLabel("Record JSON")).toHaveValue(/E2E concurrent/);
  await page
    .getByRole("button", { name: "Delete record", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Confirm deletion", exact: true })
    .click();
  await expect(page.locator(".drawer")).toHaveCount(0);
  await expect(page.locator("tbody tr")).toHaveCount(0);
  const audit = await (
    await request.get(
      "/api/audit?project=demo&entity=Product&transport=admin",
      { headers },
    )
  ).json();
  expect(
    audit.some(
      (row: { action: string; entity_id: number }) =>
        row.action === "DELETE" && row.entity_id === original.record.id,
    ),
  ).toBeTruthy();
  expect(
    audit.some(
      (row: { after?: { name?: string } }) => row.after?.name === "E2E renamed",
    ),
  ).toBeTruthy();
  expect((await request.get("/api/data?project=demo")).status()).toBe(401);
});

test("file watcher keeps last valid project on errors and activates corrected source", async ({
  page,
  request,
}) => {
  const control = JSON.parse(
    await readFile(join(tmpdir(), "flow-e2e-19091.json"), "utf8"),
  );
  const sourcePath = join(control.projects, "second", "application.flow");
  const original = await readFile(sourcePath, "utf8");
  const generation = async () => {
    const response = await request.get("/api/projects", { headers });
    return (await response.json()).projects.find(
      (p: { name: string }) => p.name === "second",
    );
  };
  const initial = await generation();
  try {
    await writeFile(sourcePath, "[invalid");
    await expect
      .poll(async () => (await generation()).status, { timeout: 10000 })
      .toBe("stale");
    expect((await generation()).generation).toBe(initial.generation);
    expect(
      (
        await request.get("http://127.0.0.1:18081/second/api/products")
      ).status(),
    ).toBe(200);
    expect(
      await readFile(join(control.projects, "second-error.txt"), "utf8"),
    ).toBeTruthy();
    await page.getByLabel("Active project").selectOption("second");
    await page.getByLabel("Refresh dashboard").click();
    await expect(
      page.getByRole("alert").filter({ hasText: "second:" }),
    ).toContainText("Last working version remains active");
    await writeFile(
      sourcePath,
      original.replace(
        '[http GET "/api/products"]',
        '[http GET "/api/catalog"]',
      ),
    );
    await expect
      .poll(async () => (await generation()).status, { timeout: 10000 })
      .toBe("running");
    expect((await generation()).generation).toBeGreaterThan(initial.generation);
    expect(
      (await request.get("http://127.0.0.1:18081/second/api/catalog")).status(),
    ).toBe(200);
    expect(
      (
        await request.get("http://127.0.0.1:18081/second/api/products")
      ).status(),
    ).toBe(404);
    expect(
      (await request.get("http://127.0.0.1:18081/demo/api/products")).status(),
    ).toBe(200);
    await expect(
      readFile(join(control.projects, "second-error.txt")),
    ).rejects.toThrow();
  } finally {
    await writeFile(sourcePath, original);
    await expect
      .poll(async () => (await generation()).status, { timeout: 10000 })
      .toBe("running");
  }
});
