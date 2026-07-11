import { beforeEach, describe, expect, it, vi } from "vitest";

const ipc = vi.hoisted(() => ({
  hydrate: vi.fn(),
  save: vi.fn(),
}));

vi.mock("../ipc/bindings", () => ({
  commands: {
    agentHistoryHydrate: ipc.hydrate,
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
});
