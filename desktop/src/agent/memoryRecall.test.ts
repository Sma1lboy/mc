import { describe, expect, it } from "vitest";
import type { UIMessage } from "ai";
import type { AgentToolContext } from "./agentContext";
import type { ConversationRecord } from "./chatStore";
import {
  DESKTOP_MEMORY_LIMITS,
  buildMemoryRecallRequest,
} from "./memoryRecall";

function context(root: string, instanceId: string): AgentToolContext {
  return {
    root,
    mode: "instance",
    instance: {
      root,
      modpackId: instanceId,
      instanceId,
      sourcePaths: [`${root}/versions/${instanceId}`],
      mcVersion: "1.20.1",
      loader: "fabric",
    },
  };
}

function toolMessage(
  id: string,
  name: string,
  output: unknown,
  state = "output-available",
): UIMessage {
  return {
    id,
    role: "assistant",
    parts: [{
      type: `tool-${name}`,
      toolCallId: `call-${id}`,
      state,
      input: {},
      output,
    }],
  } as UIMessage;
}

function preliminaryToolMessage(id: string, name: string, output: unknown): UIMessage {
  const message = toolMessage(id, name, output);
  return {
    ...message,
    parts: message.parts.map((part) => ({ ...part, preliminary: true })),
  } as UIMessage;
}

function record(
  id: string,
  toolContext: AgentToolContext | null,
  updatedAt: number,
  messages: UIMessage[],
): ConversationRecord {
  return { id, createdAt: updatedAt - 1, updatedAt, title: id, messages, toolContext };
}

const automatic = (name: string): boolean =>
  ["diagnose_instance", "wiki_search", "search_mods"].includes(name);

describe("desktop deterministic memory recall candidates", () => {
  it("extracts only completed automatic outputs from another conversation in the exact instance scope", () => {
    const current = context("/game", "pack-a");
    const request = buildMemoryRecallRequest({
      conversationId: "current",
      toolContext: current,
      records: [
        record("current", current, 400, [toolMessage("current-tool", "diagnose_instance", { leak: true })]),
        record("same-scope", current, 300, [
          { id: "user-prose", role: "user", parts: [{ type: "text", text: "raw user secret" }] },
          { id: "assistant-prose", role: "assistant", parts: [{ type: "text", text: "free model prose" }] },
          toolMessage("diagnosis", "diagnose_instance", { status: "healthy" }),
          toolMessage("interactive", "ask_user_question", { selected: ["yes"] }),
          toolMessage("failed", "wiki_search", { should: "not appear" }, "output-error"),
          preliminaryToolMessage("preliminary", "wiki_search", { should: "not appear" }),
        ]),
        record("other-instance", context("/game", "pack-b"), 500, [
          toolMessage("other-instance-tool", "diagnose_instance", { leak: "instance" }),
        ]),
        record("other-root", context("/other", "pack-a"), 600, [
          toolMessage("other-root-tool", "diagnose_instance", { leak: "root" }),
        ]),
      ],
      isAutomaticTool: automatic,
    });

    expect(request?.identity).toEqual({
      scopeId: 'instance:["/game","pack-a"]',
      conversationId: "current",
    });
    expect(request?.candidates).toHaveLength(1);
    expect(request?.candidates[0]).toMatchObject({
      conversationId: "same-scope",
      visibility: "scope",
      tier: "recall",
      content: 'diagnose_instance output: {"status":"healthy"}',
      provenance: {
        source: "launcher.conversation.automatic_tool_output",
        reference: expect.stringContaining("tool=diagnose_instance"),
      },
    });
    expect(JSON.stringify(request)).not.toContain("raw user secret");
    expect(JSON.stringify(request)).not.toContain("free model prose");
    expect(JSON.stringify(request)).not.toContain("leak");
  });

  it("fails closed when a stable instance-and-root scope cannot be proven", () => {
    const scoped = context("/game", "pack-a");
    expect(buildMemoryRecallRequest({
      conversationId: "current",
      toolContext: { root: "/game" },
      records: [record("old", scoped, 1, [toolMessage("tool", "diagnose_instance", {})])],
      isAutomaticTool: automatic,
    })).toBeUndefined();
    expect(buildMemoryRecallRequest({
      conversationId: "current",
      toolContext: { ...scoped, root: "/different" },
      records: [],
      isAutomaticTool: automatic,
    })).toBeUndefined();
    expect(buildMemoryRecallRequest({
      conversationId: "current",
      toolContext: context("/game ", "pack-a"),
      records: [],
      isAutomaticTool: automatic,
    })).toBeUndefined();
  });

  it("deduplicates stable outputs and bounds candidate count and per-item content before transport", () => {
    const scoped = context("/game", "pack-a");
    const records = [
      record("duplicate-old", scoped, 100, [
        toolMessage("duplicate-old-tool", "diagnose_instance", { status: "same" }),
      ]),
      record("duplicate-new", scoped, 200, [
        toolMessage("duplicate-new-tool", "diagnose_instance", { status: "same" }),
      ]),
      ...Array.from({ length: DESKTOP_MEMORY_LIMITS.maxCandidates + 4 }, (_, index) =>
        record(`generic-${index}`, scoped, 300 + index, [
          toolMessage(`generic-tool-${index}`, "search_mods", { index, text: "x".repeat(4_000) }),
        ])),
      record("diagnosis-priority", scoped, 50, [
        toolMessage("diagnosis-priority-tool", "diagnose_instance", { status: "priority" }),
      ]),
    ];

    const request = buildMemoryRecallRequest({
      conversationId: "current",
      toolContext: scoped,
      records,
      isAutomaticTool: automatic,
    });

    expect(request?.candidates).toHaveLength(DESKTOP_MEMORY_LIMITS.maxCandidates);
    expect(request?.candidates.every(
      (candidate) => candidate.content.length <= DESKTOP_MEMORY_LIMITS.maxContentChars,
    )).toBe(true);
    expect(request?.candidates.filter(
      (candidate) => candidate.content.includes('{"status":"same"}'),
    )).toHaveLength(1);
    expect(request?.candidates.some(
      (candidate) => candidate.content.includes('{"status":"priority"}'),
    )).toBe(true);
    expect(buildMemoryRecallRequest({
      conversationId: "current",
      toolContext: scoped,
      records: [...records].reverse(),
      isAutomaticTool: automatic,
    })).toEqual(request);
  });
});
