import { tool } from "ai";
import { z } from "zod";

export const resolveMods = () =>
  tool({
    description:
      "Resolve mod project ids into concrete, download-ready file references for a Minecraft version + loader, pulling in required dependencies. Returns resolved refs (with real version_id, url, hashes), plus anything unresolved or conflicting. The resolved refs are what you pass to build_modpack.",
    inputSchema: z.object({
      project_ids: z
        .array(z.string())
        .describe('Project ids to resolve. Each is a bare id (Modrinth) or "<provider>:<id>".'),
      mc_version: z.string().describe("Target Minecraft version."),
      loader: z.string().describe("Target loader."),
      already_installed: z
        .array(z.string())
        .optional()
        .describe('Project keys ("<provider>:<id>" or bare) already installed; treated as satisfied and not resolved again.'),
    }),
    // No execute: launcher client runs this through Rust IPC and appends output.
  });
