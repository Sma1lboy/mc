import { Component, createMemo, createResource, createSignal, For, Show, onMount } from "solid-js";
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
} from "../components";
import { KobeAccountPanel } from "../components/KobeAccountPanel";
import { Check, Info, RotateCcw } from "lucide-solid";
import {
  applyThemeColor,
  setMode,
  saveTheme,
  PRESETS,
  DEFAULT_THEME,
  normalizeThemeConfig,
} from "../theme/theme";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api } from "../ipc/api";
import {
  veilStrength,
  setVeilStrength,
  currentRoot,
  setCurrentRoot,
} from "../store";
import { t, locale, setLocale, type Locale } from "../i18n";
import type { ThemeConfig, ThemeMode, GlobalSettings } from "../ipc/types";
import "./Settings.css";

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
  onMount(async () => {
    try {
      const { config: cfg, changed } = normalizeThemeConfig(await api.getTheme());
      setLocalTheme(cfg);
      if (changed) void saveTheme(cfg).catch(() => {});
    } catch {
      setLocalTheme(DEFAULT_THEME);
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

  // 默认内存:MiB → "2G"/"1.5G"/"768M" 的友好显示。
  function formatMem(mib: number): string {
    if (mib >= 1024) {
      const g = mib / 1024;
      return `${Number.isInteger(g) ? g : g.toFixed(1)}G`;
    }
    return `${mib}M`;
  }

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

  // 恢复默认主题(深色 + 陶土橙),立即应用并持久化。
  function resetTheme() {
    setLocalTheme(DEFAULT_THEME);
    void saveTheme(DEFAULT_THEME).catch(() => {});
    toast({ type: "success", message: t("settings.resetThemeDone") });
  }

  // 块标题(像素体) + 内距统一的设置块。
  const sectionClass = "p-[20px]";

  return (
    <div class="settings-page">
      <div class="settings-page__inner">
        <Heading size="page" as="h1" class="mt-0 mb-[20px]">
          {t("settings.title")}
        </Heading>

        <div class="settings-page__grid">
          <div class="settings-page__main">
            <Panel variant="sunken" class={sectionClass}>
              <Heading size="sub" as="h2" class="mb-[14px]">
                {t("settings.sectionLanguage")}
              </Heading>
              <div class="flex items-center justify-between text-fg text-[14px]">
                <span>{t("settings.language")}</span>
                <Segmented
                  ariaLabel={t("settings.langAriaLabel")}
                  value={locale()}
                  options={languageOptions()}
                  onChange={setLocale}
                />
              </div>
            </Panel>

            <Panel variant="sunken" class={sectionClass}>
              <Heading size="sub" as="h2" class="mb-[14px]">
                {t("settings.sectionAppearance")}
              </Heading>

              <div class="flex items-center justify-between mb-[16px] text-fg text-[14px]">
                <span>{t("settings.themeMode")}</span>
                <Segmented
                  ariaLabel={t("settings.themeMode")}
                  value={mode()}
                  options={themeModeOptions()}
                  onChange={selectThemeMode}
                />
              </div>

              <div class="flex items-center justify-between mb-[16px] text-fg text-[14px]">
                <span>{t("settings.presetAccent")}</span>
                <div class="flex gap-[8px]">
                  <For each={PRESETS}>
                    {(p) => (
                      <button
                        type="button"
                        class="relative grid w-[28px] h-[28px] place-items-center rounded-none shadow-raised cursor-pointer transition-[transform,box-shadow] duration-[var(--dur)] ease-app hover:-translate-y-[1px] active:shadow-pressed focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
                        classList={{
                          "ring-2 ring-tag": isSelectedPreset(p),
                        }}
                        style={{ background: p.hex }}
                        title={p.name}
                        aria-label={t("settings.presetAccentAria", { name: p.name })}
                        aria-pressed={isSelectedPreset(p)}
                        onClick={() => pickPreset(p)}
                      >
                        <Show when={isSelectedPreset(p)}>
                          <Check size={14} aria-hidden="true" class="text-white drop-shadow-[0_1px_1px_rgba(0,0,0,0.6)]" />
                        </Show>
                      </button>
                    )}
                  </For>
                </div>
              </div>

              <Slider
                class="mb-[14px]"
                label={t("settings.hue")}
                display={(v) => String(Math.round(v))}
                min={0}
                max={360}
                value={hue()}
                ariaLabel={t("settings.hue")}
                onInput={(v) => {
                  setHue(v);
                  apply();
                }}
              />
              <Slider
                class="mb-[14px]"
                label={t("settings.saturation")}
                display={(v) => String(Math.round(v))}
                min={0}
                max={100}
                value={sat()}
                ariaLabel={t("settings.saturation")}
                onInput={(v) => {
                  setSat(v);
                  apply();
                }}
              />
              <Slider
                class="mb-[14px]"
                label={t("settings.lightness")}
                display={(v) => String(Math.round(v))}
                min={20}
                max={70}
                value={light()}
                ariaLabel={t("settings.lightness")}
                onInput={(v) => {
                  setLight(v);
                  apply();
                }}
              />
              <Slider
                class="mb-[16px]"
                label={t("settings.uiTransparency")}
                display={(v) => `${v}%`}
                min={0}
                max={65}
                step={1}
                value={Math.round((1 - veilStrength()) * 100)}
                ariaLabel={t("settings.uiTransparency")}
                onInput={(v) => setVeilStrength(1 - v / 100)}
              />

              <div class="flex gap-[3px] mb-[16px]">
                <For each={[1, 2, 3, 4, 5, 6, 7, 8]}>
                  {(i) => (
                    <span
                      class="flex-1 h-[24px] rounded-none shadow-sunken"
                      style={{ background: `var(--a-${i})` }}
                    />
                  )}
                </For>
              </div>

              <div class="flex items-center justify-between text-fg text-[14px]">
                <span>{t("settings.resetTheme")}</span>
                <Button variant="ghost" onClick={resetTheme}>
                  <RotateCcw size={14} aria-hidden="true" />
                  {t("settings.reset")}
                </Button>
              </div>
            </Panel>
          </div>

          <div class="settings-page__side">
            <KobeAccountPanel />

            <Panel variant="sunken" class={sectionClass}>
              <Heading size="sub" as="h2" class="mb-[14px]">
                {t("settings.sectionDownloadGame")}
              </Heading>
              <Show
                when={settings()}
                fallback={
                  settingsError()
                    ? <ErrorState message={t("settings.downloadSettingsFailed")} onRetry={() => void loadSettings()} />
                    : <div class="text-muted">{t("settings.loadingSettings")}</div>
                }
              >
                <div class="flex items-center justify-between mb-[16px] text-fg text-[14px]">
                  <span class="flex items-center gap-[6px]">
                    {t("settings.downloadSource")}
                    <Tooltip content={t("settings.downloadSourceTip")}>
                      <Info size={14} aria-hidden="true" />
                    </Tooltip>
                  </span>
                  <Segmented
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

                <Slider
                  class="mb-[16px]"
                  label={t("settings.concurrency")}
                  min={1}
                  max={128}
                  value={settings()!.concurrency ?? 64}
                  ariaLabel={t("settings.concurrency")}
                  onInput={(v) => patchSettings({ concurrency: v })}
                />

                <Slider
                  class="mb-[16px]"
                  label={t("settings.defaultMemory")}
                  display={formatMem}
                  min={512}
                  max={16384}
                  step={256}
                  value={settings()!.default_memory_mb ?? 2048}
                  ariaLabel={t("settings.defaultMemory")}
                  onInput={(v) => patchSettings({ default_memory_mb: v })}
                />

                <div class="flex items-center justify-between text-fg text-[14px]">
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
            </Panel>

            <Panel variant="sunken" class={sectionClass}>
              <Heading size="sub" as="h2" class="mb-[14px]">
                {t("settings.sectionContentSource")}
              </Heading>
              <Show
                when={settings()}
                fallback={
                  settingsError()
                    ? <ErrorState message={t("settings.downloadSettingsFailed")} onRetry={() => void loadSettings()} />
                    : <div class="text-muted">{t("settings.loadingSettings")}</div>
                }
              >
                <div class="flex flex-col gap-[8px]">
                  <label for="cf-api-key" class="text-[14px] text-fg">{t("settings.cfApiKey")}</label>
                  <input
                    id="cf-api-key"
                    name="cf-api-key"
                    type="password"
                    autocomplete="off"
                    class="w-full rounded-none bg-sidebar shadow-input px-[12px] h-[36px] text-[13px] text-fg placeholder:text-faint border-none outline-none focus-visible:ring-2 focus-visible:ring-accent"
                    placeholder={t("settings.cfApiKeyPlaceholder")}
                    value={settings()!.cf_api_key ?? ""}
                    onInput={(e) => setSettings({ ...settings()!, cf_api_key: e.currentTarget.value || null })}
                    onChange={(e) => patchSettings({ cf_api_key: e.currentTarget.value.trim() || null })}
                  />
                  <span class="text-[12px] text-muted">{t("settings.cfApiKeyHint")}</span>
                </div>
              </Show>
            </Panel>

            <Panel variant="sunken" class={sectionClass}>
              <Heading size="sub" as="h2" class="mb-[14px]">
                {t("settings.sectionJava")}
              </Heading>
              <Show when={!javas.loading} fallback={<div class="flex justify-center p-[20px]"><Spinner /></div>}>
                <Show
                  when={(javas() ?? []).length > 0}
                  fallback={
                    javas.error
                      ? <ErrorState compact message={t("settings.javaDetectFailed")} onRetry={() => void refetchJavas()} />
                      : <div class="text-muted">{t("settings.noJava")}</div>
                  }
                >
                  <div class="flex flex-col gap-[8px]">
                    <For each={javas()}>
                      {(j) => (
                        <Panel variant="raised" class="flex flex-col gap-[3px] px-[12px] py-[9px]">
                          <span class="flex items-center gap-[8px]">
                            <span class="font-display text-[14px] text-strong">Java {j.version}</span>
                            <Tag>{j.is_64bit ? t("settings.bit64") : t("settings.bit32")}</Tag>
                            <span class="text-[12px] text-accent">{j.source}</span>
                          </span>
                          <span class="text-[11px] text-faint break-all">{j.path}</span>
                        </Panel>
                      )}
                    </For>
                  </div>
                </Show>
              </Show>
            </Panel>

            <Panel variant="sunken" class={sectionClass}>
              <Heading size="sub" as="h2" class="mb-[14px]">
                {t("settings.sectionGameDir")}
              </Heading>
              <Show
                when={!roots.loading}
                fallback={<div class="flex justify-center p-[20px]"><Spinner /></div>}
              >
                <div class="flex flex-col gap-[8px]">
                  <For each={roots() ?? []}>
                    {(r) => {
                      const active = () => (currentRoot() ?? "") === r.path;
                      return (
                        <Panel
                          variant="raised"
                          class={`flex items-center gap-[10px] py-[9px] px-[12px]${active() ? " ring-2 ring-accent" : ""}`}
                        >
                          <button
                            class="flex-1 min-w-0 flex flex-col gap-[2px] text-left bg-transparent border-none p-0 cursor-pointer"
                            onClick={() => setCurrentRoot(r.path)}
                            title={t("settings.setAsCurrent")}
                          >
                            <span class="flex items-center gap-[6px] text-[13px] text-fg">
                              {r.name}
                              <Show when={active()}>
                                <span class="text-accent text-[12px]" aria-hidden="true">{t("settings.current")}</span>
                              </Show>
                            </span>
                            <span class="text-[11px] text-faint break-all">{r.path}</span>
                          </button>
                          <Show when={r.kind === "custom"}>
                            <button
                              class="shrink-0 text-[12px] text-danger-text px-[8px] py-[4px] rounded-none cursor-pointer hover:bg-danger-soft"
                              onClick={() => void removeCustomRoot(r.path)}
                            >
                              {t("settings.remove")}
                            </button>
                          </Show>
                        </Panel>
                      );
                    }}
                  </For>
                  <Button variant="primary" class="self-start" onClick={() => void addCustomRoot()}>
                    {t("settings.addDir")}
                  </Button>
                </div>
              </Show>
            </Panel>

            <Panel variant="sunken" class={sectionClass}>
              <Heading size="sub" as="h2" class="mb-[14px]">
                {t("settings.sectionDiagnostics")}
              </Heading>
              <div class="flex items-center justify-between text-fg text-[14px]">
                <div class="flex flex-col gap-[2px] min-w-0">
                  <span>{t("settings.logs")}</span>
                  <span class="text-[12px] text-muted">{t("settings.logsDesc")}</span>
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
                      class="text-[12px] text-muted hover:text-fg disabled:opacity-50 cursor-pointer bg-transparent border-none transition-colors duration-150"
                      onClick={() => void loadLog()}
                      disabled={logLoading()}
                    >
                      {logLoading() ? t("settings.refreshing") : t("settings.refresh")}
                    </button>
                  </div>
                  <pre
                    ref={logBox}
                    class="max-h-[340px] overflow-auto m-0 bg-sidebar shadow-input rounded-none p-[12px] text-[11px] leading-[1.55] text-muted font-mono whitespace-pre-wrap break-all"
                  >
                    {logText() || t("settings.logEmpty")}
                  </pre>
                </div>
              </Show>
            </Panel>
          </div>
        </div>
      </div>
    </div>
  );
};

export default Settings;
