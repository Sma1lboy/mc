import { convertToModelMessages, type UIMessage } from "ai";
import { describe, expect, it } from "vitest";
import type { ConversationRecord } from "./chatStore";
import {
  CONVERSATION_PRIVACY_VERSION,
  projectConversationForPersistence,
  projectMessagesForPublicShare,
} from "./conversationPrivacy";

function fixtureRecord(): ConversationRecord {
  return {
    id: "chat-1",
    createdAt: 1,
    updatedAt: 2,
    title: "Open /Users/alice/Games password=hunter2",
    toolContext: {
      mode: "instance",
      root: "/Users/alice/Games",
      instance: {
        root: "/Users/alice/Games",
        modpackId: "pack-1",
        instanceId: "instance-1",
        sourcePaths: ["/Users/alice/Games/instances/instance-1"],
        mcVersion: "1.20.1",
        loader: "fabric",
      },
    },
    messages: [
      {
        id: "user-1",
        role: "user",
        parts: [{ type: "text", text: "Read /Users/alice/Games/instance.json" }],
      },
      {
        id: "assistant-1",
        role: "assistant",
        parts: [
          { type: "reasoning", text: "private chain of thought" },
          {
            type: "tool-wiki_search",
            toolCallId: "call-1",
            state: "output-available",
            input: { query: "difficulty", source_path: "/Users/alice/Games/instances/instance-1" },
            output: {
              answer: "difficulty = hard",
              api_key: "sk-test-private-value",
              log_tail: "long private log",
              output_path: "C:\\Users\\alice\\pack.mrpack",
              download_url: "https://example.invalid/private.jar",
              sha1: "a".repeat(40),
              session_id: "diagnosis-private",
            },
          },
          { type: "text", text: "The configured difficulty is hard." },
        ],
      },
    ] as UIMessage[],
  };
}

describe("conversation privacy projection", () => {
  it("removes local release data without mutating the live conversation", () => {
    const record = fixtureRecord();
    const original = structuredClone(record);
    const projected = projectConversationForPersistence(record);
    const serialized = JSON.stringify(projected);

    expect(record).toEqual(original);
    expect(projected.privacyVersion).toBe(CONVERSATION_PRIVACY_VERSION);
    expect(projected.toolContext).toEqual({
      mode: "instance",
      instance: {
        modpackId: "pack-1",
        instanceId: "instance-1",
        mcVersion: "1.20.1",
        loader: "fabric",
      },
    });
    expect(serialized).not.toContain("private chain of thought");
    expect(serialized).not.toContain("/Users/alice");
    expect(serialized).not.toContain("hunter2");
    expect(serialized).not.toContain("C:\\\\Users");
    expect(serialized).not.toContain("sk-test-private-value");
    expect(serialized).not.toContain("long private log");
    expect(serialized).not.toContain("diagnosis-private");
    expect(serialized).not.toContain("difficulty = hard");
    expect(serialized).toContain("The configured difficulty is hard.");
    expect(serialized).toContain("instance_content_not_persisted");
    expect(serialized).toContain("[REDACTED]");
  });

  it("keeps completed tool call/result pairs convertible by the AI SDK", async () => {
    const projected = projectConversationForPersistence(fixtureRecord());

    const modelMessages = await convertToModelMessages(projected.messages);

    expect(modelMessages.some((message) => message.role === "assistant")).toBe(true);
    expect(modelMessages.some((message) => message.role === "tool")).toBe(true);
  });

  it("drops unknown legacy fields and preserves Minecraft commands", () => {
    const record = {
      ...fixtureRecord(),
      title: "Use /give @p minecraft:diamond",
      debugSnapshot: {
        home: "/Users/alice/private",
        authorization: "Bearer private-value",
      },
    } as ConversationRecord & { debugSnapshot: unknown };

    const projected = projectConversationForPersistence(record);
    const serialized = JSON.stringify(projected);

    expect(projected.title).toBe("Use /give @p minecraft:diamond");
    expect(serialized).not.toContain("debugSnapshot");
    expect(serialized).not.toContain("private-value");
  });

  it("keeps author metadata while redacting credential-suffixed keys", () => {
    const record = fixtureRecord();
    record.messages[0] = {
      ...record.messages[0],
      metadata: {
        author: "Guide Writer",
        apiToken: "private-token",
      },
    } as UIMessage;

    const projected = projectConversationForPersistence(record);
    const metadata = (projected.messages[0] as UIMessage & {
      metadata: Record<string, unknown>;
    }).metadata;

    expect(metadata.author).toBe("Guide Writer");
    expect(metadata.apiToken).toBe("[REDACTED]");
  });

  it("creates a display-only public share", () => {
    const messages = fixtureRecord().messages.concat(
      {
        id: "user-paths",
        role: "user",
        parts: [
          {
            type: "text",
            text: "Use /give, not C:/Users/Alice/Games/pack or \\\\nas\\alice\\pack or ~/Library/private",
          },
        ],
      } as UIMessage,
      {
        id: "assistant-2",
        role: "assistant",
        parts: [
          {
            type: "tool-ask_user_question",
            toolCallId: "ask-1",
            state: "output-available",
            input: {
              question: "Choose a mode",
              options: [{
                label: "Hard",
                description: "See https://example.invalid/private",
                secret: "hidden-option-field",
              }],
              metadata: "hidden-input-field",
            },
            output: { selected: ["Hard"], token: "hidden-output-field" },
          },
        ],
      } as UIMessage,
    );

    const projected = projectMessagesForPublicShare(messages);
    const serialized = JSON.stringify(projected);

    expect(projected.map((message) => message.role)).toEqual([
      "user",
      "assistant",
      "user",
      "assistant",
    ]);
    expect(serialized).toContain("The configured difficulty is hard.");
    expect(serialized).toContain("Use /give");
    expect(serialized).not.toContain("C:/Users");
    expect(serialized).not.toContain("\\\\nas");
    expect(serialized).not.toContain("~/Library");
    expect(serialized).toContain("Choose a mode");
    expect(serialized).not.toContain("tool-wiki_search");
    expect(serialized).not.toContain("private chain of thought");
    expect(serialized).not.toContain("/Users/alice");
    expect(serialized).not.toContain("https://example.invalid");
    expect(serialized).not.toContain("\"id\"");
    expect(serialized).not.toContain("toolCallId");
    expect(serialized).not.toContain("hidden-option-field");
    expect(serialized).not.toContain("hidden-input-field");
    expect(serialized).not.toContain("hidden-output-field");
  });
});
