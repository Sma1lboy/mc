// 词条汇总:每个命名空间一个文件(并行迁移时互不冲突),这里组装成 zh / en 两棵字典。
import settings from "./settings";
import account from "./account";
import instance from "./instance";
import library from "./library";
import discover from "./discover";
import classic from "./classic";
import layout from "./layout";
import store from "./store";
import components from "./components";

export const dictionaries = {
  zh: {
    settings: settings.zh,
    account: account.zh,
    instance: instance.zh,
    library: library.zh,
    discover: discover.zh,
    classic: classic.zh,
    layout: layout.zh,
    store: store.zh,
    components: components.zh,
  },
  en: {
    settings: settings.en,
    account: account.en,
    instance: instance.en,
    library: library.en,
    discover: discover.en,
    classic: classic.en,
    layout: layout.en,
    store: store.en,
    components: components.en,
  },
};
