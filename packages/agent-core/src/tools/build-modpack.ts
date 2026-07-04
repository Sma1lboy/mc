import { tool } from "ai";
import { z } from "zod";

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

export const buildModpack = () =>
  tool({
    description:
      "Deterministically build and verify a .mrpack from a base pack (or from scratch) plus extra mods. THIS WRITES TO DISK — only call it after the user has explicitly confirmed the final plan.",
    inputSchema: z.object({
      target: buildTarget,
      base_pack: buildBasePack.nullish().describe("The chosen base pack, or null to start from scratch (empty base)."),
      extra_mods: z.array(buildModRef).default([]).describe("Extra mods to add, as resolved refs from resolve_mods."),
      output_filename: z
        .string()
        .describe('Output file name (no path). ".mrpack" is appended if missing. The launcher decides the directory.'),
    }),
    // No execute: launcher client runs this through Rust IPC and appends output.
  });
