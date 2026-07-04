import { tool } from "ai";
import { z } from "zod";

export const searchBaseModpacks = () =>
  tool({
    description:
      "Search for existing Minecraft modpacks (on Modrinth) that could be used as a base pack. Returns real candidates with provider, project_id, slug, title, author, downloads, and description. Use English keywords.",
    inputSchema: z.object({
      query: z.string().describe("English search keywords describing the desired modpack."),
      mc_version: z
        .string()
        .optional()
        .describe('Target Minecraft version, e.g. "1.20.1". Omit to search all versions.'),
      loader: z
        .string()
        .optional()
        .describe('Target loader, e.g. "fabric" / "quilt" / "forge" / "neoforge".'),
    }),
    // No execute: launcher client runs this through Rust IPC and appends output.
  });
