// "home" 命名空间词条。zh 为真相源;en 缺项自动回退中文。
const dict = {
  zh: {
    welcomeSub: "上次游玩 {{ name }} · {{ rel }}",
    welcomeSubNever: "上次游玩 {{ name }} · 从未游玩",
    welcomeSubEmpty: "还没有实例 · 选个整合包开始吧",
    continueBadge: "CONTINUE",
    details: "详情",
    discoverHeading: "发现整合包",
  } as Record<string, string>,
  en: {
    welcomeSub: "Last played {{ name }} · {{ rel }}",
    welcomeSubNever: "{{ name }} · never played",
    welcomeSubEmpty: "No instances yet · pick a modpack to begin",
    continueBadge: "CONTINUE",
    details: "Details",
    discoverHeading: "Discover Modpacks",
  } as Record<string, string>,
};

export default dict;
