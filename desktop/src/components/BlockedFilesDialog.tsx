import { Component, For, Show } from "solid-js";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
import { Dialog } from "./Dialog";
import { ACCENT_BTN } from "./styles";
import type { BlockedFile } from "../ipc/types";
import { t } from "../i18n";

/**
 * BlockedFilesDialog —— 整合包导入后,列出**需用户手动下载**的文件(CurseForge 作者禁第三方
 * 分发的 mod)与被跳过的可选文件。后端早已算好 name / 下载页 / 目标目录,但之前 UI 只显示个
 * 数字,导致 CurseForge 整合包装完即缺文件、进不去游戏却不知为何。这里把它们摊开:每个给出
 * 「打开下载页」按钮 + 该放进哪个目录。
 */
export const BlockedFilesDialog: Component<{
  instanceId: string;
  blocked: BlockedFile[];
  skipped: string[];
  onClose: () => void;
}> = (props) => {
  return (
    <Dialog
      open
      onClose={props.onClose}
      label={t("components.blocked.title")}
      contentClass="w-[480px] max-w-[calc(100vw-48px)] glass-pop rounded-card overflow-hidden"
    >
      <div class="flex flex-col gap-[14px] p-[20px]">
        <div>
          <div class="text-[15px] font-bold text-fg">{t("components.blocked.heading", { id: props.instanceId })}</div>
          <div class="mt-[4px] text-[12px] leading-[1.7] text-dim">
            {t("components.blocked.body")}
          </div>
        </div>

        <Show when={props.blocked.length > 0}>
          <div class="flex flex-col gap-[8px] max-h-[300px] overflow-y-auto">
            <For each={props.blocked}>
              {(b) => (
                <div class="flex items-center gap-[10px] rounded-ctl glass-card px-[12px] py-[9px]">
                  <div class="min-w-0 flex-1">
                    <div class="text-[13px] font-semibold text-fg whitespace-nowrap overflow-hidden text-ellipsis">
                      {b.name}
                      <Show when={b.required}>
                        <span class="ml-[6px] text-[11px] text-danger-text">{t("components.blocked.required")}</span>
                      </Show>
                    </div>
                    <div class="mt-[2px] text-[11px] text-dim whitespace-nowrap overflow-hidden text-ellipsis">
                      {t("components.blocked.placeInto", { dir: b.target_dir || "mods/" })}
                    </div>
                  </div>
                  <button
                    class="shrink-0 h-[30px] rounded-ctl border border-glass-border bg-glass-card px-[12px] text-[12px] font-semibold text-a-6 cursor-pointer hover:bg-a-1"
                    onClick={() => void shellOpen(b.website_url)}
                  >
                    {t("components.blocked.openPage")}
                  </button>
                </div>
              )}
            </For>
          </div>
        </Show>

        <Show when={props.skipped.length > 0}>
          <div class="rounded-ctl bg-glass-card px-[12px] py-[10px]">
            <div class="text-[12px] font-semibold text-dim mb-[4px]">{t("components.blocked.skipped", { count: props.skipped.length })}</div>
            <div class="text-[11px] leading-[1.7] text-n-6 break-words">{props.skipped.join("、")}</div>
          </div>
        </Show>

        <div class="flex justify-end">
          <button class={ACCENT_BTN} onClick={props.onClose}>
            {t("components.blocked.gotIt")}
          </button>
        </div>
      </div>
    </Dialog>
  );
};

export default BlockedFilesDialog;
