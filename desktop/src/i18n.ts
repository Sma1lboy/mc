import { createMemo, createSignal } from "solid-js";
import { flatten, translator, resolveTemplate } from "@solid-primitives/i18n";
import { dictionaries } from "./locales";

/**
 * 国际化(中文 / English),基于 @solid-primitives/i18n。默认中文。
 *
 * 词条按命名空间分文件放在 `src/locales/<ns>.ts`(每个导出 `{ zh, en }`),在
 * `src/locales/index.ts` 汇总。用法:`t("instance.launch")`,带插值 `t("lib.installed", { id })`
 * 对应词条 `"已安装 {{ id }}"`。`t` 读响应式 `locale` 信号,切语言即时重渲染。
 *
 * English 缺词条时回退中文(zh 为真相源),便于渐进补全。
 */
export type Locale = "zh" | "en";

const STORAGE_KEY = "mc-launcher.locale";

function readInitialLocale(): Locale {
  if (typeof window === "undefined") return "zh";
  try {
    return window.localStorage.getItem(STORAGE_KEY) === "en" ? "en" : "zh";
  } catch {
    return "zh";
  }
}

const [locale, setLocaleSig] = createSignal<Locale>(readInitialLocale());
export { locale };

/** 切换界面语言并持久化;同步 <html lang>(断词 / 无障碍)。 */
export function setLocale(next: Locale): void {
  setLocaleSig(next);
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(STORAGE_KEY, next);
  } catch {
    /* localStorage 在加固的 WebView 里可能不可用 */
  }
  document.documentElement.lang = next === "en" ? "en" : "zh-CN";
}

// 扁平化为点分键。English 以 zh 为底再覆盖,缺失项自动回退中文。
// 命名空间切片是 Record<string,string>(允许迁移时任意键),flatten 在运行时会逐层展开,
// 但静态类型看不穿这层 Record,故在此收敛成扁平的 Record<string,string>。
const flatZh = flatten(dictionaries.zh) as unknown as Record<string, string>;
const flatEn: Record<string, string> = {
  ...flatZh,
  ...(flatten(dictionaries.en) as unknown as Record<string, string>),
};
const activeDict = createMemo<Record<string, string>>(() => (locale() === "en" ? flatEn : flatZh));

const translate = translator(activeDict, resolveTemplate);

/** 取词条;缺失时回退到键名本身(便于发现漏翻)。 */
export function t(key: string, params?: Record<string, string | number>): string {
  return translate(key, params as never) ?? key;
}

// 启动即把当前语言写进 <html lang>。
if (typeof window !== "undefined") {
  document.documentElement.lang = locale() === "en" ? "en" : "zh-CN";
}
