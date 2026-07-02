import { useEffect, useMemo, useRef, useState } from "react";
import clsx from "clsx";
import {
  Button,
  Spinner,
  Select,
  Tooltip,
  ErrorState,
  toast,
  Panel,
  Segmented,
  Slider,
  Heading,
  Tag,
  Toggle,
} from "../components";
import { Check, Info, RotateCcw } from "lucide-react";
import {
  applyThemeColor,
  setMode,
  saveTheme,
  PRESETS,
  THEME_PRESETS,
  accentFromHex,
  DEFAULT_THEME,
  normalizeThemeConfig,
} from "../theme/theme";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api } from "../ipc/api";
import { useAsync } from "../util/useAsync";
import { useAppStore, setVeilStrength, setCurrentRoot, setSocialEnabled } from "../store";
import { t, setLocale, useLang, type Locale } from "../i18n";
import type { ThemeConfig, ThemeMode, GlobalSettings } from "../ipc/types";
import "./Settings.css";

/**
 * Settings —— 主题(HSL 三滑块实时换肤 + 预设 + 深浅切换)
 * 以及 Java 检测列表。改动即时应用,失焦/操作后持久化到后端。
 */
export default function Settings() {
  const locale = useLang();
  const veilStrength = useAppStore((s) => s.veilStrength);
  const currentRoot = useAppStore((s) => s.currentRoot);
  const socialEnabled = useAppStore((s) => s.socialEnabled);

  const [mode, setModeSig] = useState<ThemeMode>("dark");
  const [hue, setHue] = useState(150);
  const [sat, setSat] = useState(60);
  const [light, setLight] = useState(45);

  // 全局设置(下载源/并发/默认内存/Java)—— 全部来自后端 daemon。
  const [settings, setSettings] = useState<GlobalSettings | null>(null);
  const [settingsError, setSettingsError] = useState(false);
  // 本机物理内存总量(MiB),用于在默认内存滑块旁提示「系统内存 X GB」。
  const [sysTotalMb, setSysTotalMb] = useState<number | null>(null);
  const memGb = (mb: number): string => {
    const v = mb / 1024;
    return Number.isInteger(v) ? `${v}` : v.toFixed(1);
  };

  // 加载全局设置(可重试):失败置错误态,区分「加载中」与「加载失败」。
  async function loadSettings() {
    setSettingsError(false);
    try {
      setSettings(await api.getSettings());
    } catch {
      setSettingsError(true);
    }
  }

  const languageOptions = (): { value: Locale; label: string }[] => [
    { value: "zh", label: "中文" },
    { value: "en", label: "English" },
  ];

  const themeModeOptions = (): { value: ThemeMode; label: string }[] => [
    { value: "light", label: t("settings.themeModeLight") },
    { value: "dark", label: t("settings.themeModeDark") },
    { value: "system", label: t("settings.themeModeSystem") },
  ];

  // 初始化:从后端取当前主题 + 全局设置。
  useEffect(() => {
    void (async () => {
      try {
        const { config: cfg, changed } = normalizeThemeConfig(await api.getTheme());
        setLocalTheme(cfg);
        if (changed) void saveTheme(cfg).catch(() => {});
      } catch {
        setLocalTheme(DEFAULT_THEME);
      }
      void loadSettings();
      void api.systemMemory().then((m) => setSysTotalMb(m.total_mb)).catch(() => {});
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 改一项全局设置并立即持久化到后端。
  function patchSettings(patch: Partial<GlobalSettings>) {
    if (!settings) return;
    const next = { ...settings, ...patch };
    setSettings(next);
    void api.setSettings(next).catch(() => {});
  }

  // 游戏目录(根)：列出已发现的根 + 让用户切换/增删自定义根。
  const roots = useAsync(() => api.listRoots(), []);

  // 把自定义根写盘后再让后端重新发现:setSettings 是异步落盘,必须 await 再 refetch,
  // 否则 list_roots 可能读到旧设置(看不到刚加的根)。
  async function persistCustomRoots(customRoots: string[]) {
    if (!settings) return;
    const next = { ...settings, custom_roots: customRoots };
    setSettings(next);
    try {
      await api.setSettings(next);
      roots.refetch();
    } catch (e) {
      toast({ type: "error", message: t("settings.saveDirFailed", { err: String(e) }) });
    }
  }

  async function addCustomRoot() {
    const picked = await openDialog({ directory: true, title: t("settings.pickGameDir") }).catch(() => null);
    if (!picked || typeof picked !== "string") return;
    // 已在已发现列表(便携/官方/已有自定义)里 → 直接切过去,不往设置里塞重复项。
    if ((roots.data ?? []).some((r) => r.path === picked)) {
      setCurrentRoot(picked);
      return;
    }
    const list = settings?.custom_roots ?? [];
    if (!list.includes(picked)) await persistCustomRoots([...list, picked]);
    setCurrentRoot(picked); // 切到新加的根
  }

  async function removeCustomRoot(path: string) {
    const list = (settings?.custom_roots ?? []).filter((p) => p !== path);
    await persistCustomRoots(list);
    if (currentRoot === path) {
      // 删的是当前根:落到剩余的第一个(没有则交回后端默认)。
      setCurrentRoot((roots.data ?? [])[0]?.path ?? null);
    }
  }

  const javas = useAsync(() => api.detectJava(), []);

  // 应用内日志查看(诊断):按需读取最新日志文件的末尾,展开时滚到底看最新。
  const [logOpen, setLogOpen] = useState(false);
  const [logText, setLogText] = useState("");
  const [logLoading, setLogLoading] = useState(false);
  const logBox = useRef<HTMLPreElement>(null);
  async function loadLog() {
    setLogLoading(true);
    try {
      setLogText(await api.readLogTail(800));
    } catch (e) {
      setLogText(t("settings.logReadFailed", { err: String(e) }));
    } finally {
      setLogLoading(false);
      queueMicrotask(() => {
        if (logBox.current) logBox.current.scrollTop = logBox.current.scrollHeight;
      });
    }
  }
  function toggleLog() {
    const next = !logOpen;
    setLogOpen(next);
    if (next) void loadLog();
  }

  // Java 下拉选项:自动检测 + 检测到的各 JVM。
  const javaOptions = useMemo(
    () => [
      { label: t("settings.autoDetect"), value: "" },
      ...(javas.data ?? []).map((j) => ({ label: `Java ${j.version} — ${j.path}`, value: j.path })),
    ],
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [javas.data, locale],
  );

  // 默认内存:MiB → "2G"/"1.5G"/"768M" 的友好显示。
  function formatMem(mib: number): string {
    if (mib >= 1024) {
      const g = mib / 1024;
      return `${Number.isInteger(g) ? g : g.toFixed(1)}G`;
    }
    return `${mib}M`;
  }

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

  // 块标题(像素体) + 内距统一的设置块。
  const sectionClass = "p-[20px]";

  return (
    <div className="settings-page">
      <div className="settings-page__inner">
        <Heading size="page" as="h1" className="mt-0 mb-[20px]">
          {t("settings.title")}
        </Heading>

        <div className="settings-page__grid">
          <div className="settings-page__main">
            <Panel variant="sunken" className={sectionClass}>
              <Heading size="sub" as="h2" className="mb-[14px]">
                {t("settings.sectionLanguage")}
              </Heading>
              <div className="flex items-center justify-between text-fg text-[14px]">
                <span>{t("settings.language")}</span>
                <Segmented
                  ariaLabel={t("settings.langAriaLabel")}
                  value={locale}
                  options={languageOptions()}
                  onChange={setLocale}
                />
              </div>
            </Panel>

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
          </div>

          <div className="settings-page__side">
            <Panel variant="sunken" className={sectionClass}>
              <Heading size="sub" as="h2" className="mb-[14px]">
                {t("settings.sectionSocial")}
              </Heading>
              <div className="flex items-center justify-between text-fg text-[14px]">
                <div className="flex flex-col gap-[2px] min-w-0 pr-[12px]">
                  <span>{t("settings.socialEnabled")}</span>
                  <span className="text-[12px] text-muted">{t("settings.socialEnabledDesc")}</span>
                </div>
                <Toggle checked={socialEnabled} onChange={(v) => void setSocialEnabled(v)} />
              </div>
            </Panel>

            <Panel variant="sunken" className={sectionClass}>
              <Heading size="sub" as="h2" className="mb-[14px]">
                {t("settings.sectionDownloadGame")}
              </Heading>
              {settings ? (
                <>
                  <div className="flex items-center justify-between mb-[16px] text-fg text-[14px]">
                    <span className="flex items-center gap-[6px]">
                      {t("settings.downloadSource")}
                      <Tooltip content={t("settings.downloadSourceTip")}>
                        <Info size={14} aria-hidden="true" />
                      </Tooltip>
                    </span>
                    <Segmented
                      ariaLabel={t("settings.downloadSource")}
                      value={
                        settings.use_mirror || settings.download_source === "bmclapi" ? "bmclapi" : "official"
                      }
                      options={[
                        { value: "official", label: t("settings.sourceOfficial") },
                        { value: "bmclapi", label: t("settings.sourceMirror") },
                      ]}
                      onChange={(value) =>
                        patchSettings({
                          download_source: value,
                          use_mirror: value === "bmclapi",
                        })
                      }
                    />
                  </div>

                  <Slider
                    className="mb-[16px]"
                    label={t("settings.concurrency")}
                    min={1}
                    max={128}
                    value={settings.concurrency ?? 64}
                    ariaLabel={t("settings.concurrency")}
                    onInput={(v) => setSettings({ ...settings, concurrency: v })}
                    onCommit={(v) => patchSettings({ concurrency: v })}
                  />

                  <Slider
                    className="mb-[16px]"
                    label={
                      <span className="flex items-center gap-[6px]">
                        {t("settings.defaultMemory")}
                        {sysTotalMb !== null && (
                          <span className="text-muted text-[11px]">
                            {t("settings.systemMemory", { gb: memGb(sysTotalMb) })}
                          </span>
                        )}
                      </span>
                    }
                    display={formatMem}
                    min={512}
                    max={16384}
                    step={256}
                    value={settings.default_memory_mb ?? 2048}
                    ariaLabel={t("settings.defaultMemory")}
                    onInput={(v) => setSettings({ ...settings, default_memory_mb: v })}
                    onCommit={(v) => patchSettings({ default_memory_mb: v })}
                  />

                  <div className="flex items-center justify-between text-fg text-[14px]">
                    <span>{t("settings.java")}</span>
                    <Select
                      className="max-w-[60%]"
                      value={settings.java_path ?? ""}
                      onChange={(v) => patchSettings({ java_path: v || null })}
                      options={javaOptions}
                      placeholder={t("settings.autoDetect")}
                    />
                  </div>
                </>
              ) : settingsError ? (
                <ErrorState message={t("settings.downloadSettingsFailed")} onRetry={() => void loadSettings()} />
              ) : (
                <div className="text-muted">{t("settings.loadingSettings")}</div>
              )}
            </Panel>

            <Panel variant="sunken" className={sectionClass}>
              <Heading size="sub" as="h2" className="mb-[14px]">
                {t("settings.sectionContentSource")}
              </Heading>
              {settings ? (
                <div className="flex flex-col gap-[8px]">
                  <label htmlFor="cf-api-key" className="text-[14px] text-fg">{t("settings.cfApiKey")}</label>
                  <input
                    id="cf-api-key"
                    name="cf-api-key"
                    type="password"
                    autoComplete="off"
                    className="w-full rounded-none bg-sidebar shadow-input px-[12px] h-[36px] text-[13px] text-fg placeholder:text-faint border-none outline-none focus-visible:ring-2 focus-visible:ring-accent"
                    placeholder={t("settings.cfApiKeyPlaceholder")}
                    value={settings.cf_api_key ?? ""}
                    onChange={(e) => setSettings({ ...settings, cf_api_key: e.currentTarget.value || null })}
                    onBlur={(e) => patchSettings({ cf_api_key: e.currentTarget.value.trim() || null })}
                  />
                  <span className="text-[12px] text-muted">{t("settings.cfApiKeyHint")}</span>
                </div>
              ) : settingsError ? (
                <ErrorState message={t("settings.downloadSettingsFailed")} onRetry={() => void loadSettings()} />
              ) : (
                <div className="text-muted">{t("settings.loadingSettings")}</div>
              )}
            </Panel>

            <Panel variant="sunken" className={sectionClass}>
              <Heading size="sub" as="h2" className="mb-[14px]">
                {t("settings.sectionJava")}
              </Heading>
              {javas.loading ? (
                <div className="flex justify-center p-[20px]"><Spinner /></div>
              ) : (javas.data ?? []).length > 0 ? (
                <div className="flex flex-col gap-[8px]">
                  {(javas.data ?? []).map((j) => (
                    <Panel key={j.path} variant="raised" className="flex flex-col gap-[3px] px-[12px] py-[9px]">
                      <span className="flex items-center gap-[8px]">
                        <span className="font-display text-[14px] text-strong">Java {j.version}</span>
                        <Tag>{j.is_64bit ? t("settings.bit64") : t("settings.bit32")}</Tag>
                        <span className="text-[12px] text-accent">{j.source}</span>
                      </span>
                      <span className="text-[11px] text-faint break-all">{j.path}</span>
                    </Panel>
                  ))}
                </div>
              ) : javas.error ? (
                <ErrorState compact message={t("settings.javaDetectFailed")} onRetry={() => javas.refetch()} />
              ) : (
                <div className="text-muted">{t("settings.noJava")}</div>
              )}
            </Panel>

            <Panel variant="sunken" className={sectionClass}>
              <Heading size="sub" as="h2" className="mb-[14px]">
                {t("settings.sectionGameDir")}
              </Heading>
              {roots.loading ? (
                <div className="flex justify-center p-[20px]"><Spinner /></div>
              ) : (
                <div className="flex flex-col gap-[8px]">
                  {(roots.data ?? []).map((r) => {
                    const active = (currentRoot ?? "") === r.path;
                    return (
                      <Panel
                        key={r.path}
                        variant="raised"
                        className={clsx("flex items-center gap-[10px] py-[9px] px-[12px]", { "ring-2 ring-accent": active })}
                      >
                        <button
                          className="flex-1 min-w-0 flex flex-col gap-[2px] text-left bg-transparent border-none p-0 cursor-pointer"
                          onClick={() => setCurrentRoot(r.path)}
                          title={t("settings.setAsCurrent")}
                        >
                          <span className="flex items-center gap-[6px] text-[13px] text-fg">
                            {r.name}
                            {active && (
                              <span className="text-accent text-[12px]" aria-hidden="true">{t("settings.current")}</span>
                            )}
                          </span>
                          <span className="text-[11px] text-faint break-all">{r.path}</span>
                        </button>
                        {r.kind === "custom" && (
                          <button
                            className="shrink-0 text-[12px] text-danger-text px-[8px] py-[4px] rounded-none cursor-pointer hover:bg-danger-soft"
                            onClick={() => void removeCustomRoot(r.path)}
                          >
                            {t("settings.remove")}
                          </button>
                        )}
                      </Panel>
                    );
                  })}
                  <Button variant="primary" className="self-start" onClick={() => void addCustomRoot()}>
                    {t("settings.addDir")}
                  </Button>
                </div>
              )}
            </Panel>

            <Panel variant="sunken" className={sectionClass}>
              <Heading size="sub" as="h2" className="mb-[14px]">
                {t("settings.sectionDiagnostics")}
              </Heading>
              <div className="flex items-center justify-between text-fg text-[14px]">
                <div className="flex flex-col gap-[2px] min-w-0">
                  <span>{t("settings.logs")}</span>
                  <span className="text-[12px] text-muted">{t("settings.logsDesc")}</span>
                </div>
                <div className="flex items-center gap-[8px] shrink-0">
                  <Button variant="ghost" onClick={toggleLog}>
                    {logOpen ? t("settings.hideLog") : t("settings.viewLog")}
                  </Button>
                  <Button
                    variant="ghost"
                    onClick={async () => {
                      try {
                        const dir = await api.openLogsDir();
                        await api.revealPath(dir);
                      } catch (e) {
                        toast({ type: "error", message: t("settings.openLogsDirFailed", { err: String(e) }) });
                      }
                    }}
                  >
                    {t("settings.openLogsDir")}
                  </Button>
                </div>
              </div>

              {logOpen && (
                <div className="mt-[12px]">
                  <div className="flex justify-end mb-[6px]">
                    <button
                      className="text-[12px] text-muted hover:text-fg disabled:opacity-50 cursor-pointer bg-transparent border-none transition-colors duration-150"
                      onClick={() => void loadLog()}
                      disabled={logLoading}
                    >
                      {logLoading ? t("settings.refreshing") : t("settings.refresh")}
                    </button>
                  </div>
                  <pre
                    ref={logBox}
                    className="max-h-[340px] overflow-auto m-0 bg-sidebar shadow-input rounded-none p-[12px] text-[11px] leading-[1.55] text-muted font-mono whitespace-pre-wrap break-all"
                  >
                    {logText || t("settings.logEmpty")}
                  </pre>
                </div>
              )}
            </Panel>
          </div>
        </div>
      </div>
    </div>
  );
}
