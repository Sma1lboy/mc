import { useState } from "react";
import type { UIMessage } from "ai";
import {
  Check,
  Download,
  LoaderCircle,
  MemoryStick,
  Power,
  Trash2,
  Wrench,
  X,
} from "lucide-react";
import { t } from "../i18n";
import { commands } from "../ipc/bindings";
import { resolveClientTool, useChatStore, type AgentInstanceContext } from "./chatStore";
import {
  normalizeInstanceChangeOperations,
  type InstanceChangeOperation as Operation,
} from "./instanceChangePlan";

export const SHOW_INSTANCE_CHANGES_TOOL_TYPE = "tool-show_instance_changes";

type ToolPart = Extract<UIMessage["parts"][number], { toolCallId: string }>;

interface ShowChangesInput {
  summary?: string;
  operations?: unknown;
}

interface OperationResult {
  type: Operation["type"];
  status: "completed" | "failed";
  error?: string;
}

interface ShowChangesOutput {
  applied?: boolean;
  results?: OperationResult[];
}

export function InstanceChangesCard(props: {
  msgId: string;
  part: ToolPart;
  globalStreaming: boolean;
}): React.ReactElement {
  const { part, globalStreaming } = props;
  const [busy, setBusy] = useState(false);
  const [runningIndex, setRunningIndex] = useState<number | null>(null);
  const [progress, setProgress] = useState<OperationResult[]>([]);
  const conversationId = useChatStore((state) => state.conversationId);
  const context = useChatStore((state) => state.toolContext?.instance ?? null);
  const pendingLocalToolCallIds = useChatStore((state) => state.pendingLocalToolCallIds);
  const input = (part.input ?? {}) as ShowChangesInput;
  const summary = typeof input.summary === "string" ? input.summary.trim() : "";
  const normalizedPlan = normalizeInstanceChangeOperations(input.operations);
  const operations = normalizedPlan.operations;
  const done = part.state === "output-available";
  const output = (done ? part.output : null) as ShowChangesOutput | null;
  const live =
    part.state === "input-available" &&
    (!globalStreaming || pendingLocalToolCallIds.includes(part.toolCallId)) &&
    !busy &&
    Boolean(context?.root) &&
    normalizedPlan.error === undefined &&
    operations.length > 0;
  const resultByIndex = output?.results ?? progress;

  const apply = async (): Promise<void> => {
    if (!live || !context?.root) return;
    const boundContext = { ...context, root: context.root };
    setBusy(true);
    const results: OperationResult[] = [];
    for (const [index, operation] of operations.entries()) {
      setRunningIndex(index);
      try {
        await runOperation(operation, boundContext);
        results.push({ type: operation.type, status: "completed" });
      } catch (error) {
        results.push({ type: operation.type, status: "failed", error: String(error) });
        setProgress([...results]);
        break;
      }
      setProgress([...results]);
    }
    setRunningIndex(null);
    if (results.some((result) => result.status === "completed")) {
      void commands.rebuildInstanceWikiIndex(boundContext.root, boundContext.instanceId);
    }
    resolveClientTool(conversationId, props.msgId, part.toolCallId, {
      applied: results.every((result) => result.status === "completed"),
      results,
    });
  };

  const cancel = (): void => {
    if (!live) return;
    resolveClientTool(conversationId, props.msgId, part.toolCallId, {
      applied: false,
      results: [],
    });
  };

  if (part.state === "input-streaming" && !summary && operations.length === 0) {
    return (
      <div
        className="my-[6px] h-[92px] rounded-none border border-titlebar bg-panel-2 shadow-input animate-pulse"
        aria-hidden="true"
      />
    );
  }

  return (
    <div className="my-[6px] rounded-none border border-titlebar bg-panel-2 shadow-input">
      <div className="flex items-start gap-[9px] px-[13px] py-[10px] border-b border-titlebar">
        <span className="mt-[1px] grid h-[24px] w-[24px] shrink-0 place-items-center bg-panel-3 text-accent">
          <Wrench size={14} aria-hidden="true" />
        </span>
        <div className="min-w-0">
          <div className="text-[13px] font-medium text-strong">
            {t("agent.changesTitle", { n: String(operations.length) })}
          </div>
          {summary && <div className="mt-[2px] text-[12px] leading-[1.5] text-sub">{summary}</div>}
        </div>
      </div>

      <div className="divide-y divide-titlebar">
        {operations.map((operation, index) => {
          const result = resultByIndex[index];
          const Icon = operationIcon(operation);
          return (
            <div key={operationKey(operation, index)} className="flex items-start gap-[9px] px-[13px] py-[8px]">
              <Icon
                size={15}
                className={operation.type === "delete_mod" ? "mt-[1px] shrink-0 text-danger-text" : "mt-[1px] shrink-0 text-muted"}
                aria-hidden="true"
              />
              <div className="min-w-0 flex-1">
                <div className="text-[12.5px] leading-[1.45] text-fg break-words">
                  {operationLabel(operation)}
                </div>
                {result?.error && (
                  <div className="mt-[2px] text-[11.5px] leading-[1.45] text-danger-text break-words">
                    {result.error}
                  </div>
                )}
              </div>
              {runningIndex === index && !done ? (
                <LoaderCircle size={14} className="shrink-0 animate-spin text-accent" aria-hidden="true" />
              ) : result?.status === "completed" ? (
                <Check size={14} className="shrink-0 text-accent" aria-label={t("agent.changesCompleted")} />
              ) : result?.status === "failed" ? (
                <X size={14} className="shrink-0 text-danger-text" aria-label={t("agent.changesFailed")} />
              ) : null}
            </div>
          );
        })}
      </div>

      {normalizedPlan.error && (
        <div className="px-[13px] py-[8px] border-t border-titlebar text-[12px] leading-[1.5] text-danger-text">
          {t("agent.changesInvalidPlan")}
        </div>
      )}

      <div className="flex items-center gap-[8px] px-[13px] py-[9px] border-t border-titlebar">
        {done ? (
          <div className="text-[12px] leading-[1.5] text-sub">
            {output?.applied
              ? t("agent.changesApplied")
              : resultByIndex.length > 0
                ? t("agent.changesPartial")
                : t("agent.changesCancelled")}
          </div>
        ) : (
          <>
            <button
              type="button"
              disabled={!live}
              onClick={() => void apply()}
              className="inline-flex h-[30px] items-center gap-[6px] px-[12px] rounded-none bg-accent text-accent-text shadow-raised text-[12.5px] font-medium cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed hover:enabled:bg-accent-hover transition-colors duration-[var(--dur)] ease-app"
            >
              {busy ? <LoaderCircle size={14} className="animate-spin" aria-hidden="true" /> : <Check size={14} aria-hidden="true" />}
              {busy ? t("agent.changesApplying") : t("agent.changesApply")}
            </button>
            <button
              type="button"
              disabled={!live}
              onClick={cancel}
              className="inline-flex h-[30px] items-center gap-[6px] px-[11px] rounded-none bg-panel-3 text-sub shadow-input text-[12.5px] cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed hover:enabled:text-fg transition-colors duration-[var(--dur)] ease-app"
            >
              <X size={14} aria-hidden="true" />
              {t("agent.changesCancel")}
            </button>
          </>
        )}
      </div>
    </div>
  );
}

