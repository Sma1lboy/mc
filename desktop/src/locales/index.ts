// 词条汇总:每个命名空间一个文件(并行迁移时互不冲突),这里组装成 zh / en 两棵字典。
import settings from "./settings";
import account from "./account";
import instance from "./instance";
import library from "./library";
import discover from "./discover";
import facets from "./facets";
import layout from "./layout";
import store from "./store";
import components from "./components";
import downloads from "./downloads";
import home from "./home";
import projectDetail from "./projectDetail";
import kobe from "./kobe";
import realm from "./realm";

export const dictionaries = {
  zh: {
    settings: settings.zh,
    account: account.zh,
    instance: instance.zh,
    library: library.zh,
    discover: discover.zh,
    facets: facets.zh,
    layout: layout.zh,
    store: store.zh,
    components: components.zh,
    downloads: downloads.zh,
    home: home.zh,
    projectDetail: projectDetail.zh,
    kobe: kobe.zh,
    realm: realm.zh,
  },
  en: {
    settings: settings.en,
    account: account.en,
    instance: instance.en,
    library: library.en,
    discover: discover.en,
    facets: facets.en,
    layout: layout.en,
    store: store.en,
    components: components.en,
    downloads: downloads.en,
    home: home.en,
    projectDetail: projectDetail.en,
    kobe: kobe.en,
    realm: realm.en,
  },
};
