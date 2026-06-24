// "downloads" 命名空间词条(顶栏全局下载队列)。zh 为真相源;en 缺项自动回退中文。
const dict = {
  zh: {
    title: "下载队列",
    empty: "暂无下载任务",
    queued: "排队中…",
    installing: "安装中…",
    done: "已完成",
    failed: "下载失败",
    clearFinished: "清除已完成",
    dismiss: "移除",
  } as Record<string, string>,
  en: {
    title: "Downloads",
    empty: "No downloads yet",
    queued: "Queued…",
    installing: "Installing…",
    done: "Done",
    failed: "Download failed",
    clearFinished: "Clear finished",
    dismiss: "Dismiss",
  } as Record<string, string>,
};

export default dict;
