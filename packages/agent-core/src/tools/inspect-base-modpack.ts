import { tool } from "ai";
import { z } from "zod";

export const inspectBaseModpack = () =>
  tool({
    description:
      "Inspect a base modpack: download its archive, list the mods it already includes, and report the feature categories it covers. Use this before deciding which extra mods to add.",
    inputSchema: z.object({
      project_id: z.string().describe("Modrinth project id of the base modpack (from search_base_modpacks)."),
      mc_version: z.string().optional().describe("Target Minecraft version, used to pick the right pack version."),
      loader: z.string().optional().describe("Target loader, used to pick the right pack version."),
    }),
    // No execute: launcher client runs this through Rust IPC and appends output.
  });
