import { beforeEach, describe, expect, it, vi } from "vitest";

const ipc = vi.hoisted(() => ({
  buildModpack: vi.fn(async () => ({ status: "ok" as const, data: {} })),
  startDeepDiagnosis: vi.fn(async () => ({ status: "ok" as const, data: {} })),
  instanceDir: vi.fn(async () => ({ status: "ok" as const, data: "/game/versions/pack" })),
  wikiSearch: vi.fn(async () => ({ status: "ok" as const, data: { hits: [] } })),
}));

vi.mock("../ipc/bindings", () => ({
  commands: {
    agentToolBuildModpack: ipc.buildModpack,
    agentToolStartDeepDiagnosis: ipc.startDeepDiagnosis,
    instanceDir: ipc.instanceDir,
    agentToolWikiSearch: ipc.wikiSearch,
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

  it("resolves and caches missing source paths for a bound instance", async () => {
    const boundContext = {
      mode: "instance" as const,
      root: "/game",
      instance: {
        modpackId: "pack",
        instanceId: "pack",
        mcVersion: "1.20.1",
        loader: "fabric",
      },
    };

    await runLauncherClientTool("wiki_search", { query: "difficulty" }, boundContext);
    await runLauncherClientTool("wiki_search", { query: "recipes" }, boundContext);

    expect(ipc.instanceDir).toHaveBeenCalledOnce();
    expect(ipc.instanceDir).toHaveBeenCalledWith("/game", "pack");
    expect(ipc.wikiSearch).toHaveBeenCalledWith(
      "/game",
      expect.objectContaining({
        modpack_id: "pack",
        instance_id: "pack",
        source_paths: ["/game/versions/pack"],
        query: "difficulty",
      }),
    );
  });

  it("fails closed when a restored instance has no runtime root binding", async () => {
    const restoredContext = {
      mode: "instance" as const,
      instance: {
        modpackId: "pack",
        instanceId: "pack",
        mcVersion: "1.20.1",
        loader: "fabric",
      },
    };

    await expect(
      runLauncherClientTool("wiki_search", { query: "difficulty" }, restoredContext),
    ).rejects.toThrow();
    expect(ipc.instanceDir).not.toHaveBeenCalled();
    expect(ipc.wikiSearch).not.toHaveBeenCalled();
  });
});
