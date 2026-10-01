// "downloads" 命名空间词条(顶栏全局下载队列)。zh 为真相源;en 缺项自动回退中文。
const dict = {
  zh: {
    title: "下载与安装",
    empty: "暂无任务。可在「发现」中添加整合包。",
    queued: "排队中…",
    installing: "安装中…",
    done: "任务已完成",
    doneHint: "如有缺失文件提示,请先完成手动下载。",
    failed: "安装未完成",
    retryHint: "请返回发起安装的页面重试。",
    errorDetails: "查看错误详情",
    recordsOnly: "清除记录不会删除已安装文件。",
    clearFinished: "清除已结束记录",
    dismiss: "移除「{{ title }}」的记录",
  } as Record<string, string>,
  en: {
    title: "Downloads and installs",
    empty: "No tasks yet. Add a modpack from Discover.",
    queued: "Queued…",
    installing: "Installing…",
    done: "Task complete",
    doneHint: "If files are reported missing, finish the manual downloads first.",
    failed: "Installation incomplete",
    retryHint: "Return to the page where you started the install and try again.",
    errorDetails: "View error details",
    recordsOnly: "Clearing records does not delete installed files.",
    clearFinished: "Clear ended tasks",
    dismiss: "Dismiss record for {{ title }}",
  } as Record<string, string>,
};

export default dict;
