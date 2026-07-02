import { describe, it, expect } from "vitest";

import { createModpackAgent, toolSchemas } from "../src/index";
import { mockExecutor } from "../src/executors/index";
import { startMockServer } from "./fixtures/mockOpenRouter.mjs";

const settings = (baseUrl: string) => ({ apiKey: "test", model: "mock", baseUrl });

describe("runTurn", () => {
  it("(a) streams text_delta then done, and returns grown history", async () => {
    const mock = await startMockServer({ scenario: "text", chunks: 5 });
    try {
      const agent = createModpackAgent(settings(mock.url), mockExecutor());
      const events: { type: string }[] = [];
      const { history, reply } = await agent.runTurn([], "hi", (e) => events.push(e));

      const types = events.map((e) => e.type);
      expect(types).toContain("text_delta");
      expect(types.at(-1)).toBe("done");
      expect(reply.length).toBeGreaterThan(0);
      // history = input (the user message) + the SDK's response messages.
      expect(history.length).toBeGreaterThanOrEqual(2);
      expect(history[0]).toMatchObject({ role: "user" });
    } finally {
      await mock.close();
    }
  });

  it("(b) dispatches a tool call to the executor and feeds the result back", async () => {
    const mock = await startMockServer({
      scenario: "tool",
      toolName: "search_base_modpacks",
      toolArgs: { query: "tech" },
    });
    try {
      const calls: unknown[] = [];
      const exec = mockExecutor({
        search_base_modpacks: async (args) => {
          calls.push(args);
          return { candidates: [] };
        },
      });
      const agent = createModpackAgent(settings(mock.url), exec);
      const events: { type: string }[] = [];
      const { reply } = await agent.runTurn([], "make a tech pack", (e) => events.push(e));

      expect(calls).toHaveLength(1);
      expect(calls[0]).toMatchObject({ query: "tech" });

      const types = events.map((e) => e.type);
      expect(types).toContain("tool_call");
      expect(types).toContain("tool_result");
      expect(types.at(-1)).toBe("done");
      // After the tool round-trip the model streamed a final text answer.
      expect(reply.length).toBeGreaterThan(0);
    } finally {
      await mock.close();
    }
  });

  it("(c) zod rejects malformed tool args", () => {
    // search_base_modpacks requires `query`.
    expect(toolSchemas.search_base_modpacks.safeParse({}).success).toBe(false);
    expect(toolSchemas.search_base_modpacks.safeParse({ query: "x" }).success).toBe(true);
    // search_mods additionally requires mc_version + loader.
    expect(toolSchemas.search_mods.safeParse({ query: "x" }).success).toBe(false);
    expect(
      toolSchemas.search_mods.safeParse({ query: "x", mc_version: "1.20.1", loader: "fabric" })
        .success,
    ).toBe(true);
  });
});
