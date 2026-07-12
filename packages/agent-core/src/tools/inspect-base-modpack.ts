import { tool } from "ai";
import { z } from "zod";

export const inspectBaseModpack = () =>
  tool({
    description:
      "Inspect a base modpack: select an exact compatible version, download its archive, list the mods it already includes, and report the feature categories it covers. Returns the trusted version_id to pass unchanged to validate_modpack_plan, confirm_modpack_build, or show_modpack.",
    inputSchema: z.object({
      project_id: z.string().describe("Modrinth project id of the base modpack (from search_base_modpacks)."),
      mc_version: z.string().optional().describe("Target Minecraft version, used to pick the right pack version."),
      loader: z.string().optional().describe("Target loader, used to pick the right pack version."),
    }),
    // No execute: launcher client runs this through Rust IPC and appends output.
  });
