import { describe, it, expect } from "vitest";
import type { UIMessage } from "ai";

import { createModpackAgent, toolSchemas } from "../src/index";
import { mockExecutor } from "../src/executors/index";
import { startMockServer } from "./fixtures/mockOpenRouter.mjs";

const settings = (baseUrl: string) => ({ apiKey: "test", model: "mock", baseUrl });

const textOf = (m: UIMessage): string =>
  m.parts.map((p) => (p.type === "text" ? p.text : "")).join("");

const userMsg = (text: string): UIMessage => ({ id: "u1", role: "user", parts: [{ type: "text", text }] });

describe("runTurn", () => {
  it("(a) streams a growing assistant UIMessage and returns grown history", async () => {
    const mock = await startMockServer({ scenario: "text", chunks: 5 });
    try {
      const agent = createModpackAgent(settings(mock.url), mockExecutor());
      const updates: UIMessage[] = [];
      const { messages, error } = await agent.run([userMsg("hi")], (m) => updates.push(m));

      expect(error).toBeUndefined();
      expect(updates.length).toBeGreaterThan(0); // streamed incrementally
      // history = the user message + the streamed assistant message.
      expect(messages.length).toBeGreaterThanOrEqual(2);
      expect(messages[0]).toMatchObject({ role: "user" });
      const assistant = messages.at(-1)!;
      expect(assistant.role).toBe("assistant");
      expect(textOf(assistant).length).toBeGreaterThan(0);
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
      const { messages, error } = await agent.run([userMsg("make a tech pack")], () => {});

      expect(error).toBeUndefined();
      expect(calls).toHaveLength(1);
      expect(calls[0]).toMatchObject({ query: "tech" });

      const assistant = messages.at(-1)!;
      // the turn carried a tool part (type "tool-<name>") and a final text answer.
      const toolParts = assistant.parts.filter(
        (p) => typeof p.type === "string" && p.type.startsWith("tool-"),
      );
      expect(toolParts.length).toBeGreaterThan(0);
      expect(textOf(assistant).length).toBeGreaterThan(0);
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
