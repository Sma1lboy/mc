// The six deterministic tools the chat agent can call.
//
// Each zod schema mirrors the matching Rust `*Args` struct field-for-field, in
// snake_case, so the model's calls round-trip straight through to mc-core (which
// validates again — these schemas primarily shape the model's calls and give us
// types). Descriptions are copied from `mc_core::agent::chat::tools`. The real
// work is injected: `execute` forwards the parsed args to the host `ToolExecutor`
// and returns its output JSON unchanged.

import { tool, type ToolSet } from "ai";
import { z } from "zod";

import type { ToolExecutor } from "./types";

const searchBaseModpacksArgs = z.object({
  query: z.string().describe("English search keywords describing the desired modpack."),
  mc_version: z
    .string()
    .optional()
    .describe('Target Minecraft version, e.g. "1.20.1". Omit to search all versions.'),
  loader: z
    .string()
    .optional()
    .describe('Target loader, e.g. "fabric" / "quilt" / "forge" / "neoforge".'),
});

const inspectBaseModpackArgs = z.object({
  project_id: z.string().describe("Modrinth project id of the base modpack (from search_base_modpacks)."),
  mc_version: z.string().optional().describe("Target Minecraft version, used to pick the right pack version."),
  loader: z.string().optional().describe("Target loader, used to pick the right pack version."),
});

const searchModsArgs = z.object({
  query: z.string().describe("English search keywords for the mod / feature to find."),
  mc_version: z.string().describe('Target Minecraft version, e.g. "1.20.1".'),
  loader: z.string().describe('Target loader, e.g. "fabric".'),
});

const modGetDetailArgs = z.object({
  provider: z.string().optional().describe('"modrinth" (default) or "curseforge".'),
  project_id: z.string().describe("Project id of the mod (from search_mods / inspect_base_modpack)."),
  minecraft_version: z.string().optional().describe('Target Minecraft version to filter versions by, e.g. "1.20.1".'),
  loader: z.string().optional().describe('Target loader to filter versions by, e.g. "fabric".'),
});

const resolveModsArgs = z.object({
  project_ids: z
    .array(z.string())
    .describe('Project ids to resolve. Each is a bare id (Modrinth) or "<provider>:<id>".'),
  mc_version: z.string().describe("Target Minecraft version."),
  loader: z.string().describe("Target loader."),
  already_installed: z
    .array(z.string())
    .optional()
    .describe('Project keys ("<provider>:<id>" or bare) already installed; treated as satisfied and not resolved again.'),
});

const buildTarget = z.object({
  mc_version: z.string(),
  loader: z.string(),
});
const buildBasePack = z.object({
  project_id: z.string().describe("Modrinth project id of the base pack."),
  version_id: z.string().describe("The exact base pack version id to build on (from inspect/search)."),
  title: z.string().optional(),
  slug: z.string().optional(),
});
const buildModRef = z.object({
  provider: z.string().optional().describe('"modrinth" (default) or "curseforge".'),
  project_id: z.string(),
  version_id: z.string().describe("The resolved version id (from resolve_mods)."),
  title: z.string().optional(),
});
const buildModpackArgs = z.object({
  target: buildTarget,
  base_pack: buildBasePack.nullish().describe("The chosen base pack, or null to start from scratch (empty base)."),
  extra_mods: z.array(buildModRef).default([]).describe("Extra mods to add, as resolved refs from resolve_mods."),
  output_filename: z
    .string()
    .describe('Output file name (no path). ".mrpack" is appended if missing. The launcher decides the directory.'),
});

const DESCRIPTIONS: Record<string, string> = {
  search_base_modpacks:
    "Search for existing Minecraft modpacks (on Modrinth) that could be used as a base pack. Returns real candidates with provider, project_id, slug, title, author, downloads, and description. Use English keywords.",
  inspect_base_modpack:
    "Inspect a base modpack: download its archive, list the mods it already includes, and report the feature categories it covers. Use this before deciding which extra mods to add.",
  search_mods:
    "Search for individual Minecraft mods compatible with a Minecraft version + loader. Returns real candidates with provider, project_id, slug, title, downloads, and description. Use English keywords.",
  mod_get_detail:
    "Get the details of ONE mod: its title/description/categories plus the newest versions available for a Minecraft version + loader (with real version ids, version numbers, and dependency counts). Use this to verify a specific mod actually supports the target before proposing or resolving it.",
  resolve_mods:
    "Resolve mod project ids into concrete, download-ready file references for a Minecraft version + loader, pulling in required dependencies. Returns resolved refs (with real version_id, url, hashes), plus anything unresolved or conflicting. The resolved refs are what you pass to build_modpack.",
  build_modpack:
    "Deterministically build and verify a .mrpack from a base pack (or from scratch) plus extra mods. THIS WRITES TO DISK — only call it after the user has explicitly confirmed the final plan.",
};

// One tool, with its `execute` bound to `exec[name]`. Generic over the schema so
// each call keeps a single concrete input type (indexing a union of schemas would
// collapse the SDK's inference to `never`).
function bound<S extends z.ZodType>(name: string, inputSchema: S, exec: ToolExecutor) {
  return tool({
    description: DESCRIPTIONS[name],
    inputSchema,
    execute: (args: z.infer<S>) => {
      const run = exec[name];
      if (!run) throw new Error(`no executor bound for tool ${name}`);
      return run(args);
    },
  });
}

/**
 * Build the AI SDK `ToolSet` for one turn, binding each tool's `execute` to the
 * injected host executor. The SDK auto-dispatches these during the multi-step
 * loop and feeds results back to the model (same shape as Rig on the Rust side).
 */
export function buildTools(exec: ToolExecutor): ToolSet {
  return {
    search_base_modpacks: bound("search_base_modpacks", searchBaseModpacksArgs, exec),
    inspect_base_modpack: bound("inspect_base_modpack", inspectBaseModpackArgs, exec),
    search_mods: bound("search_mods", searchModsArgs, exec),
    mod_get_detail: bound("mod_get_detail", modGetDetailArgs, exec),
    resolve_mods: bound("resolve_mods", resolveModsArgs, exec),
    build_modpack: bound("build_modpack", buildModpackArgs, exec),
  };
}
