import type { Dispatch, SetStateAction } from "react";
import { InstanceIcon } from "../InstanceIcon";
import { Spinner } from "../Spinner";
import { Toggle } from "../Toggle";
import { openInstanceDir } from "../../util/instanceActions";
import { activeRoot } from "../../store";
import { t } from "../../i18n";
import type { InstanceConfig, InstanceSummary } from "../../ipc/types";
import { FIELD, LABEL } from "./shared";

// MiB → 友好的 GB 文本(整数不带小数,否则保留一位)。
const memGb = (mb: number): string => {
  const v = mb / 1024;
  return Number.isInteger(v) ? `${v}` : v.toFixed(1);
};

/** 设置标签的表单(纯展示;所有状态与持久化仍由父组件持有,行为不变)。 */
export function SettingsTab(props: {
  instance: InstanceSummary | null;
  cfg: InstanceConfig | null;
  setCfg: Dispatch<SetStateAction<InstanceConfig | null>>;
  patch: (p: Partial<InstanceConfig>) => void;
  pickIcon: () => void;
  sysTotalMb: number | null;
  suggestedMb: number | null;
  applyRecommendedMemory: () => void;
}) {
  const { cfg, setCfg, patch, pickIcon, sysTotalMb, suggestedMb, applyRecommendedMemory } = props;
  return (
    <>
            {cfg ? (
              <>
                <div className="flex items-center gap-[12px]">
                  <div className="w-[56px] h-[56px] rounded-none overflow-hidden bg-panel-2 shrink-0 select-none">
                    <InstanceIcon name={props.instance?.name || props.instance?.id} icon={props.instance?.icon ?? undefined} />
                  </div>
                  <div className="flex flex-col gap-[5px]">
                    <span className={LABEL}>{t("instance.instanceIcon")}</span>
                    <button
                      className="h-[30px] px-[12px] shadow-raised rounded-none bg-panel-3 text-fg text-[12px] cursor-pointer transition-[box-shadow,filter] duration-[var(--dur)] ease-app hover:brightness-110 active:shadow-pressed w-fit"
                      onClick={pickIcon}
                    >
                      {t("instance.changeIcon")}
                    </button>
                  </div>
                </div>

                <label className="flex flex-col gap-[5px]">
                  <span className={LABEL}>{t("instance.name")}</span>
                  {/* 非受控 + onBlur 持久化:自由输入,失焦才写盘(等价 Solid 的 onChange)。 */}
                  <input
                    key={`name-${props.instance?.id ?? ""}`}
                    className={FIELD}
                    defaultValue={cfg.name ?? ""}
                    onBlur={(e) => patch({ name: e.currentTarget.value || null })}
                  />
                </label>

                <div className="flex flex-col gap-[5px]">
                  <div className="flex items-center justify-between gap-[8px]">
                    <span className={LABEL}>{t("instance.maxMemory", { mb: cfg.memory_mb ?? 0 })}</span>
                    <div className="flex items-center gap-[8px]">
                      {sysTotalMb !== null && (
                        <span className="text-muted text-[11px]">{t("instance.systemMemory", { gb: memGb(sysTotalMb) })}</span>
                      )}
                      {suggestedMb !== null && (
                        <button
                          type="button"
                          className="h-[22px] px-[8px] rounded-none bg-panel-3 text-fg text-[11px] cursor-pointer shadow-raised hover:brightness-110 active:shadow-pressed transition-[box-shadow,filter] duration-[var(--dur)] ease-app"
                          title={t("instance.recommendMemoryHint")}
                          onClick={applyRecommendedMemory}
                        >
                          {t("instance.recommendMemory", { gb: memGb(suggestedMb) })}
                        </button>
                      )}
                    </div>
                  </div>
                  {/* 拖动时 onChange 只更新本地(实时刻度);松手(mouseup/keyup)才写盘,避免逐帧持久化。 */}
                  <input
                    className="kb-range"
                    type="range"
                    min={512}
                    max={16384}
                    step={256}
                    value={cfg.memory_mb}
                    onChange={(e) => {
                      const v = +e.currentTarget.value;
                      setCfg((prev) => (prev ? { ...prev, memory_mb: v } : prev));
                    }}
                    onMouseUp={(e) => patch({ memory_mb: +e.currentTarget.value })}
                    onKeyUp={(e) => patch({ memory_mb: +e.currentTarget.value })}
                  />
                </div>

                <label className="flex flex-col gap-[5px]">
                  <span className={LABEL}>{t("instance.javaPath")}</span>
                  <input
                    key={`java-${props.instance?.id ?? ""}`}
                    className={FIELD}
                    placeholder={t("instance.javaPathPlaceholder")}
                    defaultValue={cfg.java_path ?? ""}
                    onBlur={(e) => patch({ java_path: e.currentTarget.value || null })}
                  />
                </label>

                <label className="flex flex-col gap-[5px]">
                  <span className={LABEL}>{t("instance.extraJvmArgs")}</span>
                  <input
                    key={`jvm-${props.instance?.id ?? ""}`}
                    className={FIELD}
                    defaultValue={(cfg.jvm_args ?? []).join(" ")}
                    onBlur={(e) => patch({ jvm_args: e.currentTarget.value.split(/\s+/).filter(Boolean) })}
                  />
                </label>

                <div className="flex gap-[12px]">
                  <label className="flex-1 flex flex-col gap-[5px]">
                    <span className={LABEL}>{t("instance.windowWidth")}</span>
                    <input
                      key={`w-${props.instance?.id ?? ""}`}
                      className={FIELD}
                      type="number"
                      min={1}
                      max={7680}
                      placeholder={t("instance.defaultPlaceholder")}
                      defaultValue={cfg.width ?? ""}
                      onBlur={(e) => {
                        const n = Math.floor(+e.currentTarget.value);
                        patch({ width: Number.isFinite(n) && n > 0 ? n : null });
                      }}
                    />
                  </label>
                  <label className="flex-1 flex flex-col gap-[5px]">
                    <span className={LABEL}>{t("instance.windowHeight")}</span>
                    <input
                      key={`h-${props.instance?.id ?? ""}`}
                      className={FIELD}
                      type="number"
                      min={1}
                      max={4320}
                      placeholder={t("instance.defaultPlaceholder")}
                      defaultValue={cfg.height ?? ""}
                      onBlur={(e) => {
                        const n = Math.floor(+e.currentTarget.value);
                        patch({ height: Number.isFinite(n) && n > 0 ? n : null });
                      }}
                    />
                  </label>
                </div>

                <div className="flex items-center justify-between text-fg text-[13px]">
                  <span>{t("instance.fullscreenLaunch")}</span>
                  <Toggle
                    checked={cfg.fullscreen ?? false}
                    onChange={(v) => patch({ fullscreen: v })}
                    title={t("instance.fullscreenLaunch")}
                  />
                </div>

                <div className="pt-[4px]">
                  <button
                    className="h-[30px] px-[12px] shadow-raised rounded-none bg-panel-3 text-fg text-[12px] cursor-pointer transition-[box-shadow,filter] duration-[var(--dur)] ease-app hover:brightness-110 active:shadow-pressed"
                    onClick={() => props.instance && openInstanceDir(activeRoot(), props.instance.id)}
                  >
                    {t("instance.openGameDir")}
                  </button>
                </div>
              </>
            ) : (
              <div className="flex items-center gap-[10px] text-muted text-[13px] py-[12px]">
                <Spinner size={16} /> {t("instance.readingConfig")}
              </div>
            )}
    </>
  );
}
