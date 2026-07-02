import { create } from "zustand";
import { flatten, translator, resolveTemplate } from "@solid-primitives/i18n";
import { dictionaries } from "./locales";

/**
 * 国际化(中文 / English)。词条与词典结构不变(仍用 @solid-primitives/i18n 的
 * flatten/translator/resolveTemplate —— 这几个是纯函数,与框架无关,直接复用)。
 * 只把「当前语言」的响应式载体从 Solid 信号换成一个独立的 zustand 小 store。
 *
 * 独立于主 store(store.ts):store.ts 顶部 `import { t } from "./i18n"`,若 i18n 反向
 * 依赖 store 会成循环。故此处自持一个极小 store,不引用 ./store。
 *
 * 用法不变:
 *   - 取词条:`t("instance.launch")`,带插值 `t("lib.installed", { id })`。
 *   - 组件里要「切语言即时重渲染」→ 在组件顶部调一次 `useLang()` 订阅当前语言
 *     (App 顶层已调一次,覆盖未 memo 的整棵子树;被 React.memo 隔离的子树需各自调)。
 *   - 非组件代码 → 直接 `t(...)`,读当前语言快照即可。
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

/** 当前语言的响应式载体(独立 store)。组件订阅走 useLang();快照走 locale()。 */
const useLocaleStore = create<{ locale: Locale }>(() => ({ locale: readInitialLocale() }));

/** 当前语言(快照;组件里想响应切换请用 useLang())。 */
export const locale = (): Locale => useLocaleStore.getState().locale;

/** 订阅当前语言:组件顶部调一次,切语言即重渲染。返回当前 Locale。 */
export const useLang = (): Locale => useLocaleStore((s) => s.locale);

/** 切换界面语言并持久化;同步 <html lang>(断词 / 无障碍)。 */
export function setLocale(next: Locale): void {
  useLocaleStore.setState({ locale: next });
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(STORAGE_KEY, next);
  } catch {
    /* localStorage 在加固的 WebView 里可能不可用 */
  }
  document.documentElement.lang = next === "en" ? "en" : "zh-CN";
}

// 扁平化为点分键。English 以 zh 为底再覆盖,缺失项自动回退中文。
const flatZh = flatten(dictionaries.zh) as unknown as Record<string, string>;
const flatEn: Record<string, string> = {
  ...flatZh,
  ...(flatten(dictionaries.en) as unknown as Record<string, string>),
};

const translateZh = translator(() => flatZh, resolveTemplate);
const translateEn = translator(() => flatEn, resolveTemplate);

/** 取词条;缺失时回退到键名本身(便于发现漏翻)。 */
export function t(key: string, params?: Record<string, string | number>): string {
  const translate = useLocaleStore.getState().locale === "en" ? translateEn : translateZh;
  return translate(key, params as never) ?? key;
}

// 启动即把当前语言写进 <html lang>。
if (typeof window !== "undefined") {
  document.documentElement.lang = locale() === "en" ? "en" : "zh-CN";
}
