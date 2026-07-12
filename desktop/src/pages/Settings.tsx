import { useEffect, useMemo, useState } from "react";
import {
  Select,
  ErrorState,
  Panel,
  Segmented,
  Slider,
  Tooltip,
  Heading,
  Toggle,
} from "../components";
import { Info } from "lucide-react";
import { api } from "../ipc/api";
import { useAsync } from "../util/useAsync";
import { useAppStore, setSocialEnabled } from "../store";
import { t, setLocale, useLang, type Locale } from "../i18n";
import type { GlobalSettings } from "../ipc/types";
import { AppearanceSection } from "./settings/AppearanceSection";
import { GameDirSection } from "./settings/GameDirSection";
import { JavaSection } from "./settings/JavaSection";
import { DiagnosticsSection } from "./settings/DiagnosticsSection";
import "./Settings.css";

/**
 * Settings —— 设置页骨架:语言 / 社交 / 下载与游戏 / 内容源在此,外观、Java、
 * 游戏目录、诊断各自成块(pages/settings/)。改动即时应用,失焦/操作后持久化。
 */
export default function Settings() {
  const locale = useLang();
  const socialEnabled = useAppStore((s) => s.socialEnabled);

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

  // 初始化:全局设置 + 系统内存(主题初始化在 AppearanceSection 内)。
  useEffect(() => {
    void loadSettings();
    void api.systemMemory().then((m) => setSysTotalMb(m.total_mb)).catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 改一项全局设置并立即持久化到后端。
  function patchSettings(patch: Partial<GlobalSettings>) {
    if (!settings) return;
    const next = { ...settings, ...patch };
    setSettings(next);
    void api.setSettings(next).catch(() => {});
  }

  const javas = useAsync(() => api.detectJava(), []);

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

            <AppearanceSection />
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

            <JavaSection javas={javas} />

            <GameDirSection settings={settings} setSettings={setSettings} />

            <DiagnosticsSection />
          </div>
        </div>
      </div>
    </div>
  );
}
