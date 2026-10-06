import { spawn, type ChildProcess } from "node:child_process";
import { test, expect } from "@playwright/test";
import { mkdtemp, mkdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

test("first setup creates a persistent owner without a manual admin token", async ({
  page,
  request,
}) => {
  const directory = await mkdtemp(join(tmpdir(), "flow-first-setup-"));
  const projects = join(directory, "projects");
  await mkdir(projects);
  const start = () =>
    spawn(
      "../runtime/target/debug/flow-runtime",
      [projects, join(directory, "data"), "127.0.0.1:18082", "127.0.0.1:11885"],
      {
        env: {
          PATH: process.env.PATH ?? "",
          FLOW_ADMIN_BIND: "127.0.0.1:19092",
          FLOW_OBSERVABILITY_DIR: join(directory, "observability"),
        },
        stdio: ["ignore", "ignore", "inherit"],
      },
    );
  let server = start();
  try {
    await expect
      .poll(async () => {
        try {
          return (
            await request.get("http://127.0.0.1:19092/api/setup")
          ).status();
        } catch {
          return 0;
        }
      })
      .toBe(200);
    await page.goto("http://127.0.0.1:19092");
    await expect(
      page.getByRole("heading", { name: "Welcome to your server." }),
    ).toBeVisible();
    await page.getByLabel("Server name").fill("Office server");
    await page.getByLabel("Choose a password").fill("fictional setup password");
    await page.getByRole("button", { name: "Start using Flow" }).click();
    await expect(
      page.getByRole("heading", { name: "Applications", exact: true }),
    ).toBeVisible();
    await page
      .getByRole("link", { name: "Server settings", exact: true })
      .click();
    await expect(page.getByLabel("Server name")).toHaveValue("Office server");
    await page.getByLabel("Server name").fill("Office renamed");
    await page
      .getByRole("button", { name: "Save settings", exact: true })
      .click();
    await expect(page.getByText("Settings saved.")).toBeVisible();
    await page.getByRole("button", { name: "Disconnect admin" }).click();
    server.kill("SIGINT");
    await stopped(server);
    server = start();
    await expect
      .poll(async () => {
        try {
          return (
            await request.get("http://127.0.0.1:19092/api/setup")
          ).status();
        } catch {
          return 0;
        }
      })
      .toBe(200);
    await page.reload();
    await expect(
      page.getByRole("heading", { name: "Welcome back." }),
    ).toBeVisible();
    await page
      .getByLabel("Password", { exact: true })
      .fill("fictional setup password");
    await page.getByRole("button", { name: "Sign in", exact: true }).click();
    await page
      .getByRole("link", { name: "Server settings", exact: true })
      .click();
    await expect(page.getByLabel("Server name")).toHaveValue("Office renamed");
  } finally {
    server.kill("SIGINT");
    await stopped(server);
    await rm(directory, { recursive: true, force: true });
  }
});

function stopped(child: ChildProcess): Promise<void> {
  return new Promise((resolve) => {
    if (child.exitCode !== null || child.signalCode !== null) resolve();
    else child.once("exit", () => resolve());
  });
}
