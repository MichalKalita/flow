import { mkdir, copyFile } from "node:fs/promises";
const out = new URL("../runtime/src/admin-assets/", import.meta.url).pathname;
await mkdir(out, { recursive: true });
const result = await Bun.build({
  entrypoints: ["./src/main.tsx"],
  outdir: out,
  naming: "app.js",
  target: "browser",
  minify: true,
  define: { "process.env.NODE_ENV": JSON.stringify("production") },
});
if (!result.success) throw new Error(result.logs.join("\n"));
const css = Bun.spawn(
  [
    "bun",
    "./node_modules/@tailwindcss/cli/dist/index.mjs",
    "-i",
    "src/styles.css",
    "-o",
    `${out}app.css`,
    "--minify",
  ],
  { stdout: "inherit", stderr: "inherit" },
);
if ((await css.exited) !== 0) throw new Error("Tailwind build failed");
await copyFile("index.html", `${out}index.html`);
console.log("Embedded control-plane assets built.");

const application = await Bun.build({
  entrypoints: ["./src/application.tsx"],
  outdir: out,
  naming: "application.js",
  target: "browser",
  minify: true,
});
if (!application.success) throw new Error(application.logs.join("\n"));
await copyFile("src/application.css", `${out}application.css`);
await copyFile("application.html", `${out}application.html`);
