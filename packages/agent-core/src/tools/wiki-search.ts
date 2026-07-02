import { tool } from "ai";
import { z } from "zod";

import type { AgentToolContext, ToolExecutor } from "../types";

export const wikiSearch = (exec: ToolExecutor, context?: AgentToolContext) =>
  tool({
    description:
      "Search the current modpack's scoped wiki/knowledge corpus. Use before answering factual questions about the current modpack. The host injects modpack scope and source selection; do not pass modpack ids or paths.",
    inputSchema: z
      .object({
        query: z
          .string()
          .describe('Short search query for the current modpack knowledge base, e.g. "aether portal".'),
        top_k: z
          .number()
          .int()
          .min(1)
          .max(8)
          .optional()
          .describe("Maximum hits to return. Defaults to 5 and cannot exceed 8."),
      })
      .strict(),
    execute: (args) => {
      const wiki = context?.wiki;
      if (!wiki?.modpackId) {
        throw new Error("wiki_search requires a host-provided current modpack wiki context.");
      }
      return exec.wiki_search({
        ...args,
        modpack_id: wiki.modpackId,
        instance_id: wiki.instanceId,
        source_paths: wiki.sourcePaths,
      });
    },
  });
