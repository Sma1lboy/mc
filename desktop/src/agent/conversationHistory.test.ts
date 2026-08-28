import { describe, expect, it } from "vitest";

import {
  MAX_CONVERSATION_MESSAGES,
  MAX_CONVERSATION_RECORD_BYTES,
  MAX_CONVERSATION_RECORDS,
  parseSerializedConversationRecords,
} from "./conversationHistory";

function message(id: string, text = "hello") {
  return { id, role: "user", parts: [{ type: "text", text }] };
}

function record(id: string, updatedAt = 2) {
  return {
    id,
    createdAt: 1,
    updatedAt,
    title: id,
    messages: [message(`${id}-message`)],
  };
}

function serialize(records: unknown[]): string {
  return JSON.stringify(records.map((value) => JSON.stringify(value)));
}

describe("native conversation history payloads", () => {
  it("parses the JSON-string records returned by the local history IPC", async () => {
    const restored = record("chat-restarted");
    restored.title = "restored";

    await expect(parseSerializedConversationRecords(serialize([restored]))).resolves.toEqual([
      restored,
    ]);
  });

  it("rejects object arrays because they are not the native IPC contract", async () => {
    await expect(
      parseSerializedConversationRecords(
        JSON.stringify([record("chat-wrong-shape")]),
      ),
    ).resolves.toEqual([]);
  });

  it("keeps valid records while rejecting malformed records and AI messages", async () => {
    const valid = record("chat-valid");
    const malformedRecord = { ...record("chat-malformed"), updatedAt: "yesterday" };
    const malformedMessage = {
      ...record("chat-bad-message"),
      messages: [{ id: "bad", role: "user", parts: [{ type: "text", text: 42 }] }],
    };
    const malformedContext = {
      ...record("chat-bad-context"),
      toolContext: { root: 42 },
    };

    await expect(
      parseSerializedConversationRecords(
        serialize([malformedRecord, valid, malformedMessage, malformedContext]),
      ),
    ).resolves.toEqual([valid]);
  });

  it("fails closed when the native transport exceeds its record count", async () => {
    const records = Array.from({ length: MAX_CONVERSATION_RECORDS + 1 }, (_, index) =>
      JSON.stringify(record(`chat-${index}`)),
    );

    await expect(parseSerializedConversationRecords(JSON.stringify(records))).resolves.toEqual([]);
  });

  it("skips records whose message count or serialized size exceeds the limits", async () => {
    const tooManyMessages = {
      ...record("chat-too-many-messages"),
      messages: Array.from({ length: MAX_CONVERSATION_MESSAGES + 1 }, (_, index) =>
        message(`message-${index}`),
      ),
    };
    const oversized = {
      ...record("chat-oversized"),
      title: "x".repeat(MAX_CONVERSATION_RECORD_BYTES),
    };
    const valid = record("chat-valid");

    await expect(
      parseSerializedConversationRecords(serialize([tooManyMessages, oversized, valid])),
    ).resolves.toEqual([valid]);
  });

  it("deduplicates records by id with deterministic newest-wins selection", async () => {
    const stale = record("chat-duplicate", 2);
    stale.title = "stale";
    const newest = record("chat-duplicate", 3);
    newest.title = "newest";

    await expect(
      parseSerializedConversationRecords(serialize([newest, stale, newest])),
    ).resolves.toEqual([newest]);
  });
});
