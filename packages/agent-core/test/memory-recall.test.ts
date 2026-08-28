import { describe, expect, it } from "vitest";

import {
  MEMORY_INPUT_LIMITS,
  renderMemoryContext,
  selectMemoryContext,
  type MemoryCandidate,
  type MemoryRecallRequest,
} from "../src/index";

const identity = { scopeId: "instance-a", conversationId: "conversation-a" };

function candidate(
  id: string,
  overrides: Partial<MemoryCandidate> = {},
): MemoryCandidate {
  return {
    id,
    scopeId: identity.scopeId,
    conversationId: identity.conversationId,
    visibility: "conversation",
    tier: "recall",
    memoryKey: id,
    content: `content for ${id}`,
    updatedAt: "2026-08-28T06:00:00.000Z",
    provenance: { source: "host-test", reference: `record:${id}` },
    ...overrides,
  };
}

function select(candidates: MemoryCandidate[], overrides: Partial<MemoryRecallRequest> = {}) {
  return selectMemoryContext({ identity, candidates, query: "memory performance", ...overrides });
}

describe("bounded memory recall", () => {
  it("publishes the conservative runtime admission limits", () => {
    expect(MEMORY_INPUT_LIMITS).toEqual({
      maxCandidates: 256,
      maxContentChars: 4096,
      maxKeywords: 32,
      maxKeywordChars: 128,
      maxQueryChars: 2048,
      maxIdChars: 256,
      maxScopeIdChars: 256,
      maxConversationIdChars: 256,
      maxMemoryKeyChars: 256,
      maxUpdatedAtChars: 64,
      maxProvenanceSourceChars: 128,
      maxProvenanceReferenceChars: 1024,
    });
  });

  it("rejects malformed runtime candidates without throwing and audits exact reasons", () => {
    const malformed = [
      null,
      "primitive",
      42,
      {},
      candidate("bad-tier", { tier: "archive" as MemoryCandidate["tier"] }),
      candidate("bad-visibility", {
        visibility: "global" as MemoryCandidate["visibility"],
      }),
      candidate("bad-provenance", {
        provenance: null as unknown as MemoryCandidate["provenance"],
      }),
      candidate("bad-keywords-shape", {
        keywords: "memory" as unknown as readonly string[],
      }),
      candidate("bad-keyword-item", {
        keywords: ["memory", null] as unknown as readonly string[],
      }),
    ] as unknown as MemoryCandidate[];

    const forward = selectMemoryContext({ identity, query: "memory", candidates: malformed });
    const reversed = selectMemoryContext({
      identity,
      query: "memory",
      candidates: [...malformed].reverse(),
    });

    expect(forward).toEqual(reversed);
    expect(forward.durable).toEqual([]);
    expect(forward.recalled).toEqual([]);
    expect(forward.decisions).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ candidateId: "bad-tier", reason: "invalid_tier" }),
        expect.objectContaining({
          candidateId: "bad-visibility",
          reason: "invalid_visibility",
        }),
        expect.objectContaining({
          candidateId: "bad-provenance",
          reason: "invalid_provenance",
        }),
        expect.objectContaining({
          candidateId: "bad-keywords-shape",
          reason: "invalid_keywords",
        }),
        expect.objectContaining({
          candidateId: "bad-keyword-item",
          reason: "invalid_keywords",
        }),
      ]),
    );
    expect(forward.decisions.filter((item) => item.reason === "invalid_candidate")).toHaveLength(4);
  });

  it("rejects an oversized candidate array before scanning individual entries", () => {
    const candidates = Array.from({ length: 257 }, (_, index) =>
      candidate(`candidate-${String(index).padStart(3, "0")}`),
    );
    const selection = selectMemoryContext({ identity, query: "memory", candidates });

    expect(selection.durable).toEqual([]);
    expect(selection.recalled).toEqual([]);
    expect(selection.decisions).toEqual([]);
    expect(selection.inputAudit).toContainEqual({
      subject: "candidates",
      outcome: "rejected",
      reason: "candidate_limit_exceeded",
      received: 257,
      accepted: 0,
    });
  });

  it("audits deterministic query truncation and rejects oversized candidate text", () => {
    const candidates = [
      candidate("content-too-long", { content: "c".repeat(4097) }),
      candidate("keyword-too-long", { keywords: ["k".repeat(129)] }),
      candidate("keywords-too-many", {
        keywords: Array.from({ length: 33 }, (_, index) => `keyword-${index}`),
      }),
    ];
    const query = `memory ${"q".repeat(2049)}`;
    const forward = selectMemoryContext({ identity, query, candidates });
    const reversed = selectMemoryContext({ identity, query, candidates: [...candidates].reverse() });

    expect(forward).toEqual(reversed);
    expect(forward.query).toHaveLength(2048);
    expect(forward.inputAudit).toContainEqual({
      subject: "query",
      outcome: "truncated",
      reason: "query_limit_exceeded",
      received: query.length,
      accepted: 2048,
    });
    expect(forward.decisions).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          candidateId: "content-too-long",
          reason: "content_limit_exceeded",
        }),
        expect.objectContaining({
          candidateId: "keyword-too-long",
          reason: "keyword_limit_exceeded",
        }),
        expect.objectContaining({
          candidateId: "keywords-too-many",
          reason: "keyword_count_limit_exceeded",
        }),
      ]),
    );
  });

  it("always considers durable facts and query-gates recalled evidence", () => {
    const selection = select([
      candidate("durable", {
        tier: "durable",
        visibility: "scope",
        conversationId: "origin-conversation",
        memoryKey: "user.language",
        content: "The user prefers Chinese explanations.",
      }),
      candidate("match", {
        memoryKey: "instance.performance",
        content: "Memory pressure caused the last performance regression.",
      }),
      candidate("unrelated", {
        memoryKey: "instance.theme",
        content: "The launcher theme is dark blue.",
      }),
    ]);

    expect(selection.durable.map((item) => item.id)).toEqual(["durable"]);
    expect(selection.recalled.map((item) => item.id)).toEqual(["match"]);
    expect(selection.decisions).toContainEqual(
      expect.objectContaining({ candidateId: "unrelated", reason: "no_query_match" }),
    );
  });

  it("uses newest corrections and removes duplicate evidence", () => {
    const selection = select([
      candidate("old", {
        memoryKey: "instance.memory_mb",
        content: "Allocate 4096 MB.",
        updatedAt: "2026-08-27T06:00:00.000Z",
      }),
      candidate("corrected", {
        memoryKey: "instance.memory_mb",
        content: "Allocate 6144 MB for memory performance.",
        updatedAt: "2026-08-28T06:00:00.000Z",
      }),
      candidate("duplicate", {
        memoryKey: "diagnosis.last_result",
        content: "Allocate 6144 MB for memory performance.",
        updatedAt: "2026-08-27T07:00:00.000Z",
      }),
    ]);

    expect(selection.recalled.map((item) => item.id)).toEqual(["corrected"]);
    expect(selection.decisions).toContainEqual(
      expect.objectContaining({ candidateId: "old", reason: "superseded" }),
    );
    expect(selection.decisions).toContainEqual(
      expect.objectContaining({ candidateId: "duplicate", reason: "duplicate" }),
    );
  });

  it("breaks equal identity and timestamp correction ties without input-order dependence", () => {
    const candidates = [
      candidate("collision", {
        memoryKey: "instance.memory_collision",
        content: "memory alpha",
      }),
      candidate("collision", {
        memoryKey: "instance.memory_collision",
        content: "memory beta",
      }),
    ];
    const forward = select(candidates);
    const reversed = select([...candidates].reverse());

    expect(forward).toEqual(reversed);
    expect(forward.recalled.map((item) => item.content)).toEqual(["memory alpha"]);
  });

  it("never crosses scope and requires explicit conversation visibility", () => {
    const selection = select([
      candidate("other-scope", {
        scopeId: "instance-b",
        content: "Memory performance evidence from another instance.",
      }),
      candidate("other-conversation", {
        conversationId: "conversation-b",
        content: "Memory performance evidence from another conversation.",
      }),
      candidate("scope-visible", {
        conversationId: "conversation-b",
        visibility: "scope",
        tier: "durable",
        content: "Memory performance policy explicitly shared in this scope.",
      }),
    ]);

    expect(selection.durable.map((item) => item.id)).toEqual(["scope-visible"]);
    expect(selection.recalled).toEqual([]);
    expect(selection.decisions).toContainEqual(
      expect.objectContaining({ candidateId: "other-scope", reason: "scope_mismatch" }),
    );
    expect(selection.decisions).toContainEqual(
      expect.objectContaining({
        candidateId: "other-conversation",
        reason: "conversation_mismatch",
      }),
    );
  });

  it("applies deterministic item and character budgets with an audit trail", () => {
    const candidates = [
      candidate("newer", {
        content: "memory performance alpha",
        updatedAt: "2026-08-28T07:00:00.000Z",
      }),
      candidate("older", {
        content: "memory performance beta",
        updatedAt: "2026-08-28T06:00:00.000Z",
      }),
    ];
    const request = {
      budget: { maxChars: 30, maxDurableItems: 1, maxRecallItems: 1 },
    };
    const forward = select(candidates, request);
    const reversed = select([...candidates].reverse(), request);

    expect(forward).toEqual(reversed);
    expect(forward.recalled.map((item) => item.id)).toEqual(["newer"]);
    expect(forward.decisions).toContainEqual(
      expect.objectContaining({ candidateId: "older", reason: "recall_item_limit" }),
    );
    expect(forward.budget).toMatchObject({ selectedChars: 24, remainingChars: 6 });
  });

  it("renders selected provenance and budget without excluded content", () => {
    const selection = select([
      candidate("selected", { content: "memory performance evidence" }),
      candidate("leak", { scopeId: "instance-b", content: "SECRET memory performance" }),
    ]);
    const rendered = renderMemoryContext(selection);

    expect(rendered).toContain('"reference":"record:selected"');
    expect(rendered).toContain("Budget:");
    expect(rendered).not.toContain("SECRET");
  });

  it("matches Chinese recall queries with deterministic Han bigrams", () => {
    const selection = selectMemoryContext({
      identity,
      query: "为什么内存不够？",
      candidates: [
        candidate("zh-memory", {
          content: "上次诊断发现内存上限过低。",
        }),
      ],
    });

    expect(selection.recalled.map((item) => item.id)).toEqual(["zh-memory"]);
  });
});
