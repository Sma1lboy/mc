import { Component, For, Show, onMount, onCleanup } from "solid-js";
import "./Lightbox.css";

/** 灯箱里一张可展示的图片(标题/描述可选)。 */
export interface LightboxImage {
  url: string;
  title?: string | null;
  description?: string | null;
}

/**
 * Lightbox —— 全屏在应用内查看图片的「灯箱」。受控:由父组件持有当前 index。
 * 支持上一张/下一张(按钮 + 键盘 ← →)、Esc / 点击遮罩关闭、底部缩略图条快速跳转、
 * 右上计数。专为整合包详情页的「画廊」标签页设计,但完全通用。
 */
const Lightbox: Component<{
  images: LightboxImage[];
  index: number;
  onIndex: (i: number) => void;
  onClose: () => void;
}> = (props) => {
  const count = () => props.images.length;
  const cur = () => props.images[props.index];
  const go = (i: number) => props.onIndex((i + count()) % count());
  const prev = () => go(props.index - 1);
  const next = () => go(props.index + 1);

  // 键盘导航:← → 切换,Esc 关闭。挂载时绑定,卸载时解绑。
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") props.onClose();
    else if (e.key === "ArrowLeft") prev();
    else if (e.key === "ArrowRight") next();
  };
  onMount(() => window.addEventListener("keydown", onKey));
  onCleanup(() => window.removeEventListener("keydown", onKey));

  return (
    // 遮罩:fixed 全屏、最高层、居中、半透明深底 + 背景模糊。lb-mask 残留类仅承载淡入动画。
    <div
      class="lb-mask fixed inset-0 z-[200] flex items-center justify-center pt-[56px] pr-[72px] pb-[120px] pl-[72px] bg-[rgba(8,10,14,0.86)] backdrop-blur-[6px]"
      onClick={props.onClose}
    >
      {/* 关闭按钮:右上圆形,半透明白底,hover 加亮。 */}
      <button
        class="fixed top-[16px] right-[18px] w-[38px] h-[38px] border-none rounded-full bg-[rgba(255,255,255,0.12)] text-white text-[17px] cursor-pointer transition-[background] duration-150 ease-[ease] hover:bg-[rgba(255,255,255,0.26)]"
        onClick={props.onClose}
        aria-label="关闭"
      >
        ✕
      </button>

      <Show when={count() > 1}>
        {/* 右上计数(左上角):等宽数字。 */}
        <div class="fixed top-[22px] left-[22px] text-[rgba(255,255,255,0.8)] text-[13px] [font-variant-numeric:tabular-nums]">
          {props.index + 1} / {count()}
        </div>
        <button
          class="fixed top-1/2 -translate-y-1/2 w-[46px] h-[46px] border-none rounded-full bg-[rgba(255,255,255,0.12)] text-white text-[26px] leading-none cursor-pointer transition-[background,transform] duration-150 ease-[ease] hover:bg-[rgba(255,255,255,0.26)] active:scale-[0.92] left-[14px]"
          aria-label="上一张"
          onClick={(e) => {
            e.stopPropagation();
            prev();
          }}
        >
          ‹
        </button>
        <button
          class="fixed top-1/2 -translate-y-1/2 w-[46px] h-[46px] border-none rounded-full bg-[rgba(255,255,255,0.12)] text-white text-[26px] leading-none cursor-pointer transition-[background,transform] duration-150 ease-[ease] hover:bg-[rgba(255,255,255,0.26)] active:scale-[0.92] right-[14px]"
          aria-label="下一张"
          onClick={(e) => {
            e.stopPropagation();
            next();
          }}
        >
          ›
        </button>
      </Show>

      {/* 主舞台:纵向布局图片 + 标题描述。 */}
      <figure
        class="m-0 max-w-full max-h-full flex flex-col items-center gap-[12px]"
        onClick={(e) => e.stopPropagation()}
      >
        {/* 当前图:contain 适配,圆角 + 深阴影。lb-img 残留类仅承载弹入动画。 */}
        <img
          class="lb-img max-w-full max-h-[calc(100vh-200px)] object-contain rounded-card"
          src={cur()?.url}
          alt={cur()?.title ?? ""}
        />
        <Show when={cur()?.title || cur()?.description}>
          <figcaption class="max-w-[720px] text-center flex flex-col gap-[3px]">
            <Show when={cur()?.title}>
              <span class="text-white text-[14px] font-semibold">
                {cur()!.title}
              </span>
            </Show>
            <Show when={cur()?.description}>
              <span class="text-[rgba(255,255,255,0.66)] text-[12px] leading-[1.5]">
                {cur()!.description}
              </span>
            </Show>
          </figcaption>
        </Show>
      </figure>

      <Show when={count() > 1}>
        {/* 底部缩略图条:横向滚动。 */}
        <div
          class="fixed bottom-[14px] left-1/2 -translate-x-1/2 max-w-[calc(100vw-32px)] flex gap-[8px] py-[8px] px-[10px] overflow-x-auto bg-[rgba(255,255,255,0.06)] rounded-[10px]"
          onClick={(e) => e.stopPropagation()}
        >
          <For each={props.images}>
            {(img, i) => (
              <img
                class="w-[84px] h-[48px] flex-[0_0_auto] object-cover rounded-[5px] cursor-pointer border-2 border-solid border-transparent transition-[opacity,border-color] duration-150 ease-[ease] hover:opacity-[0.85]"
                classList={{
                  "opacity-100 !border-white": i() === props.index,
                  "opacity-50": i() !== props.index,
                }}
                src={img.url}
                alt={img.title ?? ""}
                loading="lazy"
                onClick={() => props.onIndex(i())}
              />
            )}
          </For>
        </div>
      </Show>
    </div>
  );
};

export default Lightbox;
