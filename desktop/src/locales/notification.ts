// "notification" 命名空间词条:顶栏通知中心(好友请求,后续可扩展领域邀请等)。
// zh 为真相源;en 缺项自动回退中文。
const dict = {
  zh: {
    title: "通知",
    empty: "暂无通知",
    friendRequest: "{{ name }} 请求加你为好友",
    friendAccepted: "{{ name }} 接受了你的好友请求",
    realmInvite: "{{ name }} 邀请你加入领域「{{ realm }}」",
  } as Record<string, string>,
  en: {
    title: "Notifications",
    empty: "No notifications",
    friendRequest: "{{ name }} wants to add you as a friend",
    friendAccepted: "{{ name }} accepted your friend request",
    realmInvite: "{{ name }} invited you to the realm “{{ realm }}”",
  } as Record<string, string>,
};

export default dict;
