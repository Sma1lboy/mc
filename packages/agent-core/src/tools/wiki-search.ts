import { tool } from "ai";
import { z } from "zod";

export const wikiSearch = () =>
  tool({
    description:
      "Search the current installed modpack's local wiki/config/quest corpus. Use this for questions about the user's current instance, progression, quests, config, included files, or pack-specific behavior. The launcher host injects the current modpack id and local source paths; you only provide the user's query.",
    inputSchema: z
      .object({
        query: z.string().describe("Search terms for the current installed modpack's local corpus."),
        top_k: z
          .number()
          .int()
          .min(1)
          .max(8)
          .optional()
          .describe("Maximum number of chunks to return. Omit for the default."),
      })
      .strict(),
    // No execute: launcher client injects instance context and runs this through Rust IPC.
  });
