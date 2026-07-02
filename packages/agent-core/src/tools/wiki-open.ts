import { tool } from "ai";
import { z } from "zod";

import type { AgentToolContext, ToolExecutor } from "../types";

export const wikiOpen = (exec: ToolExecutor, context?: AgentToolContext) =>
  tool({
    description:
      "Open one wiki/knowledge chunk from the current modpack corpus by chunk_id returned from wiki_search. Use when a search hit is relevant but the snippet is insufficient.",
    inputSchema: z
      .object({
        chunk_id: z.string().describe('A chunk_id returned by wiki_search, e.g. "chunk:0:0".'),
      })
      .strict(),
    execute: (args) => {
      const wiki = context?.wiki;
      if (!wiki?.modpackId) {
        throw new Error("wiki_open requires a host-provided current modpack wiki context.");
      }
      return exec.wiki_open({
        ...args,
        modpack_id: wiki.modpackId,
        instance_id: wiki.instanceId,
        source_paths: wiki.sourcePaths,
      });
    },
  });
