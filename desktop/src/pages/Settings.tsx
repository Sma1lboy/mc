import { Component, createMemo, createResource, createSignal, For, Show, onMount } from "solid-js";
import { Button, Spinner, Select, Tooltip, ErrorState, toast } from "../components";
import { Check, Info, Monitor, Moon, RotateCcw, Sun, type LucideIcon } from "lucide-solid";
import {
  applyThemeColor,
  setMode,
  saveTheme,
  PRESETS,
  themeForLayout,
  normalizeThemeConfig,
} from "../theme/theme";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api } from "../ipc/api";
import { ACCENT_BTN } from "../components/styles";
import {
  layoutMode,
  switchLayout,
  veilStrength,
  setVeilStrength,
  currentRoot,
  setCurrentRoot,
} from "../store";
import type { LayoutMode } from "../store";
import { t, locale, setLocale, type Locale } from "../i18n";
import type { ThemeConfig, ThemeMode, GlobalSettings } from "../ipc/types";
import "./Settings.css";

type SegmentOption<T extends string> = {
  value: T;
  label: string;
  icon?: LucideIcon;
};

interface SegmentedControlProps<T extends string> {
  ariaLabel: string;
  value: T;
  options: SegmentOption<T>[];
  onChange: (value: T) => void;
}

function SegmentedControl<T extends string>(props: SegmentedControlProps<T>) {
  return (
    <div
      class="inline-flex items-center gap-[2px] rounded-ctl border border-glass-border bg-glass-card p-[3px]"
      role="group"
      aria-label={props.ariaLabel}
    >
      <For each={props.options}>
        {(option) => {
          const selected = () => props.value === option.value;
          return (
            <button
              type="button"
              class="inline-flex h-[30px] items-center justify-center gap-[6px] rounded-xs border-none px-[10px] text-[13px] font-medium leading-none cursor-pointer select-none transition-[background-color,color,box-shadow] duration-[var(--dur)] ease-app focus-visible:ring-2 focus-visible:ring-a-4 focus-visible:ring-offset-2 focus-visible:ring-offset-n-3 disabled:opacity-50 disabled:cursor-not-allowed"
              classList={{
                "bg-a-4 text-white": selected(),
                "bg-transparent text-fg hover:bg-glass-hover": !selected(),
              }}
              aria-pressed={selected()}
              onClick={() => props.onChange(option.value)}
            >
              <Show when={option.icon}>
                {(icon) => {
                  const Icon = icon();
                  return <Icon size={14} aria-hidden="true" />;
                }}
              </Show>
              <span>{option.label}</span>
            </button>
          );
        }}
      </For>
    </div>
  );
}

const SECTION_CLASS = "settings-section";

/**
 * Settings —— 主题(HSL 三滑块实时换肤 + 预设 + 深浅切换)
 * 以及 Java 检测列表。改动即时应用,失焦/操作后持久化到后端。
 */
