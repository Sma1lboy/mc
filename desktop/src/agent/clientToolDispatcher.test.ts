import { beforeEach, describe, expect, it, vi } from "vitest";

const ipc = vi.hoisted(() => ({
  buildModpack: vi.fn(async () => ({ status: "ok" as const, data: {} })),
  startDeepDiagnosis: vi.fn(async () => ({ status: "ok" as const, data: {} })),
  diagnoseInstance: vi.fn(async () => ({ status: "ok" as const, data: {} })),
}));

vi.mock("../ipc/bindings", () => ({
  commands: {
    agentToolBuildModpack: ipc.buildModpack,
    agentToolStartDeepDiagnosis: ipc.startDeepDiagnosis,
    agentToolDiagnoseInstance: ipc.diagnoseInstance,
  },
}));

vi.mock("../store", () => ({ activeRoot: () => "/game" }));

import { isAutomaticClientTool, runLauncherClientTool } from "./clientToolDispatcher";

const instanceContext = {
  mode: "instance" as const,
  instance: {
    root: "/game",
    modpackId: "pack",
    instanceId: "pack",
    sourcePaths: ["/game/versions/pack"],
    mcVersion: "1.20.1",
    loader: "fabric",
  },
};

describe("privileged client-tool boundary", () => {
  beforeEach(() => vi.clearAllMocks());

  it("does not automatically dispatch a raw modpack build", () => {
    expect(isAutomaticClientTool("build_modpack")).toBe(false);
    expect(() => runLauncherClientTool("build_modpack", {})).toThrow();
    expect(ipc.buildModpack).not.toHaveBeenCalled();
  });

  it("does not automatically dispatch a raw deep-diagnosis launch", () => {
    expect(isAutomaticClientTool("start_deep_diagnosis")).toBe(false);
    expect(() => runLauncherClientTool("start_deep_diagnosis", {}, instanceContext)).toThrow();
    expect(ipc.startDeepDiagnosis).not.toHaveBeenCalled();
  });

  it("dispatches explicit static remediation without an interactive approval tool", async () => {
    expect(isAutomaticClientTool("diagnose_instance")).toBe(true);

    await runLauncherClientTool(
      "diagnose_instance",
      { mode: "remediate", include_log_tail: false },
      instanceContext,
    );

    expect(ipc.diagnoseInstance).toHaveBeenCalledWith("/game", "pack", {
      include_log_tail: false,
      mode: "remediate",
    });
  });
});
