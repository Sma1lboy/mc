import { expect, it, vi } from "vitest";
import { forwardClientToolCall } from "../src/harness/index";

it("forwards the AI SDK toolCallId to the client bridge handler", async () => {
  const handler = vi.fn(async () => ({ ok: true }));
  const execute = forwardClientToolCall(handler);

  await expect(
    execute(
      { question: "same tool name" },
      { toolCallId: "call-actual-7" } as never,
    ),
  ).resolves.toEqual({ ok: true });
  expect(handler).toHaveBeenCalledWith(
    { question: "same tool name" },
    { toolCallId: "call-actual-7" },
  );
});
