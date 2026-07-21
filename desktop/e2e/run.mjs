import { mkdtempSync, rmSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

const desktopDir = new URL("..", import.meta.url);
const artifactsDir = new URL("./artifacts", import.meta.url);

async function findAvailableDriverPort() {
  const server = createServer();
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  if (!address || typeof address === "string") {
    server.close();
    throw new Error("Failed to allocate an E2E WebDriver port");
  }
  await new Promise((resolve, reject) => server.close((error) => (error ? reject(error) : resolve())));
  return address.port;
}

const driverPort = await findAvailableDriverPort();
const dataDir = mkdtempSync(join(tmpdir(), "kobemc-e2e-"));
const env = {
  ...process.env,
  MC_E2E_DATA_DIR: dataDir,
  TAURI_WEBDRIVER_PORT: String(driverPort),
  VITE_E2E: "1",
};

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: desktopDir,
    env,
    stdio: "inherit",
  });
  if (result.error) throw result.error;
  return result.status ?? 1;
}

let status = 1;
try {
  rmSync(artifactsDir, { recursive: true, force: true });
  status = run("npm", [
    "run",
    "tauri",
    "--",
    "build",
    "--debug",
    "--no-bundle",
    "--features",
    "e2e",
    "--config",
    "src-tauri/tauri.e2e.conf.json",
  ]);

  if (status === 0) {
    status = run("npm", ["exec", "--", "wdio", "run", "e2e/wdio.conf.ts"]);
  }
} finally {
  rmSync(dataDir, { recursive: true, force: true });
  if (status === 0) rmSync(artifactsDir, { recursive: true, force: true });
}
process.exitCode = status;
