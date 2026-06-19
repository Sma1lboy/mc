import {
  Component,
  createEffect,
  createResource,
  createSignal,
  For,
  Show,
  onCleanup,
} from "solid-js";
import {
  InstanceRow,
  Button,
  Spinner,
  SearchBox,
  toast,
  type InstanceRowData,
} from "../components";
import { open, save } from "@tauri-apps/plugin-dialog";
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
  const [showPackPanel, setShowPackPanel] = createSignal(false);
  const [filter, setFilter] = createSignal("");
  const [installing, setInstalling] = createSignal<string | null>(null);
  const [progress, setProgress] = createSignal("");
  const [selectedInstanceId, setSelectedInstanceId] = createSignal("");
  const [importing, setImporting] = createSignal(false);
  const [exporting, setExporting] = createSignal(false);

  const off = onInstallProgress((p) => {
    if (p.total > 0) setProgress(`${p.stage} ${p.current}/${p.total}`);
    else setProgress(p.stage);
  });
  onCleanup(off);

  const filtered = () => {
    const q = filter().toLowerCase();
    return (versions() ?? []).filter((v) => v.id.toLowerCase().includes(q)).slice(0, 60);
  };

  createEffect(() => {
    const list = instances() ?? [];
    if (!selectedInstanceId() && list.length > 0) {
      setSelectedInstanceId(list[0].id);
    }
  });

  const selectedInstance = () =>
    (instances() ?? []).find((inst) => inst.id === selectedInstanceId()) ?? null;

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

  function fileStem(path: string) {
    const name = path.split(/[\\/]/).pop() ?? "imported-pack";
    return name.replace(/\.mrpack$/i, "");
  }

  function safeInstanceId(input: string) {
    const cleaned = input
      .trim()
      .replace(/[\\/:*?"<>|]+/g, "-")
      .replace(/\s+/g, "-")
      .replace(/^-+|-+$/g, "");
    return cleaned || "imported-pack";
  }

  async function exportSelectedMrpack() {
    const target = selectedInstance();
    if (!target) {
      toast({ type: "warn", message: "先选择一个实例" });
      return;
    }

    const defaultName = `${safeInstanceId(target.name || target.id)}.mrpack`;
    const dest = await save({
      title: "导出整合包",
      defaultPath: defaultName,
      filters: [{ name: "Modrinth pack", extensions: ["mrpack"] }],
    });
    if (!dest) return;

    setExporting(true);
    try {
      const path = await api.exportMrpack(currentRoot() ?? "", target.id, dest);
      toast({ type: "success", message: `已导出 ${path}` });
    } catch (e) {
      toast({ type: "error", message: `导出失败:${e}` });
    } finally {
      setExporting(false);
    }
  }

  async function importMrpack() {
    const picked = await open({
      title: "导入整合包",
      multiple: false,
      filters: [{ name: "Modrinth pack", extensions: ["mrpack"] }],
    });
    if (!picked || Array.isArray(picked)) return;

    const suggested = safeInstanceId(fileStem(picked));
    const entered = window.prompt("实例 ID", suggested);
    if (entered === null) return;
    const instanceId = safeInstanceId(entered);

    setImporting(true);
    try {
      const id = await api.importMrpack(currentRoot() ?? "", picked, instanceId);
      toast({ type: "success", message: `已导入 ${id}` });
      refetch();
    } catch (e) {
      toast({ type: "error", message: `导入失败:${e}` });
    } finally {
      setImporting(false);
    }
  }

  return (
    <div class="library">
      <div class="library-head">
        <h1>库</h1>
        <div class="library-actions">
          <Button
            variant={showPackPanel() ? "primary" : "ghost"}
            onClick={() => {
              setShowPackPanel((s) => !s);
              setShowInstall(false);
            }}
          >
            整合包
          </Button>
          <Button
            variant={showInstall() ? "ghost" : "primary"}
            onClick={() => {
              setShowInstall((s) => !s);
              setShowPackPanel(false);
            }}
          >
            {showInstall() ? "关闭" : "安装新版本"}
          </Button>
        </div>
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

      <Show when={showPackPanel()}>
        <div class="install-panel mrpack-panel">
          <div class="mrpack-row">
            <label class="mrpack-label" for="mrpack-instance">
              实例
            </label>
            <select
              id="mrpack-instance"
              class="mrpack-select"
              value={selectedInstanceId()}
              disabled={importing() || exporting()}
              onChange={(e) => setSelectedInstanceId(e.currentTarget.value)}
            >
              <For each={instances() ?? []}>
                {(inst) => (
                  <option value={inst.id}>
                    {inst.name || inst.id} · {inst.mc_version} · {inst.loader}
                  </option>
                )}
              </For>
            </select>
          </div>
          <div class="mrpack-actions">
            <Button
              variant="primary"
              disabled={importing() || exporting() || !selectedInstance()}
              onClick={exportSelectedMrpack}
            >
              {exporting() ? "导出中" : "导出 .mrpack"}
            </Button>
            <Button
              variant="ghost"
              disabled={importing() || exporting()}
              onClick={importMrpack}
            >
              {importing() ? "导入中" : "导入 .mrpack"}
            </Button>
          </div>
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
