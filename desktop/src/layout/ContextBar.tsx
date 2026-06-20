import {
  Component,
  For,
  Show,
  createResource,
  createSignal,
} from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { Avatar } from "../components";
import type { AccountSummary, AccountKind } from "../ipc/types";
import "./ContextBar.css";

/**
 * ContextBar —— 340px 右侧上下文栏(Home 页内容)。
 *
 * 三块:
 *   1. Playing as —— 账号选择器(头像 + 用户名 + 下拉箭头),展开切换账号。
 *   2. Friends    —— 好友列表占位(社交功能未接入时给空态)。
 *   3. News       —— 新闻/动态占位。
 *
 * 数据:createResource(() => invoke('list_accounts'))。loading / 空 / 错误三态都处理。
 * 背景 --n-2,左侧分隔(border-left)。
 */

// 账号类型 → 中文标签。AccountKind 在后端 serde 为小写(offline/microsoft/yggdrasil)。
const KIND_LABEL: Record<AccountKind, string> = {
  offline: "离线",
  microsoft: "微软",
  yggdrasil: "外置登录",
};

const ChevronDown = () => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
       stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="m6 9 6 6 6-6" />
  </svg>
);

const ContextBar: Component = () => {
  // 账号列表。refetch 用于切换账号后刷新 selected 标记。
  const [accounts, { refetch }] = createResource<AccountSummary[]>(async () => {
    return await invoke<AccountSummary[]>("list_accounts");
  });

  // 选择器展开态
  const [open, setOpen] = createSignal(false);
  // 切换账号时的错误提示(后端命令缺失/失败时显示,不崩 UI)
  const [switchErr, setSwitchErr] = createSignal<string | null>(null);

  // 当前账号:优先 selected,否则第一个。
  const current = (): AccountSummary | undefined => {
    const list = accounts();
    if (!list || list.length === 0) return undefined;
    return list.find((a) => a.selected) ?? list[0];
  };


  // 切换到指定账号:调后端 select_account(对应 core 的 AccountStore::select),
  // 成功后收起下拉并刷新列表。命令不存在/失败时记录错误、不阻塞 UI。
  const pick = async (acc: AccountSummary) => {
    setSwitchErr(null);
    if (acc.selected) {
      setOpen(false);
      return;
    }
    try {
      await invoke("select_account", { uuid: acc.uuid });
      await refetch();
    } catch (e) {
      setSwitchErr(typeof e === "string" ? e : "切换账号失败");
    } finally {
      setOpen(false);
    }
  };

  return (
    <aside class="ctxbar" aria-label="上下文信息">
      {/* ===== Playing as ===== */}
      <section class="ctx-section">
        <h3 class="ctx-heading">Playing as</h3>

        <Show
          when={!accounts.loading}
          fallback={<div class="account-card skeleton" aria-busy="true" />}
        >
          {/* 错误态:list_accounts 失败 */}
          <Show
            when={!accounts.error}
            fallback={
              <div class="ctx-error">
                <span>账号载入失败</span>
                <button class="ctx-retry" onClick={() => refetch()}>重试</button>
              </div>
            }
          >
            {/* 空态:无任何账号 */}
            <Show
              when={(accounts()?.length ?? 0) > 0}
              fallback={
                <div class="account-empty">
                  <span class="account-empty-text">尚未添加账号</span>
                  <span class="account-empty-hint">前往设置登录</span>
                </div>
              }
            >
              {/* 选择器触发器 */}
              <button
                class="account-trigger"
                classList={{ open: open() }}
                onClick={() => setOpen((v) => !v)}
                aria-expanded={open()}
                aria-haspopup="listbox"
              >
                <span class="account-avatar">
                  <Avatar kind={current()?.kind} uuid={current()?.uuid} />
                </span>
                <span class="account-meta">
                  <span class="account-name">{current()?.username}</span>
                  <span class="account-kind">
                    {current() ? KIND_LABEL[current()!.kind] : ""}
                  </span>
                </span>
                <span class="account-chevron" aria-hidden="true">
                  <ChevronDown />
                </span>
              </button>

              {/* 切换错误提示 */}
              <Show when={switchErr()}>
                <div class="account-switch-err">{switchErr()}</div>
              </Show>

              {/* 下拉:全部账号 */}
              <Show when={open()}>
                <ul class="account-list" role="listbox">
                  <For each={accounts()}>
                    {(acc) => (
                      <li
                        role="option"
                        aria-selected={acc.selected}
                        class="account-option"
                        classList={{ selected: acc.selected }}
                        onClick={() => pick(acc)}
                      >
                        <span class="account-avatar sm">
                          <Avatar kind={acc.kind} uuid={acc.uuid} />
                        </span>
                        <span class="account-meta">
                          <span class="account-name">{acc.username}</span>
                          <span class="account-kind">{KIND_LABEL[acc.kind]}</span>
                        </span>
                        <Show when={acc.selected}>
                          <span class="account-check" aria-hidden="true">✓</span>
                        </Show>
                      </li>
                    )}
                  </For>
                </ul>
              </Show>
            </Show>
          </Show>
        </Show>
      </section>

      {/* ===== Friends ===== */}
      <section class="ctx-section">
        <h3 class="ctx-heading">Friends</h3>
        {/* 社交功能未接入:空态占位。接入后此处渲染好友 + 在线状态点。 */}
        <div class="ctx-empty">
          <span class="ctx-empty-text">暂无好友</span>
          <span class="ctx-empty-hint">联机/社交功能开发中</span>
        </div>
      </section>

      {/* ===== News ===== */}
      <section class="ctx-section">
        <h3 class="ctx-heading">News</h3>
        {/* 新闻 feed 未接入:空态占位。接入后渲染公告/更新卡片列表。 */}
        <div class="ctx-empty">
          <span class="ctx-empty-text">暂无动态</span>
          <span class="ctx-empty-hint">敬请期待</span>
        </div>
      </section>
    </aside>
  );
};

export default ContextBar;
