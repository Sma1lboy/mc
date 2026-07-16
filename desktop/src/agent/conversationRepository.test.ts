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

import { conversationRepository } from "./conversationRepository";

describe("conversation repository", () => {
  beforeEach(() => vi.resetAllMocks());

  it("hydrates object records from the native serialized transport", async () => {
    const record = {
      id: "chat-native",
      createdAt: 1,
      updatedAt: 2,
      title: "restored",
      messages: [],
    };
    ipc.hydrate.mockResolvedValue({ status: "ok", data: JSON.stringify([JSON.stringify(record)]) });

    const [migrated] = await conversationRepository.hydrate(null);

    expect(migrated).toMatchObject({
      id: record.id,
      createdAt: record.createdAt,
      title: record.title,
      messages: record.messages,
      toolContext: null,
      privacyVersion: 1,
    });
    expect(migrated.updatedAt).toBeGreaterThan(record.updatedAt);
    expect(ipc.save).toHaveBeenCalledOnce();
  });

  it("treats a failed host hydration as an empty local result", async () => {
    ipc.hydrate.mockRejectedValue(new Error("command missing"));

    await expect(conversationRepository.hydrate(null)).resolves.toEqual([]);
  });

  it("uses the host sync command only for remote reconciliation", async () => {
    const record = {
      id: "chat-remote",
      createdAt: 1,
      updatedAt: 2,
      title: "synced",
      messages: [],
    };
    ipc.sync.mockResolvedValue({ status: "ok", data: JSON.stringify([JSON.stringify(record)]) });

    const synced = await conversationRepository.sync("user-1");
    expect(synced).not.toBeNull();
    const [migrated] = synced ?? [];

    expect(migrated).toMatchObject({
      id: record.id,
      createdAt: record.createdAt,
      title: record.title,
      messages: record.messages,
      toolContext: null,
      privacyVersion: 1,
    });
    expect(migrated.updatedAt).toBeGreaterThan(record.updatedAt);
    expect(ipc.hydrate).not.toHaveBeenCalled();
    expect(ipc.sync).toHaveBeenCalledWith("user-1");
    expect(ipc.save).toHaveBeenCalledOnce();
  });

  it("distinguishes sync failure from a successful empty history", async () => {
    ipc.sync.mockRejectedValue(new Error("server unavailable"));

    await expect(conversationRepository.sync("user-1")).resolves.toBeNull();
  });

  it("does not rewrite records already at the current privacy version", async () => {
    const record = {
      id: "chat-current",
      createdAt: 1,
      updatedAt: 2,
      title: "current",
      messages: [],
      privacyVersion: 1,
    };
    ipc.hydrate.mockResolvedValue({ status: "ok", data: JSON.stringify([JSON.stringify(record)]) });

    await expect(conversationRepository.hydrate(null)).resolves.toEqual([record]);
    expect(ipc.save).not.toHaveBeenCalled();
  });

  it("projects private runtime context before saving", () => {
    conversationRepository.save({
      id: "chat-private",
      createdAt: 1,
      updatedAt: 2,
      title: "private",
      messages: [],
      ownerId: "user-1",
      toolContext: {
        mode: "instance",
        instance: {
          root: "/Users/alice/Games",
          modpackId: "pack",
          instanceId: "instance",
          sourcePaths: ["/Users/alice/Games/instance"],
          mcVersion: "1.20.1",
          loader: "fabric",
        },
      },
    }, "user-1");

    const payload = ipc.save.mock.calls[0]?.[1] as string;
    expect(ipc.save).toHaveBeenCalledWith("chat-private", payload, "user-1");
    expect(payload).not.toContain("/Users/alice");
    expect(JSON.parse(payload)).toMatchObject({
      privacyVersion: 1,
      toolContext: {
        mode: "instance",
        instance: {
          modpackId: "pack",
          instanceId: "instance",
          mcVersion: "1.20.1",
          loader: "fabric",
        },
      },
    });
  });
});
