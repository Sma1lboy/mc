import { Component, createResource, createSignal, For, Show, onCleanup } from "solid-js";
import {
  InstanceRow,
  Button,
  Spinner,
  SearchBox,
  toast,
  type InstanceRowData,
} from "../components";
import { api, onInstallProgress } from "../ipc/api";
import { currentRoot } from "../store";
import type { InstanceSummary, ManifestVersion } from "../ipc/types";
import "./Library.css";

/**
 * Library —— 当前根目录的全部实例 + "安装新版本" 抽屉。
 * 安装进度通过 install://progress 事件实时显示在按钮旁。
 */

function toRowData(i: InstanceSummary): InstanceRowData {
  return {
    id: i.id,
    name: i.name || i.id,
    mc_version: i.mc_version,
    loader: i.loader,
    loader_version: i.loader_version || undefined,
    icon: i.icon || undefined,
    last_played: i.last_played ?? 0,
    running: i.running,
  };
}

const Library: Component = () => {
  const [instances, { refetch }] = createResource(
    () => currentRoot() ?? "",
    (root) => api.listInstances(root),
  );
  const [versions] = createResource(() => api.listVersions(false));

  const [showInstall, setShowInstall] = createSignal(false);
  const [filter, setFilter] = createSignal("");
  const [installing, setInstalling] = createSignal<string | null>(null);
  const [progress, setProgress] = createSignal("");

  const off = onInstallProgress((p) => {
    if (p.total > 0) setProgress(`${p.stage} ${p.current}/${p.total}`);
    else setProgress(p.stage);
  });
  onCleanup(off);

  const filtered = () => {
    const q = filter().toLowerCase();
    return (versions() ?? []).filter((v) => v.id.toLowerCase().includes(q)).slice(0, 60);
  };

  async function install(v: ManifestVersion) {
    setInstalling(v.id);
    setProgress("准备…");
    try {
      await api.installVersion(currentRoot() ?? "", v.id);
      toast({ type: "success", message: `已安装 ${v.id}` });
      setShowInstall(false);
      refetch();
    } catch (e) {
      toast({ type: "error", message: `安装失败:${e}` });
    } finally {
      setInstalling(null);
      setProgress("");
    }
  }

  return (
    <div class="library">
      <div class="library-head">
        <h1>库</h1>
        <Button variant="primary" onClick={() => setShowInstall((s) => !s)}>
          {showInstall() ? "关闭" : "安装新版本"}
        </Button>
      </div>

      <Show when={showInstall()}>
        <div class="install-panel">
          <SearchBox
            value={filter()}
            onInput={setFilter}
            placeholder="搜索版本号,如 1.20.1"
          />
          <Show when={installing()}>
            <div class="install-status">
              <Spinner /> 安装 {installing()} · {progress()}
            </div>
          </Show>
          <Show when={!versions.loading} fallback={<Spinner />}>
            <div class="version-list">
              <For each={filtered()}>
                {(v) => (
                  <div class="version-row">
                    <span class="version-id">{v.id}</span>
                    <span class="version-kind">{v.kind}</span>
                    <Button
                      variant="ghost"
                      disabled={!!installing()}
                      onClick={() => install(v)}
                    >
                      安装
                    </Button>
                  </div>
                )}
              </For>
            </div>
          </Show>
        </div>
      </Show>

      <Show when={!instances.loading} fallback={<div class="library-loading"><Spinner /></div>}>
        <Show
          when={(instances() ?? []).length > 0}
          fallback={<div class="library-empty">这个根目录还没有实例,点「安装新版本」开始。</div>}
        >
          <div class="library-rows">
            <For each={instances()}>
              {(inst) => (
                <InstanceRow
                  instance={toRowData(inst)}
                  onPlay={(id) =>
                    api
                      .launchInstance(currentRoot() ?? "", id, "Player", false)
                      .then(() => toast({ type: "info", message: `启动 ${id}` }))
                      .catch((e) => toast({ type: "error", message: `${e}` }))
                  }
                />
              )}
            </For>
          </div>
        </Show>
      </Show>
    </div>
  );
};

export default Library;
