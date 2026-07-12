// store/ —— 全局状态单一真相。此前是单文件 store.ts,按域拆分:
// state(zustand store 本体)/ app(导航·实例·更新·外观)/ launch(启动生命周期)
// / kobe(账号·presence)/ social(好友·通知缓存)。对外出口不变:from "../store"。
export * from "./state";
export * from "./app";
export * from "./launch";
export * from "./kobe";
export * from "./social";

import { useAppStore, set } from "./state";
import { refreshInstances } from "./app";
import { refreshFriends, refreshFriendRequests, refreshNotifications, refreshIdentities } from "./social";

// 应用级副作用:实例首拉、社交轮询、登录/登出社交刷新。仅真实 Tauri 环境。
if (typeof window !== "undefined") {
  // 实例列表首拉(切根由 setCurrentRoot 触发重拉)。
  void refreshInstances();

  // 单一连续轮询:仅在已登录 + 社交开启时刷新好友 + 请求 + 通知(30s 新鲜度)。
  setInterval(() => {
    void refreshFriends();
    void refreshFriendRequests();
    void refreshNotifications();
  }, 30_000);

  // kobeUser 变为已登录 → 拉一次初值;登出 → 清空社交信号。
  // subscribeWithSelector:只在 kobeUser 引用变化时触发(等价旧 createEffect)。
  useAppStore.subscribe(
    (s) => s.kobeUser,
    (user) => {
      if (user) {
        void refreshFriends();
        void refreshFriendRequests();
        void refreshNotifications();
        void refreshIdentities();
      } else {
        set({ friends: [], friendRequests: [], notifications: [], accountIdentities: [] });
      }
    },
  );
}
