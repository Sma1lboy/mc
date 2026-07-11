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

    await expect(conversationRepository.hydrate()).resolves.toEqual([record]);
  });

  it("treats a failed host hydration as an empty local result", async () => {
    ipc.hydrate.mockRejectedValue(new Error("command missing"));

    await expect(conversationRepository.hydrate()).resolves.toEqual([]);
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

    await expect(conversationRepository.sync()).resolves.toEqual([record]);
    expect(ipc.hydrate).not.toHaveBeenCalled();
  });
});
