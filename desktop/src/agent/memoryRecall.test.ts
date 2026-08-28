import { describe, expect, it, vi } from "vitest";
import type { UIMessage } from "ai";
import type { AgentToolContext } from "./agentContext";
import type { ConversationRecord } from "./chatStore";
import {
  MAX_CONVERSATION_MESSAGES,
  MAX_CONVERSATION_RECORDS,
} from "./conversationHistory";
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

  it("rejects host, instance, and legacy wiki scope disagreement", () => {
    const scoped = context("/game", "pack-a");
    const legacyWiki = {
      root: "/game",
      modpackId: "pack-a",
      instanceId: "pack-a",
      sourcePaths: ["/game/versions/pack-a"],
    };
    expect(buildMemoryRecallRequest({
      conversationId: "current",
      toolContext: { ...scoped, wiki: { ...legacyWiki, instanceId: "pack-b" } },
      records: [],
      isAutomaticTool: automatic,
    })).toBeUndefined();

    const request = buildMemoryRecallRequest({
      conversationId: "current",
      toolContext: scoped,
      records: [
        record("host-root-mismatch", { ...scoped, root: "/other" }, 300, [
          toolMessage("host-root-tool", "diagnose_instance", { leak: "host" }),
        ]),
        record("wiki-mismatch", { ...scoped, wiki: { ...legacyWiki, root: "/other" } }, 200, [
          toolMessage("wiki-tool", "diagnose_instance", { leak: "wiki" }),
        ]),
        record("legacy-exact", { root: "/game", mode: "wiki", wiki: legacyWiki }, 100, [
          toolMessage("legacy-tool", "diagnose_instance", { accepted: true }),
        ]),
      ],
      isAutomaticTool: automatic,
    });

    expect(request?.candidates.map((candidate) => candidate.conversationId)).toEqual([
      "legacy-exact",
    ]);
    expect(JSON.stringify(request)).not.toContain("leak");
  });

  it("consults the automatic allowlist only for final successful tool outputs", () => {
    const scoped = context("/game", "pack-a");
    const allow = vi.fn((name: string) => name === "diagnose_instance");
    const request = buildMemoryRecallRequest({
      conversationId: "current",
      toolContext: scoped,
      records: [record("old", scoped, 100, [
        toolMessage("valid", "diagnose_instance", { accepted: true }),
        toolMessage("interactive", "ask_user_question", { secret: true }),
        toolMessage("error", "wiki_search", { secret: true }, "output-error"),
        preliminaryToolMessage("preliminary", "wiki_search", { secret: true }),
      ])],
      isAutomaticTool: allow,
    });

    expect(allow.mock.calls.map(([name]) => name)).toEqual([
      "diagnose_instance",
      "ask_user_question",
    ]);
    expect(request?.candidates).toHaveLength(1);
    expect(JSON.stringify(request)).not.toContain("secret");
  });

  it("normalizes nested JSON stably, permits shared children, and rejects cycles", () => {
    const scoped = context("/game", "pack-a");
    const shared = { z: 2, a: 1 };
    const cyclic: Record<string, unknown> = {};
    cyclic.self = cyclic;
    const request = buildMemoryRecallRequest({
      conversationId: "current",
      toolContext: scoped,
      records: [record("old", scoped, 100, [
        toolMessage("shared", "diagnose_instance", {
          second: shared,
          missing: undefined,
          first: shared,
          array: [undefined, Number.NaN, shared],
        }),
        toolMessage("cyclic", "diagnose_instance", cyclic),
        toolMessage("undefined", "diagnose_instance", undefined),
      ])],
      isAutomaticTool: automatic,
    });

    expect(request?.candidates.map((candidate) => candidate.content)).toEqual([
      "diagnose_instance output: " +
        '{"array":[null,null,{"a":1,"z":2}],"first":{"a":1,"z":2},' +
        '"second":{"a":1,"z":2}}',
    ]);
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

  it("keeps equal-timestamp selection independent of host locale collation", () => {
    const scoped = context("/game", "pack-a");
    const records = Array.from(
      { length: DESKTOP_MEMORY_LIMITS.maxCandidates + 1 },
      (_, index) => record(`record-${index}`, scoped, 100, [
        toolMessage(`tool-${index}`, "search_mods", { index }),
      ]),
    );
    const selectedWithOrdering = (direction: 1 | -1): string[] => {
      const localeCompare = vi.spyOn(String.prototype, "localeCompare").mockImplementation(
        function compare(this: string, other: string): number {
          if (this === other) return 0;
          return this < other ? direction : -direction;
        },
      );
      try {
        return buildMemoryRecallRequest({
          conversationId: "current",
          toolContext: scoped,
          records,
          isAutomaticTool: automatic,
        })?.candidates.map((candidate) => candidate.content) ?? [];
      } finally {
        localeCompare.mockRestore();
      }
    };

    expect(selectedWithOrdering(1)).toEqual(selectedWithOrdering(-1));
  });

  it("keeps the newest canonical duplicate and excludes invalid timestamps", () => {
    const scoped = context("/game", "pack-a");
    const request = buildMemoryRecallRequest({
      conversationId: "current",
      toolContext: scoped,
      records: [
        record("older", scoped, 100, [
          toolMessage("older-tool", "diagnose_instance", { b: 2, a: 1 }),
        ]),
        record("newer", scoped, 200, [
          toolMessage("newer-tool", "diagnose_instance", { a: 1, b: 2 }),
        ]),
        record("nan", scoped, Number.NaN, [
          toolMessage("nan-tool", "diagnose_instance", { invalid: "nan" }),
        ]),
        record("out-of-range", scoped, Number.MAX_SAFE_INTEGER, [
          toolMessage("range-tool", "diagnose_instance", { invalid: "range" }),
        ]),
      ],
      isAutomaticTool: automatic,
    });

    expect(request?.candidates).toHaveLength(1);
    expect(request?.candidates[0]).toMatchObject({
      conversationId: "newer",
      updatedAt: "1970-01-01T00:00:00.200Z",
      content: 'diagnose_instance output: {"a":1,"b":2}',
    });
    expect(JSON.stringify(request)).not.toContain("invalid");
  });

  it("truncates Unicode content without leaving an unpaired surrogate", () => {
    const scoped = context("/game", "pack-a");
    const request = buildMemoryRecallRequest({
      conversationId: "current",
      toolContext: scoped,
      records: [record("old", scoped, 100, [
        toolMessage("unicode", "search_mods", "😀".repeat(2_000)),
      ])],
      isAutomaticTool: automatic,
    });
    const content = request?.candidates[0]?.content ?? "";
    const suffix = "...[truncated]";
    const beforeSuffix = content.slice(0, -suffix.length);
    const lastCodeUnit = beforeSuffix.charCodeAt(beforeSuffix.length - 1);

    expect(content.length).toBeLessThanOrEqual(DESKTOP_MEMORY_LIMITS.maxContentChars);
    expect(content.endsWith(suffix)).toBe(true);
    expect(lastCodeUnit >= 0xd800 && lastCodeUnit <= 0xdbff).toBe(false);
  });

  it("completes bounded work across the maximum persisted record/message envelope", () => {
    const scoped = context("/game", "pack-a");
    const allow = vi.fn(() => true);
    const records = Array.from({ length: MAX_CONVERSATION_RECORDS }, (_, recordIndex) =>
      record(
        `record-${recordIndex}`,
        scoped,
        recordIndex + 1,
        Array.from({ length: MAX_CONVERSATION_MESSAGES }, (_, messageIndex) =>
          toolMessage(
            `tool-${recordIndex}-${messageIndex}`,
            "search_mods",
            { messageIndex, recordIndex },
          )),
      ));

    const request = buildMemoryRecallRequest({
      conversationId: "current",
      toolContext: scoped,
      records,
      isAutomaticTool: allow,
    });

    expect(allow).toHaveBeenCalledTimes(MAX_CONVERSATION_RECORDS * MAX_CONVERSATION_MESSAGES);
    expect(request?.candidates).toHaveLength(DESKTOP_MEMORY_LIMITS.maxCandidates);
  });
});
