// "facets" 命名空间词条:Discover 的 Modrinth 多选过滤侧栏。
// 分组标题 / 开关 / 清除等界面文案走 i18n;分类名是平台动态数据,原样展示,不在此。
// zh 为真相源;en 缺项自动回退中文。
const dict = {
  zh: {
    title: "筛选",
    clear: "清除筛选",
    loadFailed: "筛选项加载失败",
    cfNoFacets: "CurseForge 暂不支持这些筛选项,搜索按关键词进行。",

    headerCategories: "分类",
    headerFeatures: "特性",
    headerResolutions: "分辨率",
    headerPerformance: "性能影响",
    headerEnvironment: "运行环境",
    headerLoader: "加载器",
    headerGameVersion: "游戏版本",
    headerLicense: "许可证",

    envClient: "客户端",
    envServer: "服务端",

    showAllVersions: "显示全部版本(含快照)",
    versionSearchPlaceholder: "搜索版本…",
    openSource: "开源",
  } as Record<string, string>,
  en: {
    title: "Filters",
    clear: "Clear filters",
    loadFailed: "Failed to load filters",
    cfNoFacets: "CurseForge doesn't support these filters yet — search runs by keyword.",

    headerCategories: "Categories",
    headerFeatures: "Features",
    headerResolutions: "Resolutions",
    headerPerformance: "Performance impact",
    headerEnvironment: "Environment",
    headerLoader: "Loaders",
    headerGameVersion: "Game versions",
    headerLicense: "License",

    envClient: "Client",
    envServer: "Server",

    showAllVersions: "Show all versions (incl. snapshots)",
    versionSearchPlaceholder: "Search versions…",
    openSource: "Open source",
  } as Record<string, string>,
};

export default dict;
