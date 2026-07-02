import { open as shellOpen } from "@tauri-apps/plugin-shell";
import { Dialog } from "./Dialog";
import { Button } from "./Button";
import { Panel } from "./Panel";
import { Tag } from "./Tag";
import { Heading } from "./Typography";
import type { BlockedFile } from "../ipc/types";
import { t, useLang } from "../i18n";

/**
 * BlockedFilesDialog —— 整合包导入后,列出**需用户手动下载**的文件(CurseForge 作者禁第三方
 * 分发的 mod)与被跳过的可选文件。后端早已算好 name / 下载页 / 目标目录,但之前 UI 只显示个
 * 数字,导致 CurseForge 整合包装完即缺文件、进不去游戏却不知为何。这里把它们摊开:每个给出
 * 「打开下载页」按钮 + 该放进哪个目录。
 */
export function BlockedFilesDialog(props: {
  instanceId: string;
  blocked: BlockedFile[];
  skipped: string[];
  onClose: () => void;
}) {
  useLang();
  return (
    <Dialog
      open
      onClose={props.onClose}
      label={t("components.blocked.title")}
      contentClass="w-[480px] max-w-[calc(100vw-48px)]"
    >
      <div className="flex flex-col gap-[14px] p-[20px]">
        <div>
          <Heading size="sub">{t("components.blocked.heading", { id: props.instanceId })}</Heading>
          <div className="mt-[4px] text-[12px] leading-[1.7] text-sub">
            {t("components.blocked.body")}
          </div>
        </div>

        {props.blocked.length > 0 && (
          <div className="flex flex-col gap-[8px] max-h-[300px] overflow-y-auto">
            {props.blocked.map((b) => (
              <Panel key={b.name} variant="sunken" className="flex items-center gap-[10px] px-[12px] py-[9px]">
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-[6px] text-[13px] font-semibold text-fg whitespace-nowrap overflow-hidden text-ellipsis">
                    <span className="truncate">{b.name}</span>
                    {b.required && <Tag className="!text-danger-text">{t("components.blocked.required")}</Tag>}
                  </div>
                  <div className="mt-[2px] text-[11px] text-muted whitespace-nowrap overflow-hidden text-ellipsis">
                    {t("components.blocked.placeInto", { dir: b.target_dir || "mods/" })}
                  </div>
                </div>
                <Button variant="ghost" className="shrink-0 !h-[30px] !px-[12px] !text-[12px]" onClick={() => void shellOpen(b.website_url)}>
                  {t("components.blocked.openPage")}
                </Button>
              </Panel>
            ))}
          </div>
        )}

        {props.skipped.length > 0 && (
          <Panel variant="input" className="px-[12px] py-[10px]">
            <div className="text-[12px] font-semibold text-sub mb-[4px]">{t("components.blocked.skipped", { count: props.skipped.length })}</div>
            <div className="text-[11px] leading-[1.7] text-muted break-words">{props.skipped.join("、")}</div>
          </Panel>
        )}

        <div className="flex justify-end">
          <Button variant="primary" onClick={props.onClose}>
            {t("components.blocked.gotIt")}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

export default BlockedFilesDialog;
