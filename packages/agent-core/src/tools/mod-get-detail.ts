import { tool } from "ai";
import { z } from "zod";

export const modGetDetail = (instanceBound = false) =>
  tool({
    description:
      "Get the details of ONE mod: its title/description/categories plus the newest versions available for a Minecraft version + loader (with real version ids, version numbers, and dependency counts). Use this to verify a specific mod actually supports the target before proposing or resolving it.",
    inputSchema: z
      .object({
        provider: z.string().optional().describe('"modrinth" (default) or "curseforge".'),
        project_id: z
          .string()
          .describe("Project id of the mod (from search_mods / inspect_base_modpack)."),
        ...(instanceBound
          ? {}
          : {
              minecraft_version: z
                .string()
                .optional()
                .describe('Target Minecraft version to filter versions by, e.g. "1.20.1".'),
              loader: z
                .string()
                .optional()
                .describe('Target loader to filter versions by, e.g. "fabric".'),
            }),
      })
      .strict(),
    // No execute: launcher client runs this through Rust IPC and appends output.
  });
