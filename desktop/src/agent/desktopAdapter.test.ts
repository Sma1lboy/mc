import { beforeEach, describe, expect, it, vi } from "vitest";
import type { MemoryRecallRequest } from "@kobemc/agent-core";
import type { UIMessage } from "ai";
import type {
  AgentProviderRunRequest,
  AgentProviderSession,
} from "./runCoordinator";

const mocks = vi.hoisted(() => ({
  agentRun: vi.fn(),
  createAgent: vi.fn(),
}));

vi.mock("../ipc/bindings", () => ({
  commands: {
    agentLlmConfig: vi.fn(async () => ({
      status: "ok",
      data: { api_key: "test", model: "test", base_url: "https://example.test" },
    })),
  },
}));

vi.mock("@kobemc/agent-core", () => ({
  createModpackAgent: mocks.createAgent,
}));

vi.mock("./clientToolDispatcher", () => ({
  unwrap: async (promise: Promise<unknown>) => {
    const result = await promise as { status: string; data?: unknown; error?: string };
    if (result.status === "error") throw new Error(result.error);
    return result.data;
  },
}));

import { createDesktopAgent } from "./desktopAdapter";

function memory(): MemoryRecallRequest {
  return {
    identity: { scopeId: "instance:[\"/game\",\"pack\"]", conversationId: "current" },
    candidates: [{
      id: "tool-output:old:0:0",
      scopeId: "instance:[\"/game\",\"pack\"]",
      conversationId: "old",
      visibility: "scope",
      tier: "recall",
      memoryKey: "automatic-tool:diagnose_instance",
      content: "diagnose_instance output: {\"status\":\"healthy\"}",
      updatedAt: "2026-08-28T08:00:00.000Z",
      provenance: {
        source: "launcher.conversation.automatic_tool_output",
        reference: "conversation=old;message=a;toolCall=t;tool=diagnose_instance",
      },
    }],
  };
}

describe("OpenRouter desktop adapter memory bridge", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.agentRun.mockResolvedValue({ messages: [] });
    mocks.createAgent.mockReturnValue({ run: mocks.agentRun });
  });

  it("forwards host-built candidates to agent-core turn admission", async () => {
    const session = await createDesktopAgent("instance");
    const history: UIMessage[] = [{
      id: "user-current",
      role: "user",
      parts: [{ type: "text", text: "is it still healthy?" }],
    }];
    const abortController = new AbortController();
    const recall = memory();
    const request = {
      binding: {
        conversationId: "current",
        runId: "run-current",
        providerSession: {} as AgentProviderSession,
        toolContext: null,
        abortController,
      },
      history,
      onUpdate: vi.fn(),
      signal: abortController.signal,
      memory: recall,
    } as AgentProviderRunRequest & { memory: MemoryRecallRequest };

    await session.run(request);

    expect(mocks.agentRun).toHaveBeenCalledWith(
      history,
      request.onUpdate,
      request.signal,
      { memory: recall },
    );
  });
});
