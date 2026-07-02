import { useEffect, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";

/**
 * 监听窗口原生文件拖放(Tauri 启用了原生拖放,HTML5 ondrop 不触发),识别整合包文件
 * (`.mrpack` / `.zip`)。返回 `dragOver` 状态供页面渲染高亮遮罩。
 *
 * `enabled` 为 false 时不响应(如导入弹窗已打开、由它自己接管拖放),避免同一次 drop 被
 * 多个监听器重复处理。回调用 ref 存,使订阅只装一次(StrictMode 下挂载→清理→再挂载幂等),
 * 不因 onFile / enabled 变化反复重订阅。
 */
export function useModpackDrop(opts: {
  enabled: boolean;
  onFile: (path: string) => void;
  onUnsupported?: () => void;
}): boolean {
  const [dragOver, setDragOver] = useState(false);
  const optsRef = useRef(opts);
  optsRef.current = opts;

  useEffect(() => {
    const unlisten = getCurrentWebview().onDragDropEvent((e) => {
      const o = optsRef.current;
      if (!o.enabled) {
        setDragOver(false);
        return;
      }
      const p = e.payload;
      if (p.type === "enter" || p.type === "over") setDragOver(true);
      else if (p.type === "leave") setDragOver(false);
      else if (p.type === "drop") {
        setDragOver(false);
        const file = p.paths.find((x) => /\.(mrpack|zip)$/i.test(x));
        if (file) o.onFile(file);
        else o.onUnsupported?.();
      }
    });
    return () => void unlisten.then((fn) => fn());
  }, []);

  return dragOver;
}
