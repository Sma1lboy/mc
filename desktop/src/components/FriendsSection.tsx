// FriendsSection 已拆分为顶栏两个独立入口:
//   - 搜索加好友 + 好友列表(在线点 + 活动行)→ components/FriendsButton.tsx
//   - 收到的好友请求(接受/拒绝)→ components/NotificationCenter.tsx
// 文件保留为薄再导出,兼容旧引用;新代码请直接用 FriendsButton / NotificationCenter。
export { FriendsButton as FriendsSection } from "./FriendsButton";
