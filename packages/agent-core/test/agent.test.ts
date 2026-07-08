import { describe, it, expect } from "vitest";
import type { UIMessage } from "ai";

import { buildTools, createModpackAgent, promptForMode, toolSchemas } from "../src/index";
import { startMockServer } from "./fixtures/mockOpenRouter.mjs";

const settings = (baseUrl: string) => ({ apiKey: "test", model: "mock", baseUrl });

const textOf = (m: UIMessage): string =>
  m.parts.map((p) => (p.type === "text" ? p.text : "")).join("");

const userMsg = (text: string): UIMessage => ({ id: "u1", role: "user", parts: [{ type: "text", text }] });

describe("runTurn", () => {
  it("(a) streams a growing assistant UIMessage and returns grown history", async () => {
    const mock = await startMockServer({ scenario: "text", chunks: 5 });
    try {
      const agent = createModpackAgent(settings(mock.url));
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

  it("(b) surfaces tool calls as client-side UIMessage parts", async () => {
    const mock = await startMockServer({
      scenario: "tool",
      toolName: "search_base_modpacks",
      toolArgs: { query: "tech" },
    });
    try {
      const agent = createModpackAgent(settings(mock.url));
      const { messages, error } = await agent.run([userMsg("make a tech pack")], () => {});

      expect(error).toBeUndefined();

      const assistant = messages.at(-1)!;
      const toolParts = assistant.parts.filter(
        (p) => typeof p.type === "string" && p.type.startsWith("tool-"),
      );
      expect(toolParts).toHaveLength(1);
      expect(toolParts[0]).toMatchObject({
        type: "tool-search_base_modpacks",
        toolCallId: "call_mock_1",
        state: "input-available",
        input: { query: "tech" },
      });
      expect(textOf(assistant)).toBe("");
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
    // wiki tools expose only model-owned fields; host-owned paths/ids are injected by the launcher.
    expect(toolSchemas.wiki_search.safeParse({ query: "aether quest", top_k: 3 }).success).toBe(true);
    expect(
      toolSchemas.wiki_search.safeParse({
        query: "andesite alloy",
        kind: "recipe",
        target_id: "create:andesite_alloy",
        ingredient_id: "#forge:nuggets/iron",
        include_structured: true,
      }).success,
    ).toBe(true);
    expect(toolSchemas.wiki_search.safeParse({ top_k: 3 }).success).toBe(false);
    expect(toolSchemas.wiki_search.safeParse({ query: "x", source_paths: ["/tmp"] }).success).toBe(false);
    expect(toolSchemas.wiki_open.safeParse({ chunk_id: "chunk:doc:0:content" }).success).toBe(true);
    expect(toolSchemas.wiki_open.safeParse({ chunk_id: "x", modpack_id: "pack" }).success).toBe(false);
    expect(toolSchemas.wiki_open.safeParse({}).success).toBe(false);
  });

  it("(d) exposes only modpack tools by default", () => {
    expect(Object.keys(buildTools()).sort()).toEqual([
      "ask_user_question",
      "build_modpack",
      "inspect_base_modpack",
      "list_instances",
      "mod_get_detail",
      "resolve_mods",
      "search_base_modpacks",
      "search_mods",
      "show_modpack",
    ]);
  });

  it("(e) exposes only local wiki tools in wiki mode", () => {
    expect(Object.keys(buildTools("wiki")).sort()).toEqual(["wiki_open", "wiki_search"]);
  });

  it("(f) uses a wiki-specific system prompt in wiki mode", () => {
    const prompt = promptForMode("wiki");
    expect(prompt).toContain("wiki_search");
    expect(prompt).toContain("wiki_open");
    expect(prompt).toContain('kind: "recipe"');
    expect(prompt).toContain("recipe_override");
    expect(prompt).toContain("Do not fill gaps with vanilla/Create/default knowledge");
    expect(prompt).not.toContain("build_modpack");
    expect(prompt).not.toContain("search_base_modpacks");
  });
});
