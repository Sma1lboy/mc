import { expect, it, vi } from "vitest";
import { rootFromAgentContext } from "./agentContext";

it("uses the conversation-captured root without consulting the selected global root", () => {
  const selectedRoot = vi.fn(() => "/instance-B");

  expect(rootFromAgentContext({ root: "/instance-A" }, selectedRoot)).toBe("/instance-A");
  expect(selectedRoot).not.toHaveBeenCalled();

  expect(
    rootFromAgentContext(
      {
        instance: {
          root: "/instance-C",
          modpackId: "pack-C",
          instanceId: "C",
          sourcePaths: ["/instance-C/.minecraft"],
          mcVersion: "1.21.1",
          loader: "fabric",
        },
      },
      selectedRoot,
    ),
  ).toBe("/instance-C");
  expect(selectedRoot).not.toHaveBeenCalled();
});
