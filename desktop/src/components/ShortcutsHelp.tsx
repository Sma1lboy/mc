import { Component, For, Show } from "solid-js";
import { Dialog } from "./Dialog";
import { Button } from "./Button";
import { Heading } from "./Typography";
import { shortcutsHelpOpen, setShortcutsHelpOpen } from "../store";
import { isMac } from "../util/shortcuts";
import { t } from "../i18n";

/** 一段按键序列(已是平台对应的字面符号),渲染成 <kbd> 芯片。 */
const Keys: Component<{ keys: string[] }> = (props) => (
  <span class="flex items-center gap-[4px]">
    <For each={props.keys}>
      {(k) => (
        <kbd class="inline-flex min-w-[22px] items-center justify-center rounded-none border border-titlebar bg-panel-2 px-[6px] py-[2px] text-[11px] font-semibold leading-none text-sub">
          {k}
        </kbd>
      )}
    </For>
  </span>
);

/** 帮助浮层里的一组快捷键(标题 + 若干行)。 */
function group(title: string, rows: { keys: string[]; label: string }[]) {
  return { title, rows };
}

/**
 * ShortcutsHelp —— 全局键盘快捷键速查浮层,由 `?` 切换、Esc / 遮罩关闭。
 * 单实例挂在 AppShell 根部;数据与 util/shortcuts.ts 的分发表对应。
 */
export const ShortcutsHelp: Component = () => {
  const mod = () => (isMac() ? "⌘" : "Ctrl");
  const shift = () => (isMac() ? "⇧" : "Shift");

  const groups = () => [
    group(t("shortcuts.groupNav"), [
      { keys: [mod(), shift(), "H"], label: t("shortcuts.navHome") },
      { keys: [mod(), shift(), "L"], label: t("shortcuts.navLibrary") },
      { keys: [mod(), shift(), "D"], label: t("shortcuts.navDiscover") },
      { keys: [mod(), ","], label: t("shortcuts.navSettings") },
    ]),
    group(t("shortcuts.groupLaunch"), [
      { keys: [mod(), "1 – 9"], label: t("shortcuts.launchRecent") },
    ]),
    group(t("shortcuts.groupGeneral"), [
      { keys: ["?"], label: t("shortcuts.toggleHelp") },
      { keys: ["Esc"], label: t("shortcuts.closeHelp") },
    ]),
  ];

  return (
    <Show when={shortcutsHelpOpen()}>
      <Dialog
        open
        onClose={() => setShortcutsHelpOpen(false)}
        label={t("shortcuts.title")}
        contentClass="w-[440px] max-w-[calc(100vw-48px)]"
      >
        <div class="flex flex-col gap-[16px] p-[20px]">
          <div>
            <Heading size="sub">{t("shortcuts.title")}</Heading>
            <div class="mt-[4px] text-[12px] leading-[1.7] text-sub">{t("shortcuts.subtitle")}</div>
          </div>

          <For each={groups()}>
            {(g) => (
              <div class="flex flex-col gap-[6px]">
                <div class="text-[11px] font-semibold uppercase tracking-wide text-muted">
                  {g.title}
                </div>
                <div class="flex flex-col">
                  <For each={g.rows}>
                    {(row) => (
                      <div class="flex items-center justify-between gap-[12px] py-[6px] border-b border-titlebar/40 last:border-b-0">
                        <span class="min-w-0 flex-1 text-[13px] text-fg">{row.label}</span>
                        <Keys keys={row.keys} />
                      </div>
                    )}
                  </For>
                </div>
              </div>
            )}
          </For>

          <div class="flex justify-end">
            <Button variant="primary" onClick={() => setShortcutsHelpOpen(false)}>
              {t("shortcuts.close")}
            </Button>
          </div>
        </div>
      </Dialog>
    </Show>
  );
};

export default ShortcutsHelp;
