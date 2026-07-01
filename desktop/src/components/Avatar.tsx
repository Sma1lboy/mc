import { useEffect, useState } from "react";

/**
 * 玩家头像。正版(microsoft)账号按 uuid 去 Crafatar 取皮肤头像(带帽层);离线 /
 * 外置登录 / 取不到 一律回退到内嵌的默认 **Steve** 头像(不依赖网络)。
 *
 * 始终渲染成填满父容器的 `<img>`(`.mc-avatar-img`),所以可直接塞进现有的头像方框
 * (`.rail-avatar` / `.account-avatar`)里替换原来的首字母占位。
 */

/** 内嵌默认 Steve 头像(64×64,来自 mc-heads 的 MHF_Steve)——网络兜底。 */
export const STEVE_AVATAR =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAEAAAABACAYAAACqaXHeAAAACXBIWXMAAA7EAAAOxAGVKw4bAAABo0lEQVR4nO2bLUsEURhGXbnuuLo6GhSLTYPF4AeY3DUaTCKI1WIX/BGCXbFbFoNJMMkWsYhsETSaBced/V7QanoucpWDzHvqM3fm8PDCvczs5pbnJj8HAijmXcjyYNJuX+ZtTz74mzL/ESuAFqCxAmgBGiuAFqBxSdqSF8TFgsx9+3DoOeGv75/5CbACaAEaK4AWoLECaAEaF+WH5AUHpTWZz0xP6QeMjMm836zLvNXpyTxJ3mV+Vr2XeeYnwAqgBWisAFqAxgqgBWhy5/ub8rtAHE/IGxQifY7w4dvnQ/GdEzI/AVYALUBjBdACNFYALUDjfPv8zsmFzLfXj2W+u/ooc98+ff1clvll9UjmlcM9mWd+AqwAWoDGCqAFaKwAWoAm+H2Aj9uHp6D15aWFoPX2PsCDFUAL0FgBtACNFUAL0LhGsykvSHsNmd/c1WS+ODv/Y6nvnFauZL61sSLzj4b+7pD5CbACaAEaK4AWoLECaAEa5yL9t8G3ut5HO+2uzGuvL0Hro+G8zH37/Pio/v1C5ifACqAFaKwAWoDGCqAFaL4AmjtXLE9M0u0AAAAASUVORK5CYII=";

/** 头像 URL(mc-heads.net,带帽层渲染;未知 uuid 服务端也会回退 Steve)。
 * 用 mc-heads 而非 crafatar:后者在本环境会 521(Cloudflare),前者稳定。 */
export function headUrl(uuid: string | undefined): string {
  const id = (uuid ?? "").replace(/-/g, "");
  return id ? `https://mc-heads.net/avatar/${id}/128` : STEVE_AVATAR;
}

/** 全身皮肤渲染(mc-heads.net 正面 2D 全身);uuid 缺失/离线时回退默认 Steve 皮肤。 */
export function skinBodyUrl(uuid: string | undefined): string {
  const id = (uuid ?? "").replace(/-/g, "");
  return `https://mc-heads.net/body/${id || "MHF_Steve"}/90`;
}

export function Avatar(props: {
  uuid?: string;
  /** 账号类型;仅 "microsoft" 会去取真实皮肤,其余用 Steve。 */
  kind?: string;
  className?: string;
  title?: string;
}): React.ReactElement {
  const [failed, setFailed] = useState(false);
  // 账号变了就重置失败态,避免一次取失败后对新账号也只显示 Steve。
  useEffect(() => {
    setFailed(false);
  }, [props.uuid, props.kind]);

  const online = props.kind === "microsoft" && !!props.uuid;
  const src = online && !failed ? headUrl(props.uuid) : STEVE_AVATAR;

  return (
    <img
      className={`mc-avatar-img shadow-raised rounded-none${props.className ? ` ${props.className}` : ""}`}
      src={src}
      alt=""
      width="64"
      height="64"
      title={props.title}
      draggable={false}
      onError={() => setFailed(true)}
    />
  );
}
