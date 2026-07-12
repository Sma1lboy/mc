import { useEffect, useState } from "react";
import clsx from "clsx";
import { Button, Panel, Segmented, Slider, Heading, toast } from "../../components";
import { Check, RotateCcw } from "lucide-react";
import {
  applyThemeColor,
  setMode,
  saveTheme,
  PRESETS,
  THEME_PRESETS,
  accentFromHex,
  DEFAULT_THEME,
  normalizeThemeConfig,
} from "../../theme/theme";
import { api } from "../../ipc/api";
import { useAppStore, setVeilStrength } from "../../store";
import { t } from "../../i18n";
import type { ThemeConfig, ThemeMode } from "../../ipc/types";

const sectionClass = "p-[20px]";

/** 外观设置块:主题模式 / 完整预设 / HSL 三滑块 / 亚克力浓度。状态全部本地。 */
export function AppearanceSection() {
  const veilStrength = useAppStore((s) => s.veilStrength);
  const [mode, setModeSig] = useState<ThemeMode>("dark");
  const [hue, setHue] = useState(150);
  const [sat, setSat] = useState(60);
  const [light, setLight] = useState(45);

  const themeModeOptions = (): { value: ThemeMode; label: string }[] => [
    { value: "light", label: t("settings.themeModeLight") },
    { value: "dark", label: t("settings.themeModeDark") },
    { value: "system", label: t("settings.themeModeSystem") },
  ];

  // 初始化:从后端取当前主题(规范化后如有变化写回)。
  useEffect(() => {
    void (async () => {
      try {
        const { config: cfg, changed } = normalizeThemeConfig(await api.getTheme());
        setLocalTheme(cfg);
        if (changed) void saveTheme(cfg).catch(() => {});
      } catch {
        setLocalTheme(DEFAULT_THEME);
      }
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function current(): ThemeConfig {
    return { mode, hue, saturation: sat, lightness: light };
  }

  function setLocalTheme(cfg: ThemeConfig) {
    setModeSig(cfg.mode);
    setHue(cfg.hue);
    setSat(cfg.saturation);
    setLight(cfg.lightness);
    applyThemeColor(cfg.hue, cfg.saturation, cfg.lightness);
    setMode(cfg.mode);
  }

  // 实时应用 + 持久化。
  function apply(persist = true) {
    applyThemeColor(hue, sat, light);
    setMode(mode);
    if (persist) void saveTheme(current()).catch(() => {});
  }

  function selectThemeMode(next: ThemeMode) {
    const cfg = { ...current(), mode: next };
    setLocalTheme(cfg);
    void saveTheme(cfg).catch(() => {});
    const label =
      next === "system"
        ? t("settings.themeModeSystem")
        : next === "dark"
          ? t("settings.themeModeDark")
          : t("settings.themeModeLight");
    toast({ type: "info", message: t("settings.switchedTo", { label }) });
  }

  // 一键应用完整主题预设(模式 + 强调色),立即生效并持久化。
  function pickThemePreset(p: { key: string; mode: "dark" | "light"; hex: string }) {
    const cfg: ThemeConfig = { mode: p.mode, ...accentFromHex(p.hex) };
    setLocalTheme(cfg);
    void saveTheme(cfg).catch(() => {});
    toast({
      type: "success",
      message: t("settings.presetApplied", { name: t(`settings.themePreset_${p.key}`) }),
    });
  }

  function isSelectedThemePreset(p: { mode: "dark" | "light"; hex: string }): boolean {
    const a = accentFromHex(p.hex);
    return (
      mode === p.mode &&
      Math.abs(hue - a.hue) < 0.5 &&
      Math.abs(sat - a.saturation) < 0.5 &&
      Math.abs(light - a.lightness) < 0.5
    );
  }

  function pickPreset(p: { hue: number; saturation: number; lightness: number }) {
    setHue(p.hue);
    setSat(p.saturation);
    setLight(p.lightness);
    // setState 是异步的,直接用预设值应用/持久化,避免读到旧闭包值。
    applyThemeColor(p.hue, p.saturation, p.lightness);
    setMode(mode);
    void saveTheme({ mode, hue: p.hue, saturation: p.saturation, lightness: p.lightness }).catch(() => {});
  }

  function isSelectedPreset(p: { hue: number; saturation: number; lightness: number }): boolean {
    return (
      Math.abs(hue - p.hue) < 0.5 &&
      Math.abs(sat - p.saturation) < 0.5 &&
      Math.abs(light - p.lightness) < 0.5
    );
  }

  // 恢复默认主题(深色 + 陶土橙),立即应用并持久化。
  function resetTheme() {
    setLocalTheme(DEFAULT_THEME);
    void saveTheme(DEFAULT_THEME).catch(() => {});
    toast({ type: "success", message: t("settings.resetThemeDone") });
  }

  return (
            <Panel variant="sunken" className={sectionClass}>
              <Heading size="sub" as="h2" className="mb-[14px]">
                {t("settings.sectionAppearance")}
              </Heading>

              <div className="flex items-center justify-between mb-[16px] text-fg text-[14px]">
                <span>{t("settings.themeMode")}</span>
                <Segmented
                  ariaLabel={t("settings.themeMode")}
                  value={mode}
                  options={themeModeOptions()}
                  onChange={selectThemeMode}
                />
              </div>

              <div className="mb-[16px]">
                <span className="block mb-[10px] text-fg text-[14px]">{t("settings.themePresets")}</span>
                <div className="grid grid-cols-3 gap-[8px]">
                  {THEME_PRESETS.map((p) => (
                    <button
                      key={p.key}
                      type="button"
                      className={clsx(
                        "flex items-center gap-[8px] p-[8px] rounded-none shadow-raised cursor-pointer text-left transition-[transform,box-shadow] duration-[var(--dur)] ease-app hover:-translate-y-[1px] active:shadow-pressed focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent",
                        { "ring-2 ring-tag": isSelectedThemePreset(p) },
                      )}
                      title={t(`settings.themePreset_${p.key}`)}
                      aria-label={t("settings.themePresetAria", { name: t(`settings.themePreset_${p.key}`) })}
                      aria-pressed={isSelectedThemePreset(p)}
                      onClick={() => pickThemePreset(p)}
                    >
                      <span
                        className="grid w-[24px] h-[24px] shrink-0 place-items-center rounded-none shadow-sunken"
                        style={{ background: p.hex }}
                      >
                        {isSelectedThemePreset(p) && (
                          <Check size={13} aria-hidden="true" className="text-white drop-shadow-[0_1px_1px_rgba(0,0,0,0.6)]" />
                        )}
                      </span>
                      <span className="flex flex-col gap-[1px] min-w-0">
                        <span className="truncate text-fg text-[12px]">{t(`settings.themePreset_${p.key}`)}</span>
                        <span className="text-muted text-[11px]">
                          {p.mode === "dark" ? t("settings.themeModeDark") : t("settings.themeModeLight")}
                        </span>
                      </span>
                    </button>
                  ))}
                </div>
              </div>

              <div className="flex items-center justify-between mb-[16px] text-fg text-[14px]">
                <span>{t("settings.presetAccent")}</span>
                <div className="flex gap-[8px]">
                  {PRESETS.map((p) => (
                    <button
                      key={p.hex}
                      type="button"
                      className={clsx(
                        "relative grid w-[28px] h-[28px] place-items-center rounded-none shadow-raised cursor-pointer transition-[transform,box-shadow] duration-[var(--dur)] ease-app hover:-translate-y-[1px] active:shadow-pressed focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent",
                        { "ring-2 ring-tag": isSelectedPreset(p) },
                      )}
                      style={{ background: p.hex }}
                      title={p.name}
                      aria-label={t("settings.presetAccentAria", { name: p.name })}
                      aria-pressed={isSelectedPreset(p)}
                      onClick={() => pickPreset(p)}
                    >
                      {isSelectedPreset(p) && (
                        <Check size={14} aria-hidden="true" className="text-white drop-shadow-[0_1px_1px_rgba(0,0,0,0.6)]" />
                      )}
                    </button>
                  ))}
                </div>
              </div>

              <Slider
                className="mb-[14px]"
                label={t("settings.hue")}
                display={(v) => String(Math.round(v))}
                min={0}
                max={360}
                value={hue}
                ariaLabel={t("settings.hue")}
                onInput={(v) => {
                  setHue(v);
                  applyThemeColor(v, sat, light);
                  setMode(mode);
                }}
                onCommit={() => apply()}
              />
              <Slider
                className="mb-[14px]"
                label={t("settings.saturation")}
                display={(v) => String(Math.round(v))}
                min={0}
                max={100}
                value={sat}
                ariaLabel={t("settings.saturation")}
                onInput={(v) => {
                  setSat(v);
                  applyThemeColor(hue, v, light);
                  setMode(mode);
                }}
                onCommit={() => apply()}
              />
              <Slider
                className="mb-[14px]"
                label={t("settings.lightness")}
                display={(v) => String(Math.round(v))}
                min={20}
                max={70}
                value={light}
                ariaLabel={t("settings.lightness")}
                onInput={(v) => {
                  setLight(v);
                  applyThemeColor(hue, sat, v);
                  setMode(mode);
                }}
                onCommit={() => apply()}
              />
              <Slider
                className="mb-[16px]"
                label={t("settings.uiTransparency")}
                display={(v) => `${v}%`}
                min={0}
                max={65}
                step={1}
                value={Math.round((1 - veilStrength) * 100)}
                ariaLabel={t("settings.uiTransparency")}
                onInput={(v) => setVeilStrength(1 - v / 100)}
              />

              <div className="flex gap-[3px] mb-[16px]">
                {[1, 2, 3, 4, 5, 6, 7, 8].map((i) => (
                  <span
                    key={i}
                    className="flex-1 h-[24px] rounded-none shadow-sunken"
                    style={{ background: `var(--a-${i})` }}
                  />
                ))}
              </div>

              <div className="flex items-center justify-between text-fg text-[14px]">
                <span>{t("settings.resetTheme")}</span>
                <Button variant="ghost" onClick={resetTheme}>
                  <RotateCcw size={14} aria-hidden="true" />
                  {t("settings.reset")}
                </Button>
              </div>
            </Panel>
  );
}
