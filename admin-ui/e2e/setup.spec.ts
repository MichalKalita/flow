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
    const popup = page.waitForEvent("popup");
    await page
      .getByRole("button", { name: "Use Contacts", exact: true })
      .click();
    const contacts = await popup;
    await expect(
      contacts.getByRole("heading", { name: "Contacts", level: 1 }),
    ).toBeVisible();
    await contacts.getByLabel("Name", { exact: true }).fill("Ada Lovelace");
    await contacts
      .getByLabel("Email", { exact: true })
      .fill("ada@example.test");
    await contacts
      .getByRole("button", { name: "Add contact", exact: true })
      .click();
    await expect(
      contacts.getByRole("heading", { name: "Ada Lovelace", exact: true }),
    ).toBeVisible();
    await contacts.getByRole("button", { name: "Edit", exact: true }).click();
    await contacts.getByLabel("Phone", { exact: true }).fill("12345");
    await contacts
      .getByRole("button", { name: "Save changes", exact: true })
      .click();
    await expect(contacts.getByText("12345", { exact: true })).toBeVisible();
    await contacts.screenshot({
      path: "/tmp/flow-contacts.png",
      fullPage: true,
    });
    await page.getByRole("link", { name: "Backups", exact: true }).click();
    await page
      .getByRole("button", { name: "Create backup", exact: true })
      .click();
    await expect(
      page.getByText("Backup saved.", { exact: true }),
    ).toBeVisible();
    await page
      .getByRole("button", { name: "Check restore", exact: true })
      .click();
    await page
      .getByLabel("Recovery password", { exact: true })
      .fill("fictional setup password");
    await page
      .getByRole("button", { name: "Verify restoration", exact: true })
      .click();
    await expect(
      page.getByText(
        "Backup restored and verified in a separate temporary copy. Your running data is unchanged.",
      ),
    ).toBeVisible();
    await contacts.getByLabel("Name", { exact: true }).fill("Later contact");
    await contacts
      .getByRole("button", { name: "Add contact", exact: true })
      .click();
    await expect(
      contacts.getByRole("heading", { name: "Later contact", exact: true }),
    ).toBeVisible();
    await page
      .getByLabel("Recovery password", { exact: true })
      .fill("fictional setup password");
    page.once("dialog", (dialog) => dialog.accept());
    await page
      .getByRole("button", { name: "Restore this backup", exact: true })
      .click();
    await expect(
      page.getByText(
        "Server restored. A recovery copy of the previous data was kept.",
        { exact: true },
      ),
    ).toBeVisible({ timeout: 30000 });
    await contacts.reload();
    await expect(
      contacts.getByRole("heading", { name: "Ada Lovelace", exact: true }),
    ).toBeVisible();
    await expect(
      contacts.getByRole("heading", { name: "Later contact", exact: true }),
    ).toHaveCount(0);
    await page
      .getByRole("link", { name: "Server settings", exact: true })
      .click();
    await expect(page.getByLabel("Server name")).toHaveValue("Office server");
    await page.getByLabel("Server name").fill("Office renamed");
    await page.getByLabel("Local log storage (MiB)").fill("8");
    await page.getByLabel("Log chunk size (MiB)").fill("1");
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
    await expect(page.getByLabel("Local log storage (MiB)")).toHaveValue("8");
    await expect(page.getByLabel("Log chunk size (MiB)")).toHaveValue("1");
    await expect(page.getByText("minutes old", { exact: false })).toBeVisible();
    await page.screenshot({
      path: "/tmp/flow-server-settings.png",
      fullPage: true,
    });
    await contacts.reload();
    await expect(
      contacts.getByRole("heading", { name: "Ada Lovelace", exact: true }),
    ).toBeVisible();
    await expect(contacts.getByText("12345", { exact: true })).toBeVisible();
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
