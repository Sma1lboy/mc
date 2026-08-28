import { beforeEach, describe, expect, it, vi } from "vitest";

const ipc = vi.hoisted(() => ({
  hydrate: vi.fn(),
  sync: vi.fn(),
  save: vi.fn(),
}));

vi.mock("../ipc/bindings", () => ({
  commands: {
    agentHistoryHydrate: ipc.hydrate,
    agentHistorySync: ipc.sync,
    agentHistorySave: ipc.save,
  },
}));

import { conversationRepository, mergeConversationRecords } from "./conversationRepository";

function record(
  id: string,
  updatedAt: number,
  title = id,
  toolContext: { root: string } | null = null,
) {
  return {
    id,
    createdAt: 1,
    updatedAt,
    title,
    messages: [
      {
        id: `${title}-message`,
        role: "user" as const,
        parts: [{ type: "text" as const, text: title }],
      },
    ],
    toolContext,
  };
}

function serialize(records: unknown[]): string {
  return JSON.stringify(records.map((value) => JSON.stringify(value)));
}

describe("conversation repository", () => {
  beforeEach(() => vi.resetAllMocks());

  it("hydrates object records from the native serialized transport", async () => {
    const nativeRecord = record("chat-native", 2, "restored");
    ipc.hydrate.mockResolvedValue({ status: "ok", data: serialize([nativeRecord]) });

    await expect(conversationRepository.hydrate()).resolves.toEqual([nativeRecord]);
  });

  it("treats a failed host hydration as an empty local result", async () => {
    ipc.hydrate.mockRejectedValue(new Error("command missing"));

    await expect(conversationRepository.hydrate()).resolves.toEqual([]);
  });

  it("uses the host sync command only for remote reconciliation", async () => {
    const remoteRecord = record("chat-remote", 2, "synced");
    ipc.sync.mockResolvedValue({ status: "ok", data: serialize([remoteRecord]) });

    await expect(conversationRepository.sync()).resolves.toEqual([remoteRecord]);
    expect(ipc.hydrate).not.toHaveBeenCalled();
  });

  it("persists only records that pass the conversation boundary", async () => {
    const valid = record("chat-save", 2, "saved");
    const invalid = {
      ...record("chat-invalid-save", 2),
      messages: [{ id: "bad", role: "user", parts: [{ type: "text", text: 42 }] }],
    };

    conversationRepository.save(invalid as never);
    conversationRepository.save(valid);

    await vi.waitFor(() => expect(ipc.save).toHaveBeenCalledTimes(1));
    expect(ipc.save).toHaveBeenCalledWith(valid.id, JSON.stringify(valid));
  });

  it("keeps the newest whole record without mixing cross-record fields", () => {
    const stale = record("chat-shared", 2, "stale", { root: "/stale" });
    const newest = record("chat-shared", 3, "newest", { root: "/newest" });

    expect(mergeConversationRecords([newest], [stale])).toEqual([newest]);
    expect(mergeConversationRecords([stale], [newest])).toEqual([newest]);
  });

  it("normalizes duplicates and converges for equal-timestamp conflicts", () => {
    const alpha = record("chat-shared", 3, "alpha");
    const omega = record("chat-shared", 3, "omega");
    const chatA = record("chat-a", 3);
    const chatZ = record("chat-z", 3);

    const left = mergeConversationRecords([alpha, chatA, alpha], [omega, chatZ, omega]);
    const right = mergeConversationRecords([omega, chatZ, omega], [alpha, chatA, alpha]);

    expect(left).toEqual(right);
    expect(left.filter((value) => value.id === "chat-shared")).toHaveLength(1);
    expect(left.map((value) => value.id)).toEqual(["chat-z", "chat-shared", "chat-a"]);
  });
});
