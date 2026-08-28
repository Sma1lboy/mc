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
const hooksByBinding = new WeakMap<AgentRunBinding, LocalRuntimeHooks>();
let protocolPromise: Promise<Protocol> | null = null;

function sharedProtocol(): Promise<Protocol> {
  if (protocolPromise) return protocolPromise;
  let candidate!: Promise<Protocol>;
  candidate = (async () => {
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
      send: async (message) => {
        try {
          await unwrap(commands.agentHostSend(JSON.stringify(message)));
        } catch (error) {
          dispose();
          protocol.handle({ type: "host_exit" });
          throw error;
        }
      },
      isInteractiveTool: (name) => INTERACTIVE_CLIENT_TOOLS.has(name),
      runAutomaticTool: (name, input, context) =>
        runLauncherClientTool(name, input, context as AgentToolContext | null),
      waitForInteractiveTool: (binding, name, toolCallId) => {
        const hooks = hooksByBinding.get(binding);
        if (!hooks) return Promise.reject(new Error("local runtime hooks unavailable"));
        return hooks.waitForInteractiveTool(binding, name, toolCallId);
      },
    });
    try {
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
      await unwrap(commands.agentHostStart());
      if (disposed) throw new Error("local agent host exited during startup");
      return protocol;
    } catch (error) {
      dispose();
      throw error;
    }
  })().catch((error) => {
    if (protocolPromise === candidate) protocolPromise = null;
    throw error;
  });
  protocolPromise = candidate;
  return candidate;
}

function sharedProtocolForRun(signal: AbortSignal): Promise<Protocol | null> {
  if (signal.aborted) return Promise.resolve(null);
  const candidate = sharedProtocol();
  return new Promise((resolve, reject) => {
    let settled = false;
    const cleanup = () => signal.removeEventListener("abort", onAbort);
    const finish = (protocol: Protocol | null) => {
      if (settled) return;
      settled = true;
      cleanup();
      resolve(protocol);
    };
    const fail = (error: unknown) => {
      if (settled) return;
      settled = true;
      cleanup();
      reject(error);
    };
    const onAbort = () => finish(null);
    signal.addEventListener("abort", onAbort, { once: true });
    if (signal.aborted) onAbort();
    candidate.then(finish, fail);
  });
}

export async function createLocalRuntimeAgent(
  mode: AgentMode = "build",
  hooks: LocalRuntimeHooks,
  providerSessionId: string,
): Promise<AgentProviderSession> {
  await sharedProtocol();
  return {
    run: async (request) => {
      const protocol = await sharedProtocolForRun(request.signal);
      if (!protocol || request.signal.aborted) return { messages: request.history };
      hooksByBinding.set(request.binding, hooks);
      return protocol.run(request, mode, providerSessionId);
    },
  };
}
