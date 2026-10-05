import { mkdtemp, rm, mkdir, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
const directory = await mkdtemp(join(tmpdir(), "flow-e2e-"));
const projects = join(directory, "projects");
const source = await readFile("../runtime/application.flow", "utf8");
for (const name of ["demo", "second", "broken"]) {
  await mkdir(join(projects, name), { recursive: true });
  await writeFile(
    join(projects, name, "application.flow"),
    name === "broken" ? "[invalid" : source,
  );
}
const control = join(tmpdir(), "flow-e2e-19091.json");
await writeFile(control, JSON.stringify({ projects }));
const build = Bun.spawn(
  [
    "cargo",
    "build",
    "--manifest-path",
    "../runtime/Cargo.toml",
    "--jobs",
    "1",
    "--quiet",
  ],
  { stdout: "inherit", stderr: "inherit" },
);
if (await build.exited) throw Error("Runtime build failed");
const server = Bun.spawn(
  [
    "../runtime/target/debug/flow-runtime",
    projects,
    join(directory, "data"),
    "127.0.0.1:18081",
    "127.0.0.1:11884",
  ],
  {
    stdout: "inherit",
    stderr: "inherit",
    env: {
      ...process.env,
      FLOW_ADMIN_BIND: "127.0.0.1:19091",
      FLOW_ADMIN_TOKEN: "playwright-admin-test-key-at-least-32-bytes",
      FLOW_AUTOMATION_KEY: "automation-key-long-enough-123456789",
      FLOW_JWT_SECRET: "playwright-development-signing-key",
      FLOW_OBSERVABILITY_DIR: join(directory, "observability"),
    },
  },
);
for (const signal of ["SIGTERM", "SIGINT"] as const)
  process.on(signal, () => server.kill("SIGINT"));
try {
  await server.exited;
} finally {
  await rm(directory, { recursive: true, force: true });
  await rm(control, { force: true });
}
