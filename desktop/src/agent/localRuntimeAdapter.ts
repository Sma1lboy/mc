// Shared Tauri transport for the sessionized local Claude runtime host.
// Each provider wrapper supplies a mode, while every run carries immutable
// conversation/run/context identity through `localRuntimeProtocol`.
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AgentMode } from "@kobemc/agent-core";
import { commands, type AgentHostEvent } from "../ipc/bindings";
import type { AgentToolContext } from "./agentContext";
import {
  INTERACTIVE_CLIENT_TOOLS,
  runLauncherClientTool,
  unwrap,
} from "./clientToolDispatcher";
import {
  createLocalRuntimeProtocol,
  type LocalRuntimeInboundMessage,
} from "./localRuntimeProtocol";
import type {
  AgentProviderSession,
  AgentRunBinding,
} from "./runCoordinator";

interface LocalRuntimeHooks {
  waitForInteractiveTool: (
    binding: AgentRunBinding,
    name: string,
    toolCallId: string,
  ) => Promise<unknown>;
}

type Protocol = ReturnType<typeof createLocalRuntimeProtocol>;
let protocolPromise: Promise<Protocol> | null = null;

function sharedProtocol(hooks: LocalRuntimeHooks): Promise<Protocol> {
  if (protocolPromise) return protocolPromise;
  let candidate!: Promise<Protocol>;
  candidate = (async () => {
    await unwrap(commands.agentHostStart());
    let disposed = false;
    let unlisten: UnlistenFn | null = null;
    const dispose = () => {
      if (disposed) return;
      disposed = true;
      if (protocolPromise === candidate) protocolPromise = null;
      unlisten?.();
      unlisten = null;
    };
    const protocol = createLocalRuntimeProtocol({
      send: (message) =>
        unwrap(commands.agentHostSend(JSON.stringify(message))).then(() => undefined),
      isInteractiveTool: (name) => INTERACTIVE_CLIENT_TOOLS.has(name),
      runAutomaticTool: (name, input, context) =>
        runLauncherClientTool(name, input, context as AgentToolContext | null),
      waitForInteractiveTool: hooks.waitForInteractiveTool,
    });
    const stopListening = await listen<AgentHostEvent>("agent-host://event", (event) => {
      if (disposed) return;
      try {
        const message = JSON.parse(event.payload.line) as LocalRuntimeInboundMessage;
        if (message.type === "host_exit") dispose();
        protocol.handle(message);
      } catch {
        // Host stderr carries diagnostics; malformed/non-JSON stdout is ignored.
      }
    });
    unlisten = stopListening;
    if (disposed) {
      unlisten();
      unlisten = null;
      throw new Error("local agent host exited during startup");
    }
    return protocol;
  })().catch((error) => {
    if (protocolPromise === candidate) protocolPromise = null;
    throw error;
  });
  protocolPromise = candidate;
  return candidate;
}

export async function createLocalRuntimeAgent(
  mode: AgentMode = "build",
  hooks: LocalRuntimeHooks,
  providerSessionId: string,
): Promise<AgentProviderSession> {
  await sharedProtocol(hooks);
  return {
    run: async (request) => {
      const protocol = await sharedProtocol(hooks);
      return protocol.run(request, mode, providerSessionId);
    },
  };
}
