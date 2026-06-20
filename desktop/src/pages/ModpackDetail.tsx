import { Component, createResource, createSignal, For, Show } from "solid-js";
import { Spinner, toast, type ModpackHit } from "../components";
import { api } from "../ipc/api";
import { currentRoot } from "../store";
import type { ModrinthVersion } from "../ipc/types";
import "./ModpackDetail.css";

/**
 * ModpackDetail —— 整合包详情页(照 Modrinth 项目页):头部信息 + 画廊 + 版本列表,
 * 每个版本带类型/MC/loader/发布时间/下载数 + 可展开的更新日志 + 「安装此版本」。
 * 点击卡片进入此页,而非直接下载。
 */

const typeLabel = (t: string) =>
  (({ release: "正式版", beta: "测试版", alpha: "内测版" }) as Record<string, string>)[t] ?? t;

const loaderLabel = (l: string) =>
  (({ fabric: "Fabric", forge: "Forge", neoforge: "NeoForge", quilt: "Quilt" }) as Record<
    string,
    string
  >)[l] ?? l;

const fmtSize = (n: number | null) =>
  !n ? "" : n >= 1 << 20 ? `${(n / (1 << 20)).toFixed(1)} MB` : `${Math.ceil(n / 1024)} KB`;

const fmtDate = (s: string) => {
  const d = new Date(s);
  return isNaN(d.getTime()) ? s : d.toLocaleDateString();
};

const ModpackDetail: Component<{
  hit: ModpackHit;
  onBack: () => void;
  onInstalled?: () => void;
}> = (props) => {
  const [versions] = createResource(
    () => props.hit.id,
    (id) =>
      api.modrinthVersions(id).catch((e) => {
        toast({ type: "error", message: `版本列表加载失败:${e}` });
        return [] as ModrinthVersion[];
      }),
  );
  const [openLog, setOpenLog] = createSignal<Record<string, boolean>>({});
  const [installing, setInstalling] = createSignal<string | null>(null);

  async function install(v: ModrinthVersion) {
    if (installing()) return;
    if (!v.mrpack_url) {
      toast({ type: "error", message: "该版本没有可安装的 .mrpack 文件" });
      return;
    }
    setInstalling(v.id);
    toast({
      type: "info",
      message: `开始安装「${props.hit.title} ${v.version_number}」…首次会下载原版与依赖,可能需要几分钟`,
    });
    try {
      const out = await api.installModpackUrl(currentRoot() ?? "", v.mrpack_url, null);
      const blocked = out.blocked.length;
      toast({
        type: blocked > 0 ? "info" : "success",
        message:
          blocked > 0
            ? `已安装「${out.instance_id}」(${blocked} 个文件需手动下载),去启动页选择它`
            : `已安装「${out.instance_id}」,去启动页选择它即可开玩`,
      });
      props.onInstalled?.();
    } catch (e) {
      toast({ type: "error", message: `安装失败:${e}` });
    } finally {
      setInstalling(null);
    }
  }

  return (
    <div class="mpd">
      <button class="mpd-back" onClick={props.onBack}>
        ← 返回
      </button>

      <div class="mpd-header">
        <Show when={props.hit.gallery_url}>
          <img class="mpd-banner" src={props.hit.gallery_url} alt="" />
        </Show>
        <div class="mpd-head-row">
          <Show
            when={props.hit.icon_url}
            fallback={<div class="mpd-icon mpd-icon-ph">{(props.hit.title[0] ?? "?").toUpperCase()}</div>}
          >
            <img class="mpd-icon" src={props.hit.icon_url} alt="" />
          </Show>
          <div class="mpd-head-meta">
            <h1 class="mpd-title">{props.hit.title}</h1>
            <div class="mpd-sub">
              by {props.hit.author} · ⬇ {props.hit.downloads.toLocaleString()}
            </div>
            <div class="mpd-cats">
              <For each={props.hit.categories}>{(c) => <span class="mpd-cat">{c}</span>}</For>
            </div>
          </div>
        </div>
        <Show when={props.hit.description}>
          <p class="mpd-desc">{props.hit.description}</p>
        </Show>
      </div>

      <div class="mpd-versions">
        <div class="mpd-vh">版本</div>
        <Show when={!versions.loading} fallback={<div class="mpd-loading"><Spinner /></div>}>
          <Show
            when={(versions() ?? []).length > 0}
            fallback={<div class="mpd-empty">没有可用版本</div>}
          >
            <For each={versions()}>
              {(v) => (
                <div class="mpd-vrow">
                  <div class="mpd-vmain">
                    <div class="mpd-vtop">
                      <span class="mpd-vnum">{v.version_number}</span>
                      <span class="mpd-vtype" data-type={v.version_type}>
                        {typeLabel(v.version_type)}
                      </span>
                    </div>
                    <div class="mpd-vmeta">
                      {v.game_versions.slice(0, 5).join(", ")}
                      <Show when={v.loaders.length}>
                        {" · "}
                        {v.loaders.map(loaderLabel).join(" / ")}
                      </Show>
                      {" · "}
                      {fmtDate(v.date_published)} · ⬇ {v.downloads.toLocaleString()}
                      <Show when={v.file_size}>{" · " + fmtSize(v.file_size)}</Show>
                    </div>
                    <Show when={v.changelog?.trim()}>
                      <button
                        class="mpd-cl-toggle"
                        onClick={() => setOpenLog((o) => ({ ...o, [v.id]: !o[v.id] }))}
                      >
                        {openLog()[v.id] ? "收起更新日志" : "更新日志"}
                      </button>
                      <Show when={openLog()[v.id]}>
                        <pre class="mpd-cl">{v.changelog}</pre>
                      </Show>
                    </Show>
                  </div>
                  <button
                    class="mpd-install"
                    disabled={!v.mrpack_url || installing() !== null}
                    onClick={() => install(v)}
                  >
                    {installing() === v.id ? "安装中…" : "安装此版本"}
                  </button>
                </div>
              )}
            </For>
          </Show>
        </Show>
      </div>
    </div>
  );
};

export default ModpackDetail;
