// Shared Tauri transport for the sessionized local Claude runtime host.
// Each provider wrapper supplies a mode, while every run carries immutable
// conversation/run/context identity through `localRuntimeProtocol`.
import { listen } from "@tauri-apps/api/event";
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
  protocolPromise = (async () => {
    await unwrap(commands.agentHostStart());
    const protocol = createLocalRuntimeProtocol({
      send: (message) =>
        unwrap(commands.agentHostSend(JSON.stringify(message))).then(() => undefined),
      isInteractiveTool: (name) => INTERACTIVE_CLIENT_TOOLS.has(name),
      runAutomaticTool: (name, input, context) =>
        runLauncherClientTool(name, input, context as AgentToolContext | null),
      waitForInteractiveTool: hooks.waitForInteractiveTool,
    });
    await listen<AgentHostEvent>("agent-host://event", (event) => {
      try {
        protocol.handle(JSON.parse(event.payload.line) as LocalRuntimeInboundMessage);
      } catch {
        // Host stderr carries diagnostics; malformed/non-JSON stdout is ignored.
      }
    });
    return protocol;
  })().catch((error) => {
    protocolPromise = null;
    throw error;
  });
  return protocolPromise;
}

export async function createLocalRuntimeAgent(
  mode: AgentMode = "modpack",
  hooks: LocalRuntimeHooks,
): Promise<AgentProviderSession> {
  const protocol = await sharedProtocol(hooks);
  return {
    run: (request) => protocol.run(request, mode),
  };
}
