import { useState } from "react";
import type { UIMessage } from "ai";
import { t } from "../i18n";
import { commands } from "../ipc/bindings";
import { activeRoot } from "../store";
import { rootFromAgentContext } from "./agentContext";
import { resolveClientTool, useChatStore } from "./chatStore";

/* ============================================================================
 * ModpackCard —— 渲染 `show_modpack`(原生 client-side tool)的可安装整合包卡片。
 *
 * 与 AskUserOptions 同一套状态机:input-streaming 渲染骨架;input-available 时 turn
 * 已暂停,展示卡片 + 安装/暂不按钮 —— 安装永远是用户在这里点出来的,模型只负责展示:
 *   base   → 现成整合包,直接走 provider 安装(installModpack,与详情页同一条命令);
 *   mrpack → 本次对话构建的 .mrpack,走沙箱导入(agentToolInstallModpack)。
 * 结果 { installed, instance_id? } 写回该 tool part 续跑,模型据此收尾。
 * ========================================================================== */

/** UIMessage 里 show_modpack 工具 part 的 type。 */
export const SHOW_MODPACK_TOOL_TYPE = "tool-show_modpack";

type ToolPart = Extract<UIMessage["parts"][number], { toolCallId: string }>;

interface ShowInput {
  base?: {
    provider?: string;
    project_id?: string;
    version_id?: string;
    title?: string;
    mc_version?: string;
    loader?: string;
  } | null;
  mrpack?: { path?: string; title?: string; mc_version?: string; loader?: string } | null;
}

interface ShowOutput {
  installed?: boolean;
  instance_id?: string;
}

export function ModpackCard(props: {
  msgId: string;
  part: ToolPart;
  /** 全局是否在流式(input-available 且非流式时才可操作)。 */
  globalStreaming: boolean;
}): React.ReactElement {
  const { part, globalStreaming } = props;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const input = (part.input ?? {}) as ShowInput;
  const base = input.base && typeof input.base.project_id === "string" ? input.base : null;
  const mrpack = input.mrpack && typeof input.mrpack.path === "string" ? input.mrpack : null;
  const title = (mrpack?.title ?? base?.title ?? "").trim();
  const mcVersion = mrpack?.mc_version ?? base?.mc_version;
  const loader = mrpack?.loader ?? base?.loader;
  const source = mrpack
    ? t("agent.packBuilt")
    : t("agent.packFromProvider", { provider: base?.provider ?? "modrinth" });

  const done = part.state === "output-available";
  const output = (done ? part.output : null) as ShowOutput | null;
  // 本地引擎(claude-code)下 turn 暂停等答时 streaming 仍为 true —— 以待答标记放行。
  const pendingLocalToolCallIds = useChatStore((s) => s.pendingLocalToolCallIds);
  const conversationId = useChatStore((s) => s.conversationId);
  const toolContext = useChatStore((s) => s.toolContext);
  const live =
    part.state === "input-available" &&
    (!globalStreaming || pendingLocalToolCallIds.includes(part.toolCallId)) &&
    !busy;
  const skeleton = !done && !title;

  const install = async (): Promise<void> => {
    if (!live) return;
    setBusy(true);
    setError(null);
    const root = rootFromAgentContext(toolContext, activeRoot);
    const res = mrpack
      ? await commands.agentToolInstallModpack(root, { path: mrpack.path! })
      : await commands.installModpack(
          root,
          base?.provider ?? "modrinth",
          base?.project_id ?? "",
          base?.version_id ?? "",
          null,
          null,
        );
    if (res.status === "ok") {
      resolveClientTool(conversationId, props.msgId, part.toolCallId, {
        installed: true,
        instance_id: res.data.instance_id,
      });
    } else {
      setBusy(false);
      setError(res.error);
    }
  };

  const skip = (): void => {
    if (!live) return;
    resolveClientTool(conversationId, props.msgId, part.toolCallId, { installed: false });
  };

  if (skeleton) {
    return (
      <div className="my-[6px] h-[74px] rounded-none border border-titlebar bg-panel-2 shadow-input animate-pulse" aria-hidden="true" />
    );
  }

  return (
    <div className="my-[6px] flex flex-col gap-[8px] rounded-none border border-titlebar bg-panel-2 shadow-input px-[13px] py-[11px]">
      <div className="min-w-0">
        <div className="text-[14px] font-medium text-strong break-words">{title}</div>
        <div className="mt-[3px] flex flex-wrap items-center gap-[6px] text-[12px] text-muted">
          <span>{source}</span>
          {mcVersion && <span className="px-[6px] py-[1px] bg-panel-3 text-sub">{mcVersion}</span>}
          {loader && <span className="px-[6px] py-[1px] bg-panel-3 text-sub">{loader}</span>}
        </div>
      </div>

      {done ? (
        <div className="text-[12.5px] leading-[1.5] text-sub">
          {output?.installed
            ? t("agent.packInstalled", { id: output.instance_id ?? "" })
            : t("agent.packSkipped")}
        </div>
      ) : (
        <div className="flex items-center gap-[8px]">
          <button
            type="button"
            disabled={!live}
            onClick={() => void install()}
            className="px-[14px] py-[7px] rounded-none bg-accent text-accent-text shadow-raised text-[13px] font-medium cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed hover:enabled:bg-accent-hover transition-colors duration-[var(--dur)] ease-app"
          >
            {busy ? t("agent.packInstalling") : t("agent.packInstall")}
          </button>
          <button
            type="button"
            disabled={!live}
            onClick={skip}
            className="px-[12px] py-[7px] rounded-none bg-panel-3 text-sub shadow-input text-[13px] cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed hover:enabled:text-fg transition-colors duration-[var(--dur)] ease-app"
          >
            {t("agent.packSkip")}
          </button>
        </div>
      )}

      {error && (
        <div className="px-[10px] py-[6px] rounded-none bg-danger-soft text-danger-text text-[12px] leading-[1.5] break-words">
          {t("agent.packInstallFailed", { err: error })}
        </div>
      )}
    </div>
  );
}

export default ModpackCard;
