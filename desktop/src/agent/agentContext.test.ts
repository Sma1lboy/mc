import { expect, it, vi } from "vitest";
import { rootFromAgentContext } from "./agentContext";

it("uses the conversation-captured root without consulting the selected global root", () => {
  const selectedRoot = vi.fn(() => "/instance-B");

  expect(rootFromAgentContext({ root: "/instance-A" }, selectedRoot)).toBe("/instance-A");
  expect(selectedRoot).not.toHaveBeenCalled();
});
