import { Component, createMemo, createResource, createSignal, For, Show, onMount } from "solid-js";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
import { Button, Spinner, Select, Tooltip, toast } from "../components";
import { Check, Info, Monitor, Moon, RotateCcw, Sun, type LucideIcon } from "lucide-solid";
import {
  applyThemeColor,
  setMode,
  saveTheme,
  PRESETS,
  themeForLayout,
  normalizeThemeConfig,
} from "../theme/theme";
import { api } from "../ipc/api";
import { layoutMode, switchLayout } from "../store";
import type { LayoutMode } from "../store";
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
              class="inline-flex h-[30px] items-center justify-center gap-[6px] rounded-[4px] border-none px-[10px] text-[13px] font-medium leading-none cursor-pointer select-none transition-[background-color,color,box-shadow] duration-[var(--dur)] ease-app focus-visible:ring-2 focus-visible:ring-a-4 focus-visible:ring-offset-2 focus-visible:ring-offset-n-3 disabled:opacity-45 disabled:cursor-not-allowed"
              classList={{
                "bg-a-4 text-white shadow-[0_1px_4px_rgba(0,0,0,0.12)]": selected(),
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

  const layoutOptions: SegmentOption<LayoutMode>[] = [
    { value: "workspace", label: "工作台视图" },
    { value: "classic", label: "经典视图" },
  ];

  const themeModeOptions: SegmentOption<ThemeMode>[] = [
    { value: "light", label: "浅色", icon: Sun },
    { value: "dark", label: "深色", icon: Moon },
    { value: "system", label: "跟随系统", icon: Monitor },
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
    try {
      setSettings(await api.getSettings());
    } catch {
      /* 后端不可用时不显示该区块 */
    }
  });

  // 改一项全局设置并立即持久化到后端。
  function patchSettings(patch: Partial<GlobalSettings>) {
    const cur = settings();
    if (!cur) return;
    const next = { ...cur, ...patch };
    setSettings(next);
    void api.setSettings(next).catch(() => {});
  }

  const [javas] = createResource(() => api.detectJava());

  // Java 下拉选项:自动检测 + 检测到的各 JVM。
  const javaOptions = createMemo(() => [
    { label: "自动检测", value: "" },
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
    const label = next === "system" ? "跟随系统" : next === "dark" ? "深色" : "浅色";
    toast({ type: "info", message: `已切换到${label}` });
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
    toast({ type: "success", message: "已恢复默认主题" });
  }

  return (
    <div class="settings-page">
      <div class="settings-page__inner">
        <h1 class="text-[24px] font-bold text-fg mt-0 mb-[20px] mx-0">设置</h1>

        <div class="settings-page__grid">
          <div class="settings-page__main">
            <section class={SECTION_CLASS}>
              <h2 class="text-[15px] font-semibold text-fg mt-0 mb-[14px] mx-0">界面布局</h2>
              <div class="flex items-center justify-between mb-[14px] text-fg text-[14px]">
                <span>风格</span>
                <SegmentedControl
                  ariaLabel="界面布局"
                  value={layoutMode()}
                  options={layoutOptions}
                  onChange={switchLayout}
                />
              </div>
            </section>

            <section class={SECTION_CLASS}>
              <h2 class="text-[15px] font-semibold text-fg mt-0 mb-[14px] mx-0">外观</h2>

              <div class="flex items-center justify-between mb-[14px] text-fg text-[14px]">
                <span>主题模式</span>
                <SegmentedControl
                  ariaLabel="主题模式"
                  value={mode()}
                  options={themeModeOptions}
                  onChange={selectThemeMode}
                />
              </div>

              <div class="flex items-center justify-between mb-[14px] text-fg text-[14px]">
                <span>预设强调色</span>
                <div class="flex gap-[8px]">
                  <For each={PRESETS}>
                    {(p) => (
                      <button
                        type="button"
                        class="relative grid w-[26px] h-[26px] place-items-center rounded-full border-2 border-n-6 cursor-pointer transition-[transform,border-color,box-shadow] duration-[var(--dur)] ease-app hover:scale-[1.12] focus-visible:ring-2 focus-visible:ring-a-4 focus-visible:ring-offset-2 focus-visible:ring-offset-n-3"
                        classList={{
                          "border-white shadow-[0_0_0_2px_var(--a-4)]": isSelectedPreset(p),
                        }}
                        style={{ background: `hsl(${p.hue} ${p.saturation}% ${p.lightness}%)` }}
                        title={p.name}
                        aria-label={`使用${p.name}色强调色`}
                        aria-pressed={isSelectedPreset(p)}
                        onClick={() => pickPreset(p)}
                      >
                        <Show when={isSelectedPreset(p)}>
                          <Check size={13} aria-hidden="true" class="text-white drop-shadow-[0_1px_1px_rgba(0,0,0,0.35)]" />
                        </Show>
                      </button>
                    )}
                  </For>
                </div>
              </div>

              <div class="flex flex-col gap-[4px] mb-[12px]">
                <label for="theme-hue" class="text-[12px] text-dim">
                  色相 {Math.round(hue())}
                </label>
                <input
                  id="theme-hue"
                  name="theme-hue"
                  autocomplete="off"
                  class="w-full accent-[var(--a-4)]"
                  type="range"
                  aria-label="色相"
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
                  饱和度 {Math.round(sat())}
                </label>
                <input
                  id="theme-saturation"
                  name="theme-saturation"
                  autocomplete="off"
                  class="w-full accent-[var(--a-4)]"
                  type="range"
                  aria-label="饱和度"
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
                  明度 {Math.round(light())}
                </label>
                <input
                  id="theme-lightness"
                  name="theme-lightness"
                  autocomplete="off"
                  class="w-full accent-[var(--a-4)]"
                  type="range"
                  aria-label="明度"
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

              <div class="flex gap-[4px] mt-[12px]">
                <For each={[1, 2, 3, 4, 5, 6, 7, 8]}>
                  {(i) => <span class="flex-1 h-[24px] rounded-xs" style={{ background: `var(--a-${i})` }} />}
                </For>
              </div>

              <div class="flex items-center justify-between mb-[14px] text-fg text-[14px]">
                <span>恢复默认主题</span>
                <Button variant="ghost" onClick={resetTheme}>
                  <RotateCcw size={14} aria-hidden="true" />
                  重置
                </Button>
              </div>
            </section>
          </div>

          <div class="settings-page__side">
            <section class={SECTION_CLASS}>
              <h2 class="text-[15px] font-semibold text-fg mt-0 mb-[14px] mx-0">下载与游戏</h2>
              <Show when={settings()} fallback={<div class="text-dim">加载设置中…</div>}>
                <div class="flex items-center justify-between mb-[14px] text-fg text-[14px]">
                  <span class="flex items-center gap-[6px]">
                    下载源
                    <Tooltip content="官方直连走 Mojang/作者源,海外更稳;国内镜像走 BMCLAPI + McIM,国内更快。安装/导入整合包都按此设置选源。">
                      <Info size={14} aria-hidden="true" />
                    </Tooltip>
                  </span>
                  <SegmentedControl
                    ariaLabel="下载源"
                    value={
                      settings()!.use_mirror || settings()!.download_source === "bmclapi"
                        ? "bmclapi"
                        : "official"
                    }
                    options={[
                      { value: "official", label: "官方直连" },
                      { value: "bmclapi", label: "国内镜像" },
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
                    下载并发 {settings()!.concurrency}
                  </label>
                  <input
                    id="download-concurrency"
                    name="download-concurrency"
                    autocomplete="off"
                    class="w-full accent-[var(--a-4)]"
                    type="range"
                    aria-label="下载并发"
                    min="1"
                    max="128"
                    value={settings()!.concurrency}
                    onInput={(e) => setSettings({ ...settings()!, concurrency: +e.currentTarget.value })}
                    onChange={() => patchSettings({})}
                  />
                </div>

                <div class="flex flex-col gap-[4px] mb-[12px]">
                  <label for="default-memory" class="text-[12px] text-dim">
                    默认内存 {settings()!.default_memory_mb} MiB
                  </label>
                  <input
                    id="default-memory"
                    name="default-memory"
                    autocomplete="off"
                    class="w-full accent-[var(--a-4)]"
                    type="range"
                    aria-label="默认内存"
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
                  <span>Java</span>
                  <Select
                    class="max-w-[60%]"
                    value={settings()!.java_path ?? ""}
                    onChange={(v) => patchSettings({ java_path: v || null })}
                    options={javaOptions()}
                    placeholder="自动检测"
                  />
                </div>
              </Show>
            </section>

            <section class={SECTION_CLASS}>
              <h2 class="text-[15px] font-semibold text-fg mt-0 mb-[14px] mx-0">Java</h2>
              <Show when={!javas.loading} fallback={<Spinner />}>
                <Show
                  when={(javas() ?? []).length > 0}
                  fallback={<div class="text-dim">未检测到 Java。</div>}
                >
                  <div class="flex flex-col gap-[8px]">
                    <For each={javas()}>
                      {(j) => (
                        <div class="flex flex-col gap-[2px] px-[10px] py-[8px] glass-card rounded-ctl">
                          <span class="font-semibold text-fg">Java {j.version}</span>
                          <span class="text-[12px] text-a-5">
                            {j.is_64bit ? "64-bit" : "32-bit"} · {j.source}
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
              <h2 class="text-[15px] font-semibold text-fg mt-0 mb-[14px] mx-0">诊断</h2>
              <div class="flex items-center justify-between text-fg text-[14px]">
                <div class="flex flex-col gap-[2px] min-w-0">
                  <span>日志</span>
                  <span class="text-[12px] text-dim">前端与本地数据层的日志都写在这里(按日滚动)</span>
                </div>
                <Button
                  variant="ghost"
                  onClick={async () => {
                    try {
                      const dir = await api.openLogsDir();
                      await shellOpen(dir);
                    } catch (e) {
                      toast({ type: "error", message: `打开日志目录失败:${e}` });
                    }
                  }}
                >
                  打开日志目录
                </Button>
              </div>
            </section>
          </div>
        </div>
      </div>
    </div>
  );
};

export default Settings;
