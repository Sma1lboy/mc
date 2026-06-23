// "components" 命名空间词条。zh 为真相源;en 缺项自动回退中文。
const dict = {
  zh: {
    // Toast
    "toast.close": "关闭",

    // SearchBox
    "searchBox.label": "搜索",
    "searchBox.placeholder": "搜索…",
    "searchBox.clear": "清空搜索",

    // ErrorState
    "errorState.failed": "加载失败",
    "errorState.retry": "重试",

    // Select
    "select.placeholder": "请选择",

    // PlayButton / Spinner
    "play.start": "启动",
    "play.stop": "停止",
    "spinner.loading": "加载中",

    // Lightbox
    "lightbox.close": "关闭",
    "lightbox.prev": "上一张",
    "lightbox.next": "下一张",

    // BlockedFilesDialog
    "blocked.title": "需要手动下载的文件",
    "blocked.heading": "「{{ id }}」已安装,但有文件需手动下载",
    "blocked.body": "下列文件的作者在 CurseForge 上禁止了第三方下载。点「打开下载页」下载后,放进实例对应目录即可。",
    "blocked.required": "必需",
    "blocked.placeInto": "放进:{{ dir }}",
    "blocked.openPage": "打开下载页 ↗",
    "blocked.skipped": "已跳过的可选文件({{ count }})",
    "blocked.gotIt": "知道了",

    // NewInstanceDialog — loader options
    "newInstance.loaderVanilla": "原版 (Vanilla)",
    "newInstance.loaderFabric": "Fabric",
    "newInstance.loaderQuilt": "Quilt",
    "newInstance.loaderForge": "Forge",
    "newInstance.loaderNeoforge": "NeoForge",

    // NewInstanceDialog — fields & flow
    "newInstance.title": "新建实例",
    "newInstance.name": "名称",
    "newInstance.namePlaceholder": "例如 生存整合包…",
    "newInstance.mcVersion": "Minecraft 版本",
    "newInstance.loadingVersions": "加载版本中…",
    "newInstance.selectVersion": "选择版本",
    "newInstance.loader": "加载器",
    "newInstance.forgeVersion": "Forge 版本",
    "newInstance.neoforgeVersion": "NeoForge 版本",
    "newInstance.fabricVersionOptional": "Fabric 版本(可选)",
    "newInstance.quiltVersionOptional": "Quilt 版本(可选)",
    "newInstance.latestRecommended": "最新(推荐)",
    "newInstance.loadingAvailable": "加载可用版本中…",
    "newInstance.forgePlaceholder": "例如 47.2.0…",
    "newInstance.neoforgePlaceholder": "例如 20.4.237…",
    "newInstance.preparing": "准备…",
    "newInstance.created": "已创建实例「{{ name }}」",
    "newInstance.createFailed": "创建失败:{{ error }}",
    "newInstance.creating": "创建中…",
    "newInstance.cancel": "取消",
    "newInstance.create": "创建",
  } as Record<string, string>,
  en: {
    // Toast
    "toast.close": "Close",

    // SearchBox
    "searchBox.label": "Search",
    "searchBox.placeholder": "Search…",
    "searchBox.clear": "Clear search",

    // ErrorState
    "errorState.failed": "Failed to load",
    "errorState.retry": "Retry",

    // Select
    "select.placeholder": "Select…",

    // PlayButton / Spinner
    "play.start": "Launch",
    "play.stop": "Stop",
    "spinner.loading": "Loading",

    // Lightbox
    "lightbox.close": "Close",
    "lightbox.prev": "Previous",
    "lightbox.next": "Next",

    // BlockedFilesDialog
    "blocked.title": "Files that need manual download",
    "blocked.heading": "“{{ id }}” is installed, but some files need a manual download",
    "blocked.body": "The authors of the files below disabled third-party downloads on CurseForge. Click “Open download page”, then drop the file into the matching folder in this instance.",
    "blocked.required": "Required",
    "blocked.placeInto": "Place into: {{ dir }}",
    "blocked.openPage": "Open download page ↗",
    "blocked.skipped": "Skipped optional files ({{ count }})",
    "blocked.gotIt": "Got it",

    // NewInstanceDialog — loader options
    "newInstance.loaderVanilla": "Vanilla",
    "newInstance.loaderFabric": "Fabric",
    "newInstance.loaderQuilt": "Quilt",
    "newInstance.loaderForge": "Forge",
    "newInstance.loaderNeoforge": "NeoForge",

    // NewInstanceDialog — fields & flow
    "newInstance.title": "New instance",
    "newInstance.name": "Name",
    "newInstance.namePlaceholder": "e.g. Survival modpack…",
    "newInstance.mcVersion": "Minecraft version",
    "newInstance.loadingVersions": "Loading versions…",
    "newInstance.selectVersion": "Select a version",
    "newInstance.loader": "Loader",
    "newInstance.forgeVersion": "Forge version",
    "newInstance.neoforgeVersion": "NeoForge version",
    "newInstance.fabricVersionOptional": "Fabric version (optional)",
    "newInstance.quiltVersionOptional": "Quilt version (optional)",
    "newInstance.latestRecommended": "Latest (recommended)",
    "newInstance.loadingAvailable": "Loading available versions…",
    "newInstance.forgePlaceholder": "e.g. 47.2.0…",
    "newInstance.neoforgePlaceholder": "e.g. 20.4.237…",
    "newInstance.preparing": "Preparing…",
    "newInstance.created": "Created instance “{{ name }}”",
    "newInstance.createFailed": "Create failed: {{ error }}",
    "newInstance.creating": "Creating…",
    "newInstance.cancel": "Cancel",
    "newInstance.create": "Create",
  } as Record<string, string>,
};

export default dict;