const Settings: Component = () => {
  const [mode, setModeSig] = createSignal<ThemeMode>("dark");
  const [hue, setHue] = createSignal(150);
  const [sat, setSat] = createSignal(60);
  const [light, setLight] = createSignal(45);

  // 全局设置(下载源/并发/默认内存/Java)—— 全部来自后端 daemon。
  const [settings, setSettings] = createSignal<GlobalSettings | null>(null);
  const [settingsError, setSettingsError] = createSignal(false);

  // 加载全局设置(可重试):失败置错误态,区分「加载中」与「加载失败」。
  async function loadSettings() {
    setSettingsError(false);
    try {
      setSettings(await api.getSettings());
    } catch {
      setSettingsError(true);
    }
  }

  const languageOptions: SegmentOption<Locale>[] = [
    { value: "zh", label: "中文" },
    { value: "en", label: "English" },
  ];

  const layoutOptions = (): SegmentOption<LayoutMode>[] => [
    { value: "workspace", label: t("settings.layoutWorkspace") },
    { value: "classic", label: t("settings.layoutClassic") },
  ];

  const themeModeOptions = (): SegmentOption<ThemeMode>[] => [
    { value: "light", label: t("settings.themeModeLight"), icon: Sun },
    { value: "dark", label: t("settings.themeModeDark"), icon: Moon },
    { value: "system", label: t("settings.themeModeSystem"), icon: Monitor },
  ];

  // 初始化:从后端取当前主题 + 全局设置。
  onMount(async () => {
    try {
      const { config: cfg, changed } = normalizeThemeConfig(await api.getTheme(), layoutMode());
      setLocalTheme(cfg);
      if (changed) void saveTheme(cfg).catch(() => {});
    } catch {
      setLocalTheme(themeForLayout(layoutMode()));
    }
    void loadSettings();
  });

  // 改一项全局设置并立即持久化到后端。
  function patchSettings(patch: Partial<GlobalSettings>) {
    const cur = settings();
    if (!cur) return;
    const next = { ...cur, ...patch };
    setSettings(next);
    void api.setSettings(next).catch(() => {});
  }

  // 游戏目录(根)：列出已发现的根 + 让用户切换/增删自定义根。
  const [roots, { refetch: refetchRoots }] = createResource(() => api.listRoots());

  // 把自定义根写盘后再让后端重新发现:setSettings 是异步落盘,必须 await 再 refetch,
  // 否则 list_roots 可能读到旧设置(看不到刚加的根)。
  async function persistCustomRoots(customRoots: string[]) {
    const cur = settings();
    if (!cur) return;
    const next = { ...cur, custom_roots: customRoots };
    setSettings(next);
    try {
      await api.setSettings(next);
      await refetchRoots();
    } catch (e) {
      toast({ type: "error", message: t("settings.saveDirFailed", { err: String(e) }) });
    }
  }

  async function addCustomRoot() {
    const picked = await openDialog({ directory: true, title: t("settings.pickGameDir") }).catch(() => null);
    if (!picked || typeof picked !== "string") return;
    // 已在已发现列表(便携/官方/已有自定义)里 → 直接切过去,不往设置里塞重复项。
    if ((roots() ?? []).some((r) => r.path === picked)) {
      setCurrentRoot(picked);
      return;
    }
    const list = settings()?.custom_roots ?? [];
    if (!list.includes(picked)) await persistCustomRoots([...list, picked]);
    setCurrentRoot(picked); // 切到新加的根
  }

  async function removeCustomRoot(path: string) {
    const list = (settings()?.custom_roots ?? []).filter((p) => p !== path);
    await persistCustomRoots(list);
    if (currentRoot() === path) {
      // 删的是当前根:落到剩余的第一个(没有则交回后端默认)。
      setCurrentRoot((roots() ?? [])[0]?.path ?? null);
    }
  }

  const [javas, { refetch: refetchJavas }] = createResource(() => api.detectJava());

  // 应用内日志查看(诊断):按需读取最新日志文件的末尾,展开时滚到底看最新。
  const [logOpen, setLogOpen] = createSignal(false);
  const [logText, setLogText] = createSignal("");
  const [logLoading, setLogLoading] = createSignal(false);
  let logBox: HTMLPreElement | undefined;
  async function loadLog() {
    setLogLoading(true);
    try {
      setLogText(await api.readLogTail(800));
    } catch (e) {
      setLogText(t("settings.logReadFailed", { err: String(e) }));
    } finally {
      setLogLoading(false);
      queueMicrotask(() => logBox && (logBox.scrollTop = logBox.scrollHeight));
    }
  }
  function toggleLog() {
    const next = !logOpen();
    setLogOpen(next);
    if (next) void loadLog();
  }

  // Java 下拉选项:自动检测 + 检测到的各 JVM。
  const javaOptions = createMemo(() => [
    { label: t("settings.autoDetect"), value: "" },
    ...(javas() ?? []).map((j) => ({ label: `Java ${j.version} — ${j.path}`, value: j.path })),
  ]);

  function current(): ThemeConfig {
    return { mode: mode(), hue: hue(), saturation: sat(), lightness: light() };
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
    applyThemeColor(hue(), sat(), light());
    setMode(mode());
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

  function pickPreset(p: { hue: number; saturation: number; lightness: number }) {
    setHue(p.hue);
    setSat(p.saturation);
    setLight(p.lightness);
    apply();
  }

  function isSelectedPreset(p: { hue: number; saturation: number; lightness: number }): boolean {
    return (
      Math.abs(hue() - p.hue) < 0.5 &&
      Math.abs(sat() - p.saturation) < 0.5 &&
      Math.abs(light() - p.lightness) < 0.5
    );
  }

  // 恢复当前布局相称的默认主题(经典→浅色蓝 / 工作台→深色绿),立即应用并持久化。
  // 关键:按布局取默认,避免在经典布局里重置成深色、出现顶栏浅正文深的诡异组合。
  function resetTheme() {
    const def = themeForLayout(layoutMode());
    setLocalTheme(def);
    void saveTheme(def).catch(() => {});
    toast({ type: "success", message: t("settings.resetThemeDone") });
  }

  return (
    <div class="settings-page">
      <div class="settings-page__inner">
        <h1 class="text-[24px] font-bold text-fg mt-0 mb-[20px] mx-0">{t("settings.title")}</h1>

        <div class="settings-page__grid">
          <div class="settings-page__main">
            <section class={SECTION_CLASS}>
              <h2 class="text-[15px] font-semibold text-fg mt-0 mb-[14px] mx-0">{t("settings.sectionLayout")}</h2>
              <div class="flex items-center justify-between mb-[14px] text-fg text-[14px]">
                <span>{t("settings.language")}</span>
                <SegmentedControl
                  ariaLabel={t("settings.langAriaLabel")}
                  value={locale()}
                  options={languageOptions}
                  onChange={setLocale}
                />
              </div>
              <div class="flex items-center justify-between mb-[14px] text-fg text-[14px]">
                <span>{t("settings.style")}</span>
                <SegmentedControl
                  ariaLabel={t("settings.layoutAriaLabel")}
                  value={layoutMode()}
                  options={layoutOptions()}
                  onChange={switchLayout}
                />
              </div>
            </section>

            <section class={SECTION_CLASS}>
              <h2 class="text-[15px] font-semibold text-fg mt-0 mb-[14px] mx-0">{t("settings.sectionAppearance")}</h2>

              <div class="flex items-center justify-between mb-[14px] text-fg text-[14px]">
                <span>{t("settings.themeMode")}</span>
                <SegmentedControl
                  ariaLabel={t("settings.themeMode")}
                  value={mode()}
                  options={themeModeOptions()}
                  onChange={selectThemeMode}
                />
              </div>

              <div class="flex items-center justify-between mb-[14px] text-fg text-[14px]">
                <span>{t("settings.presetAccent")}</span>
                <div class="flex gap-[8px]">
                  <For each={PRESETS}>
                    {(p) => (
                      <button
                        type="button"
                        class="relative grid w-[26px] h-[26px] place-items-center rounded-full border-2 border-n-6 cursor-pointer transition-[transform,border-color,box-shadow] duration-[var(--dur)] ease-app hover:scale-[1.12] focus-visible:ring-2 focus-visible:ring-a-4 focus-visible:ring-offset-2 focus-visible:ring-offset-n-3"
                        classList={{
                          "border-white shadow-[0_0_0_2px_var(--a-4)]": isSelectedPreset(p),
                        }}
                        style={{ background: p.hex }}
                        title={p.name}
                        aria-label={t("settings.presetAccentAria", { name: p.name })}
                        aria-pressed={isSelectedPreset(p)}
                        onClick={() => pickPreset(p)}
                      >
                        <Show when={isSelectedPreset(p)}>
                          <Check size={13} aria-hidden="true" class="text-white" />
                        </Show>
                      </button>
                    )}
                  </For>
                </div>
              </div>

              <div class="flex flex-col gap-[4px] mb-[12px]">
                <label for="theme-hue" class="text-[12px] text-dim">
                  {t("settings.hue")} {Math.round(hue())}
                </label>
                <input
                  id="theme-hue"
                  name="theme-hue"
                  autocomplete="off"
                  class="w-full accent-[var(--a-4)]"
                  type="range"
                  aria-label={t("settings.hue")}
                  min="0"
                  max="360"
                  value={hue()}
                  onInput={(e) => {
                    setHue(+e.currentTarget.value);
                    apply(false);
                  }}
                  onChange={() => apply()}
                />
              </div>
              <div class="flex flex-col gap-[4px] mb-[12px]">
                <label for="theme-saturation" class="text-[12px] text-dim">
                  {t("settings.saturation")} {Math.round(sat())}
                </label>
                <input
                  id="theme-saturation"
                  name="theme-saturation"
                  autocomplete="off"
                  class="w-full accent-[var(--a-4)]"
                  type="range"
                  aria-label={t("settings.saturation")}
                  min="0"
                  max="100"
                  value={sat()}
                  onInput={(e) => {
                    setSat(+e.currentTarget.value);
                    apply(false);
                  }}
                  onChange={() => apply()}
                />
              </div>
              <div class="flex flex-col gap-[4px] mb-[12px]">
                <label for="theme-lightness" class="text-[12px] text-dim">
                  {t("settings.lightness")} {Math.round(light())}
                </label>
                <input
                  id="theme-lightness"
                  name="theme-lightness"
                  autocomplete="off"
                  class="w-full accent-[var(--a-4)]"
                  type="range"
                  aria-label={t("settings.lightness")}
                  min="20"
                  max="70"
                  value={light()}
                  onInput={(e) => {
                    setLight(+e.currentTarget.value);
                    apply(false);
                  }}
                  onChange={() => apply()}
                />
              </div>

              <div class="flex flex-col gap-[4px] mb-[12px]">
                <label for="ui-transparency" class="text-[12px] text-dim">
                  {t("settings.uiTransparency")} {Math.round((1 - veilStrength()) * 100)}%
                </label>
                <input
                  id="ui-transparency"
                  name="ui-transparency"
                  autocomplete="off"
                  class="w-full accent-[var(--a-4)]"
                  type="range"
                  aria-label={t("settings.uiTransparency")}
                  min="0"
                  max="65"
                  step="1"
                  value={Math.round((1 - veilStrength()) * 100)}
                  onInput={(e) => setVeilStrength(1 - +e.currentTarget.value / 100)}
                />
              </div>

              <div class="flex gap-[4px] mt-[12px]">
                <For each={[1, 2, 3, 4, 5, 6, 7, 8]}>
                  {(i) => <span class="flex-1 h-[24px] rounded-xs" style={{ background: `var(--a-${i})` }} />}
                </For>
              </div>

              <div class="flex items-center justify-between mb-[14px] text-fg text-[14px]">
                <span>{t("settings.resetTheme")}</span>
                <Button variant="ghost" onClick={resetTheme}>
                  <RotateCcw size={14} aria-hidden="true" />
                  {t("settings.reset")}
                </Button>
              </div>
            </section>
          </div>

          <div class="settings-page__side">
            <section class={SECTION_CLASS}>
              <h2 class="text-[15px] font-semibold text-fg mt-0 mb-[14px] mx-0">{t("settings.sectionDownloadGame")}</h2>
              <Show
                when={settings()}
                fallback={
                  settingsError()
                    ? <ErrorState message={t("settings.downloadSettingsFailed")} onRetry={() => void loadSettings()} />
                    : <div class="text-dim">{t("settings.loadingSettings")}</div>
                }
              >
                <div class="flex items-center justify-between mb-[14px] text-fg text-[14px]">
                  <span class="flex items-center gap-[6px]">
                    {t("settings.downloadSource")}
                    <Tooltip content={t("settings.downloadSourceTip")}>
                      <Info size={14} aria-hidden="true" />
                    </Tooltip>
                  </span>
                  <SegmentedControl
                    ariaLabel={t("settings.downloadSource")}
                    value={
                      settings()!.use_mirror || settings()!.download_source === "bmclapi"
                        ? "bmclapi"
                        : "official"
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

                <div class="flex flex-col gap-[4px] mb-[12px]">
                  <label for="download-concurrency" class="text-[12px] text-dim">
                    {t("settings.concurrency")} {settings()!.concurrency}
                  </label>
                  <input
                    id="download-concurrency"
                    name="download-concurrency"
                    autocomplete="off"
                    class="w-full accent-[var(--a-4)]"
                    type="range"
                    aria-label={t("settings.concurrency")}
                    min="1"
                    max="128"
                    value={settings()!.concurrency}
                    onInput={(e) => setSettings({ ...settings()!, concurrency: +e.currentTarget.value })}
                    onChange={() => patchSettings({})}
                  />
                </div>

                <div class="flex flex-col gap-[4px] mb-[12px]">
                  <label for="default-memory" class="text-[12px] text-dim">
                    {t("settings.defaultMemory")} {settings()!.default_memory_mb} MiB
                  </label>
                  <input
                    id="default-memory"
                    name="default-memory"
                    autocomplete="off"
                    class="w-full accent-[var(--a-4)]"
                    type="range"
                    aria-label={t("settings.defaultMemory")}
                    min="512"
                    max="16384"
                    step="256"
                    value={settings()!.default_memory_mb}
                    onInput={(e) =>
                      setSettings({ ...settings()!, default_memory_mb: +e.currentTarget.value })
                    }
                    onChange={() => patchSettings({})}
                  />
                </div>

                <div class="flex items-center justify-between mb-[14px] text-fg text-[14px]">
                  <span>{t("settings.java")}</span>
                  <Select
                    class="max-w-[60%]"
                    value={settings()!.java_path ?? ""}
                    onChange={(v) => patchSettings({ java_path: v || null })}
                    options={javaOptions()}
                    placeholder={t("settings.autoDetect")}
                  />
                </div>
              </Show>
            </section>

            <section class={SECTION_CLASS}>
              <h2 class="text-[15px] font-semibold text-fg mt-0 mb-[14px] mx-0">{t("settings.sectionJava")}</h2>
              <Show when={!javas.loading} fallback={<div class="flex justify-center p-[20px]"><Spinner /></div>}>
                <Show
                  when={(javas() ?? []).length > 0}
                  fallback={
                    javas.error
                      ? <ErrorState compact message={t("settings.javaDetectFailed")} onRetry={() => void refetchJavas()} />
                      : <div class="text-dim">{t("settings.noJava")}</div>
                  }
                >
                  <div class="flex flex-col gap-[8px]">
                    <For each={javas()}>
                      {(j) => (
                        <div class="flex flex-col gap-[2px] px-[10px] py-[8px] glass-card rounded-ctl">
                          <span class="font-semibold text-fg">Java {j.version}</span>
                          <span class="text-[12px] text-a-5">
                            {j.is_64bit ? t("settings.bit64") : t("settings.bit32")} · {j.source}
                          </span>
                          <span class="text-[11px] text-dim break-all">{j.path}</span>
                        </div>
                      )}
                    </For>
                  </div>
                </Show>
              </Show>
            </section>

            <section class={SECTION_CLASS}>
              <h2 class="text-[15px] font-semibold text-fg mt-0 mb-[14px] mx-0">{t("settings.sectionGameDir")}</h2>
              <Show
                when={!roots.loading}
                fallback={<div class="flex justify-center p-[20px]"><Spinner /></div>}
              >
                <div class="flex flex-col gap-[8px]">
                  <For each={roots() ?? []}>
                    {(r) => {
                      const active = () => (currentRoot() ?? "") === r.path;
                      return (
                        <div
                          class="flex items-center gap-[10px] py-[8px] px-[10px] rounded-ctl bg-glass-card border border-transparent transition-colors duration-150"
                          classList={{ "!border-a-4 bg-a-1": active() }}
                        >
                          <button
                            class="flex-1 min-w-0 flex flex-col gap-[2px] text-left bg-transparent border-none p-0 cursor-pointer"
                            onClick={() => setCurrentRoot(r.path)}
                            title={t("settings.setAsCurrent")}
                          >
                            <span class="flex items-center gap-[6px] text-[13px] text-fg">
                              {r.name}
                              <Show when={active()}>
                                <span class="text-a-6 text-[12px]" aria-hidden="true">{t("settings.current")}</span>
                              </Show>
                            </span>
                            <span class="text-[11px] text-dim break-all">{r.path}</span>
                          </button>
                          <Show when={r.kind === "custom"}>
                            <button
                              class="shrink-0 text-[12px] text-danger-text px-[8px] py-[4px] rounded-xs cursor-pointer hover:bg-danger-soft"
                              onClick={() => void removeCustomRoot(r.path)}
                            >
                              {t("settings.remove")}
                            </button>
                          </Show>
                        </div>
                      );
                    }}
                  </For>
                  <button class={`${ACCENT_BTN} self-start`} onClick={() => void addCustomRoot()}>
                    {t("settings.addDir")}
                  </button>
                </div>
              </Show>
            </section>

            <section class={SECTION_CLASS}>
              <h2 class="text-[15px] font-semibold text-fg mt-0 mb-[14px] mx-0">{t("settings.sectionDiagnostics")}</h2>
              <div class="flex items-center justify-between text-fg text-[14px]">
                <div class="flex flex-col gap-[2px] min-w-0">
                  <span>{t("settings.logs")}</span>
                  <span class="text-[12px] text-dim">{t("settings.logsDesc")}</span>
                </div>
                <div class="flex items-center gap-[8px] shrink-0">
                  <Button variant="ghost" onClick={toggleLog}>
                    {logOpen() ? t("settings.hideLog") : t("settings.viewLog")}
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

              <Show when={logOpen()}>
                <div class="mt-[12px]">
                  <div class="flex justify-end mb-[6px]">
                    <button
                      class="text-[12px] text-dim hover:text-fg disabled:opacity-50 cursor-pointer bg-transparent border-none transition-colors duration-150"
                      onClick={() => void loadLog()}
                      disabled={logLoading()}
                    >
                      {logLoading() ? t("settings.refreshing") : t("settings.refresh")}
                    </button>
                  </div>
                  <pre
                    ref={logBox}
                    class="max-h-[340px] overflow-auto m-0 rounded-ctl border border-glass-border bg-glass-card p-[10px] text-[11px] leading-[1.55] text-dim font-mono whitespace-pre-wrap break-all"
                  >
                    {logText() || t("settings.logEmpty")}
                  </pre>
                </div>
              </Show>
            </section>
          </div>
        </div>
      </div>
    </div>
  );
};

export default Settings;
