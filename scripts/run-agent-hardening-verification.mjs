#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const sealed = process.argv.includes("--sealed");
const quick = process.argv.includes("--quick");

const checks = [
  ["agent-core tests", "npm", ["test", "--workspace", "@kobemc/agent-core"]],
  [
    "instance deterministic eval",
    "npm",
    ["run", "eval:instance", "--workspace", "@kobemc/agent-core", "--", "--json"],
  ],
  ["desktop tests", "npm", ["test", "--workspace", "desktop"]],
  ["desktop types", "npx", ["tsc", "--noEmit", "-p", "desktop/tsconfig.json"]],
  ...(
    quick
      ? []
      : [
          ["desktop production build", "npm", ["run", "build", "--workspace", "desktop"]],
          [
            "native conversation history",
            "cargo",
            [
              "test",
              "--manifest-path",
              "desktop/src-tauri/Cargo.toml",
              "--test",
              "agent_history",
            ],
          ],
        ]
  ),
  ["maintained file-size gate", "npm", ["run", "check:lines"]],
  ["generate Web report", "node", ["scripts/generate-agent-hardening-report.mjs"]],
  [
    "Web report contract",
    "node",
    [
      "scripts/check-agent-hardening-report.mjs",
      ...(sealed ? ["--sealed"] : []),
    ],
  ],
  ["whitespace gate", "git", ["diff", "--check"]],
];

const results = [];
for (const [label, command, args] of checks) {
  const startedAt = Date.now();
  process.stdout.write("\n[verify] " + label + "\n");
  const result = spawnSync(command, args, {
    cwd: root,
    env: process.env,
    stdio: "inherit",
    timeout: 10 * 60 * 1000,
  });
  const elapsedMs = Date.now() - startedAt;
  const passed = result.status === 0;
  results.push({ label, passed, elapsedMs });
  if (!passed) {
    const detail =
      result.error?.message ??
      "exit=" + result.status + " signal=" + (result.signal ?? "none");
    process.stderr.write("[verify] FAILED " + label + ": " + detail + "\n");
    process.exitCode = 1;
    break;
  }
}

const passed = results.filter((result) => result.passed).length;
process.stdout.write(
  "\n[verify] " +
    passed +
    "/" +
    checks.length +
    " checks passed" +
    (quick ? " (quick)" : "") +
    (sealed ? " (sealed)" : "") +
    "\n",
);
