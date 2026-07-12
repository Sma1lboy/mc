import { useState } from "react";
import type { UIMessage } from "ai";
import { Check, Hammer, LoaderCircle, X } from "lucide-react";
import { t } from "../i18n";
import type { BuildModpackArgs, BuildModpackOutput } from "../ipc/bindings";
import { decideApprovedAction, executeApprovedModpackBuild } from "./approvalActions";
import { captureClientToolRunId, resolveClientTool, useChatStore } from "./chatStore";

export const CONFIRM_MODPACK_BUILD_TOOL_TYPE = "tool-confirm_modpack_build";

type ToolPart = Extract<UIMessage["parts"][number], { toolCallId: string }>;
type ApprovalOutput = BuildModpackOutput | { approved: false };

function approvalOutput(value: unknown): ApprovalOutput | null {
  return value !== null && typeof value === "object" ? (value as ApprovalOutput) : null;
}

export function BuildConfirmationCard(props: {
  msgId: string;
  part: ToolPart;
  globalStreaming: boolean;
}): React.ReactElement {
  const { part, globalStreaming } = props;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const pendingLocalToolCallIds = useChatStore((state) => state.pendingLocalToolCallIds);
  const conversationId = useChatStore((state) => state.conversationId);
  const input = (part.input ?? {}) as Partial<BuildModpackArgs>;
  const valid =
    typeof input.target?.mc_version === "string" &&
    typeof input.target.loader === "string" &&
    Array.isArray(input.extra_mods) &&
    typeof input.output_filename === "string" &&
    input.output_filename.trim().length > 0;
  const done = part.state === "output-available";
  const output = approvalOutput(done ? part.output : null);
  const declined = output !== null && "approved" in output && output.approved === false;
  const live =
    part.state === "input-available" &&
    (!globalStreaming || pendingLocalToolCallIds.includes(part.toolCallId)) &&
    valid &&
    !busy;

  const decide = async (approved: boolean): Promise<void> => {
    if (!live) return;
    const expectedRunId = captureClientToolRunId(
      conversationId,
      props.msgId,
      part.toolCallId,
    );
    if (approved) setBusy(true);
    setError(null);
    try {
      const result = await decideApprovedAction(approved, () =>
        executeApprovedModpackBuild(input as BuildModpackArgs),
      );
      resolveClientTool(
        conversationId,
        props.msgId,
        part.toolCallId,
        result,
        undefined,
        expectedRunId,
      );
    } catch (cause) {
      setBusy(false);
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  if (part.state === "input-streaming" && !valid) {
    return <div className="my-[6px] h-[110px] border border-titlebar bg-panel-2 shadow-input animate-pulse" aria-hidden="true" />;
  }

  return (
    <div className="my-[6px] border border-titlebar bg-panel-2 shadow-input">
      <div className="flex items-start gap-[9px] px-[13px] py-[10px] border-b border-titlebar">
        <span className="mt-[1px] grid h-[24px] w-[24px] shrink-0 place-items-center bg-panel-3 text-accent">
          <Hammer size={14} aria-hidden="true" />
        </span>
        <div className="min-w-0">
          <div className="text-[13px] font-medium text-strong">{t("agent.buildConfirmTitle")}</div>
          <div className="mt-[2px] text-[12px] leading-[1.5] text-sub">
            {t("agent.buildConfirmSummary", {
              version: input.target?.mc_version ?? "",
              loader: input.target?.loader ?? "",
              count: String(input.extra_mods?.length ?? 0),
            })}
          </div>
        </div>
      </div>
      <div className="grid gap-[5px] px-[13px] py-[9px] text-[12px] text-sub">
        <div>{t("agent.buildConfirmBase", { name: input.base_pack?.title ?? input.base_pack?.project_id ?? t("agent.buildConfirmScratch") })}</div>
        <div>{t("agent.buildConfirmFile", { file: input.output_filename ?? "" })}</div>
        {(input.extra_mods?.length ?? 0) > 0 && (
          <div>
            <div className="text-faint">{t("agent.buildConfirmMods")}</div>
            <ul className="mt-[3px] list-disc space-y-[2px] pl-[18px]">
              {input.extra_mods?.map((mod) => (
                <li key={`${mod.provider ?? "modrinth"}:${mod.project_id}:${mod.version_id}`}>
                  {mod.title ?? mod.project_id} · {mod.version_id}
                </li>
              ))}
            </ul>
          </div>
        )}
      </div>
      <div className="flex items-center gap-[8px] px-[13px] py-[9px] border-t border-titlebar">
        {done ? (
          <div className="text-[12px] text-sub">
            {declined
              ? t("agent.buildDeclined")
              : output && "status" in output && output.status === "completed"
                ? t("agent.buildCompleted")
                : t("agent.buildNotCompleted")}
          </div>
        ) : (
          <>
            <button type="button" disabled={!live} onClick={() => void decide(true)} className="inline-flex h-[30px] items-center gap-[6px] px-[12px] bg-accent text-accent-text shadow-raised text-[12.5px] font-medium cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed">
              {busy ? <LoaderCircle size={14} className="animate-spin" aria-hidden="true" /> : <Check size={14} aria-hidden="true" />}
              {busy ? t("agent.buildBuilding") : t("agent.buildApprove")}
            </button>
            <button type="button" disabled={!live} onClick={() => void decide(false)} className="inline-flex h-[30px] items-center gap-[6px] px-[11px] bg-panel-3 text-sub shadow-input text-[12.5px] cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed">
              <X size={14} aria-hidden="true" />
              {t("agent.buildCancel")}
            </button>
          </>
        )}
      </div>
      {!valid && <div className="px-[13px] py-[8px] border-t border-titlebar text-[12px] text-danger-text">{t("agent.buildInvalidPlan")}</div>}
      {error && <div className="px-[13px] py-[8px] border-t border-titlebar text-[12px] text-danger-text">{t("agent.buildFailed", { err: error })}</div>}
    </div>
  );
}

export default BuildConfirmationCard;
