// An instant, canned `ToolExecutor` for tests and offline demos.
//
// Every tool resolves immediately with a small, wire-shaped stub (field names
// match the Rust tools). Pass `fixtures` to override any tool with your own
// async function — e.g. a spy that records the args the model produced, or a
// canned result a specific test asserts on.

import type { ToolExecutor } from "../types";

/** Default canned outputs, shaped like the real tool results (snake_case). */
const DEFAULTS: ToolExecutor = {
  search_base_modpacks: async () => ({
    candidates: [
      {
        provider: "modrinth",
        project_id: "mock-pack",
        slug: "mock-pack",
        title: "Mock Pack",
        author: "mock",
        downloads: 1000,
        description: "A canned base modpack.",
      },
    ],
  }),
  inspect_base_modpack: async () => ({
    title: "Mock Pack",
    mc_version: "1.20.1",
    loader: "fabric",
    mod_count: 1,
    mods: [{ title: "Sodium", categories: ["performance"] }],
    covered_features: ["performance"],
  }),
  search_mods: async () => ({
    mods: [
      {
        provider: "modrinth",
        project_id: "mock-mod",
        slug: "mock-mod",
        title: "Mock Mod",
        downloads: 500,
        description: "A canned mod.",
      },
    ],
  }),
  mod_get_detail: async () => ({
    project: {
      title: "Mock Mod",
      slug: "mock-mod",
      description: "A canned mod.",
      categories: ["utility"],
      downloads: 500,
    },
    versions: [
      {
        version_id: "mock-ver",
        version_number: "1.0.0",
        game_versions: ["1.20.1"],
        loaders: ["fabric"],
        dependencies_count: 0,
        filename: "mock-mod-1.0.0.jar",
      },
    ],
  }),
  resolve_mods: async () => ({
    resolved: [
      {
        provider: "modrinth",
        project_id: "mock-mod",
        version_id: "mock-ver",
        filename: "mock-mod-1.0.0.jar",
        url: "https://example.invalid/mock-mod-1.0.0.jar",
        sha1: null,
        sha512: null,
        size: null,
      },
    ],
    unresolved: [],
    conflicts: [],
  }),
  build_modpack: async () => ({
    status: "ok",
    output_path: "/mock/out.mrpack",
    output_size: 1024,
    manifest: { status: "ok" },
  }),
};

/** Build a canned executor; `fixtures` overrides individual tools. */
export function mockExecutor(fixtures: Partial<ToolExecutor> = {}): ToolExecutor {
  return { ...DEFAULTS, ...fixtures };
}
