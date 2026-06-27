// 头像方块瓦片:由 key(用户名 / id)哈希定一个方块色调 + 取名字首字。
// 好友 popover、领域成员 / 邀请共用,保证人员行的视觉语言一致(方块工坊风)。

const AVATAR_TONES = ["#6f9b4e", "#8a6f3f", "#5b7f86", "#7a5b3a", "#5e5b6e"];

/** 由稳定 key(用户名 / id)派生一个方块色调。 */
export function avatarTone(key: string): string {
  let h = 0;
  for (let i = 0; i < key.length; i++) h = (h * 31 + key.charCodeAt(i)) >>> 0;
  return AVATAR_TONES[h % AVATAR_TONES.length];
}

/** 名字首字(大写),空时回落 "?"。 */
export function avatarInitial(name: string): string {
  return (name.trim().slice(0, 1) || "?").toUpperCase();
}
