import { describe, expect, it } from "vitest";

import { parseSerializedConversationRecords } from "./conversationHistory";

describe("native conversation history payloads", () => {
  it("parses the JSON-string records returned by the local history IPC", () => {
    const raw = JSON.stringify([
      JSON.stringify({
        id: "chat-restarted",
        createdAt: 1,
        updatedAt: 2,
        title: "restored",
        messages: [],
      }),
    ]);

    expect(parseSerializedConversationRecords(raw)).toEqual([
      {
        id: "chat-restarted",
        createdAt: 1,
        updatedAt: 2,
        title: "restored",
        messages: [],
      },
    ]);
  });

  it("rejects object arrays because they are not the native IPC contract", () => {
    expect(
      parseSerializedConversationRecords(
        JSON.stringify([{ id: "chat-wrong-shape", messages: [] }]),
      ),
    ).toEqual([]);
  });
});
