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
import lobby from "./lobby";
import friend from "./friend";
import notification from "./notification";
import link from "./link";
import skin from "./skin";
import tags from "./tags";
import shortcuts from "./shortcuts";
import crash from "./crash";
import agent from "./agent";

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
    lobby: lobby.zh,
    friend: friend.zh,
    notification: notification.zh,
    link: link.zh,
    skin: skin.zh,
    tags: tags.zh,
    shortcuts: shortcuts.zh,
    crash: crash.zh,
    agent: agent.zh,
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
    lobby: lobby.en,
    friend: friend.en,
    notification: notification.en,
    link: link.en,
    skin: skin.en,
    tags: tags.en,
    shortcuts: shortcuts.en,
    crash: crash.en,
    agent: agent.en,
  },
};
