// Ladle-only Tauri backend stub.
//
// The whole point: DON'T maintain a second UI. Import the REAL components/pages into
// stories and let them run — this routes their `invoke()` (via Tauri v2's
// `window.__TAURI_INTERNALS__`) to canned mock data, so a real page renders in Ladle
// without a backend. Add/adjust mock responses HERE in one place (keyed by the Rust
// command name); unknown commands resolve to `null` so `.catch`/optional handling in
// the real code copes gracefully. Never shipped — Ladle dev tooling only.

type Args = Record<string, unknown> | undefined;

const NOW = Date.now();

const INSTANCES = [
  { id: "tech", name: "科技生存 1.20.1", mc_version: "1.20.1", loader: "fabric", loader_version: "0.15.7", last_played: NOW - 3e5, icon: null },
  { id: "rpg", name: "RPG 冒险", mc_version: "1.20.1", loader: "forge", loader_version: "47.2.0", last_played: NOW - 864e5, icon: null },
];

const HITS = [
  { id: "a", slug: "create-aab", title: "Create: Above and Beyond", description: "以 Create 机械动力为核心的科技进度整合包。", author: "simibubi", downloads: 4820000, icon_url: null, gallery_url: null, categories: ["technology", "quests"] },
  { id: "b", slug: "better-mc", title: "Better MC [FORGE]", description: "大而全的冒险整合。", author: "luna", downloads: 1100000, icon_url: null, gallery_url: null, categories: ["adventure"] },
];

// Rust command name → mock response (value or fn(args)). Extend as pages need data.
const MOCK: Record<string, unknown | ((a: Args) => unknown)> = {
  list_roots: [{ id: "default", path: "/mock/minecraft", label: "默认根目录", is_default: true }],
  current_root: "default",
  list_instances: INSTANCES,
  running_instances: [],
  get_theme: { mode: "dark", hue: 26, saturation: 80, lightness: 55 },
  agent_llm_config: { api_key: "", model: "mock-model", base_url: "https://openrouter.ai/api/v1" },
  modrinth_search: HITS,
  kobe_list_credentials: [],
  kobe_current: null,
  list_accounts: [],
  current_account: null,
};

function mockInvoke(cmd: string, args: Args): unknown {
  const v = MOCK[cmd];
  if (typeof v === "function") return (v as (a: Args) => unknown)(args);
  if (v !== undefined) return v;
  // event plugin listen/unlisten → return a fake id so `listen()` doesn't throw.
  if (cmd.startsWith("plugin:event")) return 1;
  if (cmd.startsWith("plugin:")) return undefined;
  return null; // unknown data command → null; real code's optional/.catch handling copes
}

// Install the Tauri v2 internals shim once (before any story renders).
let cbId = 0;
const w = window as unknown as { __TAURI_INTERNALS__?: unknown };
if (!w.__TAURI_INTERNALS__) {
  w.__TAURI_INTERNALS__ = {
    invoke: (cmd: string, args: Args) => Promise.resolve(mockInvoke(cmd, args)),
    transformCallback: () => {
      cbId += 1;
      return cbId; // events never fire in Ladle; a stable id is enough
    },
    // getCurrentWindow()/getCurrentWebview() read these — needed by window/webview
    // APIs (e.g. useModpackDrop's drag-drop listener) so they don't crash on mount.
    metadata: {
      currentWindow: { label: "main" },
      currentWebview: { windowLabel: "main", label: "main" },
    },
  };
}

export {};
