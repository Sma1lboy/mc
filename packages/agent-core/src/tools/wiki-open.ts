import { tool } from "ai";
import { z } from "zod";

export const wikiOpen = () =>
  tool({
    description:
      "Open the full content and structured data of a wiki chunk returned by wiki_search. Use this when a search snippet or structured hit is not enough to answer accurately. The chunk_id must come from a wiki_search result in this conversation.",
    inputSchema: z
      .object({
        chunk_id: z.string().describe("A chunk_id returned by wiki_search."),
      })
      .strict(),
    // No execute: launcher client injects instance context and runs this through Rust IPC.
  });
