import { tool } from "ai";
import { z } from "zod";

import type { ToolExecutor } from "../types";

export const searchMods = (exec: ToolExecutor) =>
  tool({
    description:
      "Search for individual Minecraft mods compatible with a Minecraft version + loader. Returns real candidates with provider, project_id, slug, title, downloads, and description. Use English keywords.",
    inputSchema: z.object({
      query: z.string().describe("English search keywords for the mod / feature to find."),
      mc_version: z.string().describe('Target Minecraft version, e.g. "1.20.1".'),
      loader: z.string().describe('Target loader, e.g. "fabric".'),
    }),
    execute: (args) => exec.search_mods(args),
  });
