// Host tool backends for the brain: a canned one for tests, and a real
// read-only one over the Modrinth HTTP API (server hosting; no disk writes).
export { mockExecutor } from "./mock";
export { modrinthExecutor, BUILD_UNSUPPORTED_MESSAGE } from "./modrinth";
export type { ModrinthExecutorOptions } from "./modrinth";