async function runOperation(
  operation: Operation,
  context: AgentInstanceContext & { root: string },
): Promise<void> {
  switch (operation.type) {
    case "set_memory": {
      const config = await unwrapCommand(commands.getInstanceConfig(context.root, context.instanceId));
      await unwrapCommand(
        commands.setInstanceConfig(context.root, context.instanceId, {
          ...config,
          memory_mb: operation.memory_mb,
        }),
      );
      return;
    }
    case "set_mod_enabled":
      await unwrapCommand(
        commands.setModEnabled(
          context.root,
          context.instanceId,
          operation.file_name,
          operation.enabled,
        ),
      );
      return;
    case "delete_mod":
      await unwrapCommand(commands.deleteMod(context.root, context.instanceId, operation.file_name));
      return;
    case "install_mod":
      await unwrapCommand(
        commands.installMod(
          context.root,
          context.instanceId,
          operation.project_id,
          context.mcVersion,
          context.loader,
          operation.provider,
        ),
      );
  }
}

async function unwrapCommand<T>(
  promise: Promise<{ status: "ok"; data: T } | { status: "error"; error: string }>,
): Promise<T> {
  const result = await promise;
  if (result.status === "error") throw new Error(result.error);
  return result.data;
}

function operationIcon(operation: Operation) {
  switch (operation.type) {
    case "set_memory":
      return MemoryStick;
    case "set_mod_enabled":
      return Power;
    case "delete_mod":
      return Trash2;
    case "install_mod":
      return Download;
  }
}

function operationLabel(operation: Operation): string {
  switch (operation.type) {
    case "set_memory":
      return t("agent.changesSetMemory", { memory: String(operation.memory_mb) });
    case "set_mod_enabled":
      return operation.enabled
        ? t("agent.changesEnableMod", { file: operation.file_name })
        : t("agent.changesDisableMod", { file: operation.file_name });
    case "delete_mod":
      return t("agent.changesDeleteMod", { file: operation.file_name });
    case "install_mod":
      return t("agent.changesInstallMod", { name: operation.title || operation.project_id });
  }
}

function operationKey(operation: Operation, index: number): string {
  switch (operation.type) {
    case "set_memory":
      return `memory:${operation.memory_mb}:${index}`;
    case "set_mod_enabled":
    case "delete_mod":
      return `${operation.type}:${operation.file_name}:${index}`;
    case "install_mod":
      return `install:${operation.provider}:${operation.project_id}:${index}`;
  }
}

export default InstanceChangesCard;
