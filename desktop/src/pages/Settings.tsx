import { Component, createResource, createSignal, For, Show, onMount } from "solid-js";
import { Button, Spinner, toast } from "../components";
import { applyThemeColor, setMode, saveTheme, PRESETS } from "../theme/theme";
import { api } from "../ipc/api";
import type { ThemeConfig, ThemeMode } from "../ipc/types";
import "./Settings.css";

/**
 * Settings —— 主题(PCL 灵魂特性:HSL 三滑块实时换肤 + 预设 + 深浅切换)
 * 以及 Java 检测列表。改动即时应用,失焦/操作后持久化到后端。
 */
const Settings: Component = () => {
  const [mode, setModeSig] = createSignal<ThemeMode>("dark");
  const [hue, setHue] = createSignal(150);
  const [sat, setSat] = createSignal(60);
  const [light, setLight] = createSignal(45);

  // 初始化:从后端取当前主题填充滑块。
  onMount(async () => {
    try {
      const cfg = await api.getTheme();
      setModeSig(cfg.mode);
      setHue(cfg.hue);
      setSat(cfg.saturation);
      setLight(cfg.lightness);
    } catch {
      /* 用默认值 */
    }
  });

  const [javas] = createResource(() => api.detectJava());

  function current(): ThemeConfig {
    return { mode: mode(), hue: hue(), saturation: sat(), lightness: light() };
  }

  // 实时应用 + 持久化。
  function apply(persist = true) {
    applyThemeColor(hue(), sat(), light());
    setMode(mode());
    if (persist) void saveTheme(current()).catch(() => {});
  }

  function pickPreset(p: { hue: number; saturation: number; lightness: number }) {
    setHue(p.hue);
    setSat(p.saturation);
    setLight(p.lightness);
    apply();
  }

  function toggleMode() {
    setModeSig((m) => (m === "dark" ? "light" : "dark"));
    apply();
    toast({ type: "info", message: `已切换到${mode() === "dark" ? "深色" : "浅色"}` });
  }

  return (
    <div class="settings">
      <h1>设置</h1>

      <section class="settings-section">
        <h2>外观</h2>

        <div class="settings-row">
          <span>主题模式</span>
          <Button variant="ghost" onClick={toggleMode}>
            {mode() === "dark" ? "🌙 深色" : "☀️ 浅色"}
          </Button>
        </div>

        <div class="settings-row">
          <span>预设强调色</span>
          <div class="preset-row">
            <For each={PRESETS}>
              {(p) => (
                <button
                  class="preset-swatch"
                  style={{ background: `hsl(${p.hue} ${p.saturation}% ${p.lightness}%)` }}
                  title={p.name}
                  onClick={() => pickPreset(p)}
                />
              )}
            </For>
          </div>
        </div>

        <div class="slider-row">
          <label>色相 {Math.round(hue())}</label>
          <input
            type="range"
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
        <div class="slider-row">
          <label>饱和度 {Math.round(sat())}</label>
          <input
            type="range"
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
        <div class="slider-row">
          <label>明度 {Math.round(light())}</label>
          <input
            type="range"
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

        <div class="accent-preview">
          <For each={[1, 2, 3, 4, 5, 6, 7, 8]}>
            {(i) => <span class="accent-chip" style={{ background: `var(--a-${i})` }} />}
          </For>
        </div>
      </section>

      <section class="settings-section">
        <h2>Java</h2>
        <Show when={!javas.loading} fallback={<Spinner />}>
          <Show
            when={(javas() ?? []).length > 0}
            fallback={<div class="settings-empty">未检测到 Java。</div>}
          >
            <div class="java-list">
              <For each={javas()}>
                {(j) => (
                  <div class="java-row">
                    <span class="java-ver">Java {j.version}</span>
                    <span class="java-meta">
                      {j.is_64bit ? "64-bit" : "32-bit"} · {j.source}
                    </span>
                    <span class="java-path">{j.path}</span>
                  </div>
                )}
              </For>
            </div>
          </Show>
        </Show>
      </section>
    </div>
  );
};

export default Settings;
