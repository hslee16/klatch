// The published package (@klatch/client) is compiled JavaScript with types. This checks its metadata, then builds it,
// packs it, installs the tarball into an empty directory and imports it there, as a stranger would.
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const pkg = JSON.parse(readFileSync(join(here, "package.json"), "utf8"));

test("the package is public, scoped, ESM and ships compiled files only", () => {
  assert.equal(pkg.name, "@klatch/client");
  assert.notEqual(pkg.private, true);
  assert.equal(pkg.type, "module");
  assert.equal(pkg.publishConfig?.access, "public");
  assert.ok(pkg.engines?.node, "engines.node is set");
  assert.deepEqual(pkg.files, ["dist", "README.md", "LICENSE"]);
  assert.equal(pkg.exports["."].import, "./dist/klatch.js");
  assert.equal(pkg.exports["."].types, "./dist/klatch.d.ts");
  assert.match(pkg.version, /^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/);
});

test("the readme says pre-release, no review, and not related to the other klatch", () => {
  const text = readFileSync(join(here, "README.md"), "utf8").toLowerCase();
  for (const phrase of ["pre-release", "not for sensitive data", "not related to design-in-product/klatch"]) {
    assert.ok(text.includes(phrase), phrase);
  }
});

test("the packed tarball installs in an empty directory and imports", () => {
  const work = mkdtempSync(join(tmpdir(), "klatch-npm-"));
  try {
    execFileSync("npm", ["run", "build"], { cwd: here, stdio: "pipe" });
    const out = execFileSync("npm", ["pack", "--pack-destination", work, "--json"], { cwd: here, stdio: "pipe" });
    const [{ filename, files }] = JSON.parse(out.toString());
    const names: string[] = files.map((f: { path: string }) => f.path);
    assert.ok(names.includes("dist/klatch.js") && names.includes("dist/klatch.d.ts"), names.join(","));
    assert.ok(!names.some((n) => n.endsWith(".ts") && !n.endsWith(".d.ts")), "no TypeScript source is shipped");
    assert.ok(!names.some((n) => n.includes("test")), "no tests are shipped");
    const consumer = join(work, "consumer");
    execFileSync("mkdir", [consumer]);
    writeFileSync(join(consumer, "package.json"), JSON.stringify({ type: "module" }));
    execFileSync("npm", ["install", "--silent", join(work, filename)], { cwd: consumer, stdio: "pipe" });
    writeFileSync(
      join(consumer, "check.mjs"),
      'import { Client, PROTOCOL_VERSION, ApiError } from "@klatch/client";\n' +
        'if (PROTOCOL_VERSION !== 1 || typeof Client !== "function" || typeof ApiError !== "function") process.exit(1);\n' +
        "console.log(PROTOCOL_VERSION);\n",
    );
    const printed = execFileSync("node", ["check.mjs"], { cwd: consumer }).toString().trim();
    assert.equal(printed, "1");
    assert.ok(readdirSync(join(consumer, "node_modules", "@klatch", "client", "dist")).includes("klatch.js"));
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
});
