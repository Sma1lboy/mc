import { tool } from "ai";
import { z } from "zod";

export const searchMods = (instanceBound = false) =>
  tool({
    description: instanceBound
      ? "Search for individual Minecraft mods compatible with the currently bound instance. Returns real candidates with provider and project ids. The launcher injects the instance Minecraft version and loader. Use English keywords."
      : "Search for individual Minecraft mods compatible with a Minecraft version + loader. Returns real candidates with provider, project_id, slug, title, downloads, and description. Use English keywords.",
    inputSchema: instanceBound
      ? z
          .object({
            query: z.string().describe("English search keywords for the mod / feature to find."),
          })
          .strict()
      : z
          .object({
            query: z.string().describe("English search keywords for the mod / feature to find."),
            mc_version: z.string().describe('Target Minecraft version, e.g. "1.20.1".'),
            loader: z.string().describe('Target loader, e.g. "fabric".'),
          })
          .strict(),
    // No execute: launcher client runs this through Rust IPC and appends output.
  });
