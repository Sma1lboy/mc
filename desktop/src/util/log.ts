/**
 * 前端(client)统一日志。
 *
 * 既照常打到浏览器 console(开发期 devtools 看),又把同一条转发到后端的统一日志文件
 * (`client_log` 命令,落进 `<data_dir>/logs/mc-launcher.log`,带 `client:` 前缀),这样
 * client 与本地数据层(daemon)的日志能在同一处对照排查。
 *
 * 转发是 best-effort:Tauri 不可用(浏览器预览)或命令失败都静默忽略,绝不反过来影响 UI。
 */
import { api } from "../ipc/api";

type Level = "error" | "warn" | "info" | "debug";

function forward(level: Level, args: unknown[]): void {
  // 已是字符串就直接用,否则尽量序列化(Error 取 stack)。
  const message = args
    .map((a) => {
      if (typeof a === "string") return a;
      if (a instanceof Error) return a.stack || `${a.name}: ${a.message}`;
      try {
        return JSON.stringify(a);
      } catch {
        return String(a);
      }
    })
    .join(" ");
  // 不 await:转发失败不能阻塞/抛出到调用方。
  void api.clientLog(level, message).catch(() => {});
}

export const log = {
  error: (...args: unknown[]) => {
    console.error(...args);
    forward("error", args);
  },
  warn: (...args: unknown[]) => {
    console.warn(...args);
    forward("warn", args);
  },
  info: (...args: unknown[]) => {
    console.info(...args);
    forward("info", args);
  },
  debug: (...args: unknown[]) => {
    console.debug(...args);
    forward("debug", args);
  },
};

let installed = false;

/**
 * 全局挂未捕获错误/Promise 拒绝的转发,并把 `console.error`/`console.warn` 也镜像进统一日志。
 * 在应用入口调用一次即可;重复调用为 no-op。
 */
export function installClientLogForwarding(): void {
  if (installed || typeof window === "undefined") return;
  installed = true;

  window.addEventListener("error", (e) => {
    forward("error", [`[window.onerror] ${e.message}`, e.error ?? ""]);
  });
  window.addEventListener("unhandledrejection", (e) => {
    forward("error", ["[unhandledrejection]", (e as PromiseRejectionEvent).reason ?? ""]);
  });

  // 镜像 console.error / console.warn(保留原始行为)。info/debug 不镜像,避免日志过吵。
  const origError = console.error.bind(console);
  const origWarn = console.warn.bind(console);
  console.error = (...args: unknown[]) => {
    origError(...args);
    forward("error", args);
  };
  console.warn = (...args: unknown[]) => {
    origWarn(...args);
    forward("warn", args);
  };
}
