import { expect, it, vi } from "vitest";

const diagnose = vi.hoisted(() =>
  vi.fn(async () => ({ status: "ok" as const, data: { report: {} } })),
);

vi.mock("../ipc/bindings", () => ({
  commands: { agentToolDiagnoseInstance: diagnose },
}));

import { closePostInstallCompatibility } from "./postInstallCompatibility";

it("runs bounded remediation immediately after install without a confirmation tool", async () => {
  await closePostInstallCompatibility("/game", "installed-pack");

  expect(diagnose).toHaveBeenCalledWith("/game", "installed-pack", {
    include_log_tail: false,
    mode: "remediate",
  });
});
