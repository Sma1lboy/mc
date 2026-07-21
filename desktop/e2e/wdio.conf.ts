import { mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const binary = join(here, "..", "src-tauri", "target", "debug", "mc-launcher-desktop");
const artifacts = join(here, "artifacts");

export const config = {
  runner: "local",
  specs: [join(here, "specs", "app-smoke.e2e.ts")],
  maxInstances: 1,
  capabilities: [
    {
      browserName: "tauri",
      "tauri:options": { application: binary },
      "wdio:tauriServiceOptions": {
        appBinaryPath: binary,
        captureBackendLogs: true,
        captureFrontendLogs: true,
        backendLogLevel: "info",
        frontendLogLevel: "info",
      },
    },
  ],
  services: [["@wdio/tauri-service", { driverProvider: "embedded" }]],
  framework: "mocha",
  reporters: ["spec"],
  logLevel: "info",
  outputDir: join(artifacts, "logs"),
  waitforTimeout: 15_000,
  connectionRetryTimeout: 120_000,
  connectionRetryCount: 1,
  // webdriver's bundled undici is incompatible with Node 26's global dispatcher
  // when Content-Length is explicit; let undici compute the loopback request header.
  transformRequest: (requestOptions: RequestInit) => {
    const headers = new Headers(requestOptions.headers);
    headers.delete("content-length");
    return { ...requestOptions, headers };
  },
  mochaOpts: { ui: "bdd", timeout: 120_000 },
  tsConfigPath: join(here, "tsconfig.json"),
  afterTest: async function (test: { title: string }, _context: unknown, result: { passed: boolean }) {
    if (result.passed) return;
    mkdirSync(artifacts, { recursive: true });
    const name = test.title.replace(/[^a-z0-9_-]+/gi, "-").toLowerCase();
    await browser.saveScreenshot(join(artifacts, `${name || "failed"}.png`));
  },
};
