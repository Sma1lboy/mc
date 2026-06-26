import {
  Component,
  createEffect,
  createMemo,
  createResource,
  createSignal,
  For,
  onCleanup,
  Show,
} from "solid-js";
import {
  Button,
  Panel,
  Heading,
  Dialog,
  Select,
  Spinner,
  EmptyState,
  Toggle,
  Tag,
  toast,
} from "../components";
import { api, onRealmSyncProgress } from "../ipc/api";
import {
  kobeUser,
  isKobeSignedIn,
  setCurrentPage,
  instances,
  activeRoot,
  refreshInstances,
} from "../store";
import { t } from "../i18n";
import type { RealmSummary, RealmMember, SyncReport, InstanceSummary } from "../ipc/bindings";

// 表单输入(与账号面板一致的石质暗底深凹倒角)。
const INPUT =
  "h-[38px] px-[14px] rounded-none text-[13px] text-fg bg-sidebar shadow-input w-full " +
  "placeholder:text-faint transition-[box-shadow] duration-150 ease-app " +
  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent";

/** 把后端 role 字符串映射到本地化标签。 */
function roleLabel(role: string): string {
  return role === "owner"
    ? t("realm.roleOwner")
    : role === "admin"
      ? t("realm.roleAdmin")
      : t("realm.roleMember");
}

/** 实例下拉项:名称 + 版本/loader 提示。 */
function instanceOptions(list: InstanceSummary[]): { value: string; label: string }[] {
  return list.map((i) => ({
    value: i.id,
    label: i.loader && i.loader !== "vanilla" ? `${i.name} · ${i.mc_version} · ${i.loader}` : `${i.name} · ${i.mc_version}`,
  }));
}

// 领域 → 本地实例 的绑定(每客户端,存 localStorage):创建时确定 / 加入者首次绑定,
// 之后**锁定不可替换**,且作为自动同步的目标。
const BIND_KEY = "mc-launcher.realm-instance";
function readBindings(): Record<string, string> {
  if (typeof window === "undefined") return {};
  try {
    return JSON.parse(window.localStorage.getItem(BIND_KEY) ?? "{}") as Record<string, string>;
  } catch {
    return {};
  }
}
function getBoundInstance(realmId: string): string | null {
  return readBindings()[realmId] ?? null;
}
function setBoundInstance(realmId: string, instanceId: string): void {
  if (typeof window === "undefined") return;
  const m = readBindings();
  m[realmId] = instanceId;
  try {
    window.localStorage.setItem(BIND_KEY, JSON.stringify(m));
  } catch {
    /* localStorage 不可用时忽略 */
  }
}

/**
 * Realms —— 临时领域(我们自己的私有共享 mod 集,区别于 Minecraft 官方 Realms)。
 *
 * 未登录 kobeMC → 引导去设置登录。已登录 → 我的领域列表 + 创建(基于实例)/ 加入(加入码);
 * 进入某领域看成员与清单版本,成员可把本地实例一键同步到清单,owner/admin 可从实例发布新清单。
 */
const Realms: Component = () => {
  const [realms, { refetch: refetchRealms }] = createResource(
    () => (isKobeSignedIn() ? kobeUser()?.id : null),
    () => api.realmList(),
  );

  const [selected, setSelected] = createSignal<RealmSummary | null>(null);
  const [createOpen, setCreateOpen] = createSignal(false);
  const [joinOpen, setJoinOpen] = createSignal(false);

  function onCreated(r: RealmSummary) {
    setCreateOpen(false);
    void refetchRealms();
    setSelected(r);
  }
  function onJoined(r: RealmSummary) {
    setJoinOpen(false);
    void refetchRealms();
    setSelected(r);
  }

  return (
    <div class="p-[24px] max-w-[1100px] mx-auto w-full">
      <div class="flex items-start justify-between gap-[16px] mb-[18px]">
        <div class="min-w-0">
          <Heading size="page" as="h1" class="mt-0 mb-[6px]">
            {t("realm.title")}
          </Heading>
          <p class="text-[13px] text-muted leading-[1.6] max-w-[640px]">{t("realm.subtitle")}</p>
        </div>
        <Show when={isKobeSignedIn() && !selected()}>
          <div class="flex gap-[8px] shrink-0">
            <Button variant="ghost" onClick={() => setJoinOpen(true)}>
              {t("realm.joinAction")}
            </Button>
            <Button variant="primary" onClick={() => setCreateOpen(true)}>
              {t("realm.createAction")}
            </Button>
          </div>
        </Show>
      </div>

      <Show
        when={isKobeSignedIn()}
        fallback={
          <Panel variant="sunken" class="p-[28px]">
            <EmptyState
              title={t("realm.needLogin")}
              action={
                <Button variant="primary" onClick={() => setCurrentPage("settings")}>
                  {t("realm.goLogin")}
                </Button>
              }
            />
          </Panel>
        }
      >
        <Show
          when={selected()}
          fallback={
            <RealmList
              realms={realms()}
              loading={realms.loading}
              onOpen={setSelected}
              onCreate={() => setCreateOpen(true)}
              onJoin={() => setJoinOpen(true)}
            />
          }
        >
          <RealmDetail
            realm={selected()!}
            onBack={() => {
              setSelected(null);
              void refetchRealms();
            }}
            onGone={() => {
              setSelected(null);
              void refetchRealms();
            }}
          />
        </Show>
      </Show>

      <CreateRealmDialog open={createOpen()} onClose={() => setCreateOpen(false)} onCreated={onCreated} />
      <JoinRealmDialog open={joinOpen()} onClose={() => setJoinOpen(false)} onJoined={onJoined} />
    </div>
  );
};

/* ---------- realm list ---------- */

const RealmList: Component<{
  realms: RealmSummary[] | undefined;
  loading: boolean;
  onOpen: (r: RealmSummary) => void;
  onCreate: () => void;
  onJoin: () => void;
}> = (props) => {
  return (
    <Show
      when={!props.loading}
      fallback={
        <div class="flex justify-center p-[40px]">
          <Spinner />
        </div>
      }
    >
      <Show
        when={(props.realms ?? []).length > 0}
        fallback={
          <Panel variant="sunken" class="p-[28px]">
            <EmptyState
              title={t("realm.empty")}
              action={
                <div class="flex gap-[8px]">
                  <Button variant="ghost" onClick={props.onJoin}>
                    {t("realm.joinAction")}
                  </Button>
                  <Button variant="primary" onClick={props.onCreate}>
                    {t("realm.createAction")}
                  </Button>
                </div>
              }
            />
          </Panel>
        }
      >
        <div class="grid gap-[12px] grid-cols-[repeat(auto-fill,minmax(280px,1fr))]">
          <For each={props.realms}>
            {(r) => (
              <button
                type="button"
                class="text-left bg-transparent border-none p-0 cursor-pointer"
                onClick={() => props.onOpen(r)}
              >
                <Panel variant="raised" class="p-[16px] h-full flex flex-col gap-[10px] hover:brightness-110 transition-[filter] duration-150">
                  <div class="flex items-center gap-[8px]">
                    <span class="font-display text-[15px] text-strong truncate flex-1">{r.name}</span>
                    <Tag>{roleLabel(r.role)}</Tag>
                  </div>
                  <div class="flex flex-wrap gap-[6px] text-[11px] text-muted">
                    <Show when={r.mc_version}>
                      <span class="bg-sidebar shadow-input px-[6px] py-[2px]">{r.mc_version}</span>
                    </Show>
                    <Show when={r.loader && r.loader !== "vanilla"}>
                      <span class="bg-sidebar shadow-input px-[6px] py-[2px]">{r.loader}</span>
                    </Show>
                    <span class="bg-sidebar shadow-input px-[6px] py-[2px]">
                      {t("realm.manifestVersion", { version: r.manifest_version })}
                    </span>
                  </div>
                  <div class="mt-auto flex items-center gap-[6px] text-[12px] text-faint">
                    <span>{t("realm.codeLabel")}</span>
                    <span class="font-mono text-accent tracking-[0.12em]">{r.code}</span>
                  </div>
                </Panel>
              </button>
            )}
          </For>
        </div>
      </Show>
    </Show>
  );
};

/* ---------- realm detail ---------- */

const RealmDetail: Component<{
  realm: RealmSummary;
  onBack: () => void;
  onGone: () => void;
}> = (props) => {
  const realmId = () => props.realm.id;
  const isOwner = () => props.realm.role === "owner";
  const canPush = () => props.realm.role === "owner" || props.realm.role === "admin";
  const myId = () => kobeUser()?.id;

  const [members, { refetch: refetchMembers }] = createResource(realmId, () => api.realmMembers(realmId()));

  const insts = () => instances() ?? [];

  // 绑定的本地实例:创建时确定 / 加入者首次绑定,之后锁定不可替换,且作为自动同步目标。
  const [boundId, setBoundIdSig] = createSignal<string | null>(getBoundInstance(props.realm.id));
  const boundInstance = createMemo(() => {
    const id = boundId();
    return id ? insts().find((i) => i.id === id) ?? null : null;
  });
  function bindInstance(id: string) {
    setBoundInstance(props.realm.id, id);
    setBoundIdSig(id);
  }
  const [pickInst, setPickInst] = createSignal(""); // 加入者首次绑定的临时选择

  const [removeExtras, setRemoveExtras] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [progress, setProgress] = createSignal<{ current: number; total: number } | null>(null);
  const [confirmKind, setConfirmKind] = createSignal<"leave" | "disband" | null>(null);

  // 自动检测差异:绑定实例后,随 (领域, 清单版本) 自动重算,无需手动点击。
  const [plan, { refetch: refetchPlan }] = createResource(
    () => {
      const inst = boundInstance();
      return inst ? { rid: realmId(), iid: inst.id, mv: props.realm.manifest_version } : null;
    },
    (k) => api.realmPlanSync(k.rid, activeRoot(), k.iid),
  );

  // 领域同步进度(专用事件,避免与安装队列串台)。仅在本页挂载期间订阅。
  onCleanup(onRealmSyncProgress((p) => setProgress({ current: p.current, total: p.total })));

  function fail(e: unknown) {
    toast({ type: "error", message: t("realm.opError", { err: String(e) }) });
  }

  // 自动同步:当差异为「纯新增」(无需移除)时自动下载;有移除(破坏性)留给用户显式确认。
  // 用「领域:版本」键去重,避免重复触发,且失败后不死循环(手动重试仍可)。
  let autoSyncedKey = "";
  createEffect(() => {
    const p = plan();
    if (!p || !boundInstance() || busy()) return;
    const key = `${realmId()}:${p.version}`;
    if (p.download.length > 0 && p.remove.length === 0 && autoSyncedKey !== key) {
      autoSyncedKey = key;
      void runSync(false);
    }
  });

  async function runSync(remove: boolean) {
    const inst = boundInstance();
    if (!inst || busy()) return;
    setBusy(true);
    setProgress({ current: 0, total: 0 });
    try {
      const report: SyncReport = await api.realmSync(realmId(), activeRoot(), inst.id, remove);
      void refreshInstances();
      void refetchMembers();
      await refetchPlan();
      setRemoveExtras(false);
      toast({
        type: report.failed.length ? "error" : "success",
        message: report.failed.length
          ? t("realm.syncFailed", { count: report.failed.length })
          : t("realm.syncDone", { downloaded: report.downloaded, removed: report.removed }),
      });
      if (report.manual.length) {
        toast({ type: "info", message: t("realm.manualCount", { count: report.manual.length }) });
      }
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
      setProgress(null);
    }
  }

  async function pushManifest() {
    const inst = boundInstance();
    if (!inst) return;
    setBusy(true);
    try {
      const version = await api.realmPushManifest(
        realmId(),
        activeRoot(),
        inst.id,
        inst.mc_version,
        inst.loader ?? "vanilla",
        inst.loader_version ?? null,
      );
      toast({ type: "success", message: t("realm.pushDone", { version }) });
      props.onBack(); // 回列表刷新清单版本
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  }

  async function copyCode() {
    try {
      await navigator.clipboard.writeText(props.realm.code);
      toast({ type: "success", message: t("realm.copied") });
    } catch (e) {
      fail(e);
    }
  }

  async function setRole(uid: string, role: string) {
    setBusy(true);
    try {
      await api.realmSetRole(realmId(), uid, role);
      await refetchMembers();
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  }

  async function removeMember(uid: string) {
    setBusy(true);
    try {
      await api.realmRemoveMember(realmId(), uid);
      await refetchMembers();
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  }

  // 退出 / 解散走应用内 Dialog 确认(原生 window.confirm 在 Linux WebKitGTK 上可能是
  // no-op,会让操作静默失效,且与全站的危险操作确认样式不一致)。
  async function doLeave() {
    if (!myId()) return;
    setConfirmKind(null);
    setBusy(true);
    try {
      await api.realmRemoveMember(realmId(), myId()!);
      props.onGone();
    } catch (e) {
      fail(e);
      setBusy(false);
    }
  }

  async function doDisband() {
    setConfirmKind(null);
    setBusy(true);
    try {
      await api.realmDisband(realmId());
      props.onGone();
    } catch (e) {
      fail(e);
      setBusy(false);
    }
  }

  return (
    <div class="flex flex-col gap-[16px]">
      <button
        type="button"
        class="self-start bg-transparent border-none p-0 text-[13px] text-muted hover:text-fg cursor-pointer"
        onClick={props.onBack}
      >
        ← {t("realm.back")}
      </button>

      {/* 头部:名称 / 角色 / 版本 / 加入码 */}
      <Panel variant="sunken" class="p-[20px]">
        <div class="flex items-start justify-between gap-[12px] flex-wrap">
          <div class="min-w-0">
            <div class="flex items-center gap-[10px] mb-[6px]">
              <Heading size="sub" as="h2" class="m-0">
                {props.realm.name}
              </Heading>
              <Tag>{roleLabel(props.realm.role)}</Tag>
            </div>
            <div class="flex flex-wrap gap-[6px] text-[11px] text-muted">
              <Show when={props.realm.mc_version}>
                <span class="bg-window shadow-input px-[6px] py-[2px]">{props.realm.mc_version}</span>
              </Show>
              <Show when={props.realm.loader && props.realm.loader !== "vanilla"}>
                <span class="bg-window shadow-input px-[6px] py-[2px]">{props.realm.loader}</span>
              </Show>
              <span class="bg-window shadow-input px-[6px] py-[2px]">
                {t("realm.manifestVersion", { version: props.realm.manifest_version })}
              </span>
            </div>
          </div>
          <div class="flex flex-col items-end gap-[6px]">
            <span class="text-[11px] text-faint">{t("realm.codeLabel")}</span>
            <button
              type="button"
              class="font-mono text-[18px] text-accent tracking-[0.16em] bg-window shadow-input px-[12px] py-[6px] cursor-pointer hover:brightness-110"
              title={t("realm.copyCode")}
              onClick={() => void copyCode()}
            >
              {props.realm.code}
            </button>
          </div>
        </div>
      </Panel>

      {/* 同步(绑定实例 → 自动检测差异 → 自动同步) */}
      <Panel variant="sunken" class="p-[20px]">
        <Heading size="sub" as="h2" class="mb-[6px]">
          {t("realm.syncTitle")}
        </Heading>

        <Show
          when={boundInstance()}
          fallback={
            // 未绑定(加入者首次):选一个实例绑定,之后锁定。
            <Show
              when={insts().length > 0}
              fallback={<p class="text-[13px] text-muted">{t("realm.noInstances")}</p>}
            >
              <p class="text-[12px] text-muted mb-[12px] leading-[1.6]">{t("realm.bindHint")}</p>
              <div class="flex items-center gap-[10px] flex-wrap">
                <Select
                  class="min-w-[240px]"
                  value={pickInst()}
                  onChange={setPickInst}
                  options={instanceOptions(insts())}
                  placeholder={t("realm.pickInstance")}
                />
                <Button variant="primary" disabled={!pickInst()} onClick={() => bindInstance(pickInst())}>
                  {t("realm.bindAction")}
                </Button>
              </div>
            </Show>
          }
        >
          {(inst) => (
            <div class="flex flex-col gap-[12px]">
              {/* 绑定信息(锁定,不可替换) */}
              <div class="flex items-center gap-[8px] text-[13px] text-fg flex-wrap">
                <span class="text-muted">{t("realm.boundInstance")}</span>
                <span class="font-display text-strong">{inst().name}</span>
                <Tag>{t("realm.boundLocked")}</Tag>
              </div>

              {/* 状态:检查中 / 同步中 / 已最新 / 待确认的差异 */}
              <Show
                when={!plan.loading}
                fallback={<p class="text-[13px] text-muted">{t("realm.checking")}</p>}
              >
                <Show when={busy()}>
                  <p class="text-[13px] text-accent">{t("realm.syncing")}</p>
                </Show>
                <Show when={!busy() && plan()}>
                  {(p) => (
                    <Show
                      when={!(p().download.length === 0 && p().remove.length === 0)}
                      fallback={<p class="text-[13px] text-accent">{t("realm.planUpToDate")}</p>}
                    >
                      <div class="flex flex-wrap gap-[8px] text-[12px]">
                        <Show when={p().download.length}>
                          <span class="bg-window shadow-input px-[8px] py-[3px]">
                            {t("realm.planDownload", { count: p().download.length })}
                          </span>
                        </Show>
                        <Show when={p().remove.length}>
                          <span class="bg-window shadow-input px-[8px] py-[3px]">
                            {t("realm.planRemove", { count: p().remove.length })}
                          </span>
                        </Show>
                        <Show when={p().manual.length}>
                          <span class="bg-window shadow-input px-[8px] py-[3px]">
                            {t("realm.planManual", { count: p().manual.length })}
                          </span>
                        </Show>
                      </div>

                      {/* 破坏性:有「需移除」时不自动执行,显式确认后再应用 */}
                      <Show when={p().remove.length > 0}>
                        <div class="flex items-center justify-between text-[13px] text-fg mt-[8px]">
                          <div class="flex flex-col gap-[2px] min-w-0 pr-[12px]">
                            <span>{t("realm.removeExtras")}</span>
                            <span class="text-[11px] text-muted">{t("realm.removeExtrasHint")}</span>
                          </div>
                          <Toggle checked={removeExtras()} onChange={setRemoveExtras} disabled={busy()} />
                        </div>
                        <Button
                          variant="primary"
                          class="self-start mt-[8px]"
                          disabled={busy()}
                          onClick={() => void runSync(removeExtras())}
                        >
                          {t("realm.applyChanges")}
                        </Button>
                      </Show>

                      <Show when={p().manual.length > 0}>
                        <div class="text-[12px] text-muted mt-[8px]">
                          <div class="mb-[4px]">{t("realm.manualList")}</div>
                          <ul class="list-disc pl-[18px] flex flex-col gap-[2px]">
                            <For each={p().manual}>
                              {(f) => <li class="break-all text-faint">{f.path.replace(/^mods\//, "")}</li>}
                            </For>
                          </ul>
                        </div>
                      </Show>
                    </Show>
                  )}
                </Show>
              </Show>

              {/* 同步进度条(自动 / 手动同步时显示) */}
              <Show when={progress()}>
                {(pr) => (
                  <div class="h-[6px] w-full bg-window shadow-input rounded-none overflow-hidden">
                    <div
                      class="h-full bg-accent transition-[width] duration-150 ease-app"
                      style={{
                        width: `${pr().total > 0 ? Math.round((pr().current / pr().total) * 100) : 8}%`,
                      }}
                    />
                  </div>
                )}
              </Show>

              {/* owner/admin:从绑定实例发布新清单 */}
              <Show when={canPush()}>
                <div class="mt-[8px] pt-[16px] border-t border-titlebar">
                  <Heading size="sub" as="h3" class="mb-[6px] text-[14px]">
                    {t("realm.pushTitle")}
                  </Heading>
                  <p class="text-[12px] text-muted mb-[10px] leading-[1.6]">{t("realm.pushHint")}</p>
                  <Button variant="ghost" disabled={busy()} onClick={() => void pushManifest()}>
                    {busy() ? t("realm.pushing") : t("realm.pushAction")}
                  </Button>
                </div>
              </Show>
            </div>
          )}
        </Show>
      </Panel>

      {/* 成员 */}
      <Panel variant="sunken" class="p-[20px]">
        <Heading size="sub" as="h2" class="mb-[14px]">
          {t("realm.members")}
        </Heading>
        <Show
          when={!members.loading}
          fallback={
            <div class="flex justify-center p-[16px]">
              <Spinner />
            </div>
          }
        >
          <div class="flex flex-col gap-[8px]">
            <For each={members() ?? []}>
              {(m: RealmMember) => {
                const isMe = () => m.user_id === myId();
                return (
                  <Panel variant="raised" class="flex items-center gap-[10px] px-[12px] py-[9px]">
                    <div class="flex flex-col min-w-0 flex-1">
                      <span class="text-[13px] text-fg truncate">
                        {m.username || m.user_id.slice(0, 8)}
                        <Show when={isMe()}>
                          <span class="text-muted"> {t("realm.you")}</span>
                        </Show>
                      </span>
                      <span class="text-[11px] text-faint">
                        {m.synced_version > 0
                          ? t("realm.syncedTo", { version: m.synced_version })
                          : t("realm.notSynced")}
                      </span>
                    </div>
                    <Tag>{roleLabel(m.role)}</Tag>
                    {/* owner 管理其它成员(非 owner 行) */}
                    <Show when={isOwner() && m.role !== "owner"}>
                      <div class="flex items-center gap-[6px] shrink-0">
                        <Show
                          when={m.role === "member"}
                          fallback={
                            <button
                              class="text-[12px] text-muted hover:text-fg bg-transparent border-none cursor-pointer disabled:opacity-50"
                              disabled={busy()}
                              onClick={() => void setRole(m.user_id, "member")}
                            >
                              {t("realm.demote")}
                            </button>
                          }
                        >
                          <button
                            class="text-[12px] text-muted hover:text-fg bg-transparent border-none cursor-pointer disabled:opacity-50"
                            disabled={busy()}
                            onClick={() => void setRole(m.user_id, "admin")}
                          >
                            {t("realm.promote")}
                          </button>
                        </Show>
                        <button
                          class="text-[12px] text-danger-text hover:underline bg-transparent border-none cursor-pointer disabled:opacity-50"
                          disabled={busy()}
                          onClick={() => void removeMember(m.user_id)}
                        >
                          {t("realm.removeMember")}
                        </button>
                      </div>
                    </Show>
                  </Panel>
                );
              }}
            </For>
          </div>
        </Show>

        <div class="mt-[16px] pt-[16px] border-t border-titlebar flex justify-end">
          <Show
            when={isOwner()}
            fallback={
              <Button variant="danger" disabled={busy()} onClick={() => setConfirmKind("leave")}>
                {t("realm.leave")}
              </Button>
            }
          >
            <Button variant="danger" disabled={busy()} onClick={() => setConfirmKind("disband")}>
              {t("realm.disband")}
            </Button>
          </Show>
        </div>
      </Panel>

      {/* 危险操作:应用内 Dialog 确认(退出 / 解散) */}
      <Dialog
        open={confirmKind() !== null}
        onClose={() => setConfirmKind(null)}
        label={confirmKind() === "disband" ? t("realm.disband") : t("realm.leave")}
      >
        <div class="p-[20px] flex flex-col gap-[16px]">
          <p class="text-[14px] text-fg leading-[1.6]">
            {confirmKind() === "disband"
              ? t("realm.confirmDisband", { name: props.realm.name })
              : t("realm.confirmLeave", { name: props.realm.name })}
          </p>
          <div class="flex justify-end gap-[8px]">
            <Button variant="ghost" onClick={() => setConfirmKind(null)}>
              {t("realm.cancel")}
            </Button>
            <Button
              variant="danger"
              disabled={busy()}
              onClick={() => void (confirmKind() === "disband" ? doDisband() : doLeave())}
            >
              {confirmKind() === "disband" ? t("realm.disband") : t("realm.leave")}
            </Button>
          </div>
        </div>
      </Dialog>
    </div>
  );
};

/* ---------- dialogs ---------- */

const EXPIRY_OPTIONS = (): { value: string; label: string }[] => [
  { value: "0", label: t("realm.expiryNever") },
  { value: "86400", label: t("realm.expiry1d") },
  { value: "604800", label: t("realm.expiry7d") },
  { value: "2592000", label: t("realm.expiry30d") },
];

const CreateRealmDialog: Component<{
  open: boolean;
  onClose: () => void;
  onCreated: (r: RealmSummary) => void;
}> = (props) => {
  const insts = () => instances() ?? [];
  const [name, setName] = createSignal("");
  const [instId, setInstId] = createSignal("");
  const [expiry, setExpiry] = createSignal("0");
  const [busy, setBusy] = createSignal(false);
  const picked = createMemo(() => insts().find((i) => i.id === instId()) ?? null);

  async function submit() {
    const inst = picked();
    if (busy() || !name().trim() || !inst) return;
    setBusy(true);
    try {
      const secs = parseInt(expiry(), 10) || 0;
      const r = await api.realmCreate(
        activeRoot(),
        inst.id,
        name().trim(),
        inst.mc_version,
        inst.loader ?? "vanilla",
        inst.loader_version ?? null,
        secs > 0 ? secs : null,
      );
      // 创建时即把该实例绑定到新领域(之后锁定,不可替换)。
      setBoundInstance(r.id, inst.id);
      toast({ type: "success", message: t("realm.createdToast", { name: r.name }) });
      setName("");
      setInstId("");
      setExpiry("0");
      props.onCreated(r);
    } catch (e) {
      toast({ type: "error", message: t("realm.opError", { err: String(e) }) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={props.open} onClose={props.onClose} label={t("realm.createTitle")}>
      <div class="p-[20px] flex flex-col gap-[14px]">
        <Heading size="sub" as="h2" class="m-0">
          {t("realm.createTitle")}
        </Heading>

        <label class="flex flex-col gap-[6px]">
          <span class="text-[12px] text-muted">{t("realm.nameLabel")}</span>
          <input
            class={INPUT}
            type="text"
            placeholder={t("realm.namePlaceholder")}
            value={name()}
            onInput={(e) => setName(e.currentTarget.value)}
          />
        </label>

        <label class="flex flex-col gap-[6px]">
          <span class="text-[12px] text-muted">{t("realm.sourceInstance")}</span>
          <Show
            when={insts().length > 0}
            fallback={<span class="text-[12px] text-faint">{t("realm.noInstances")}</span>}
          >
            <Select
              value={instId()}
              onChange={setInstId}
              options={instanceOptions(insts())}
              placeholder={t("realm.pickInstance")}
            />
          </Show>
          <span class="text-[11px] text-faint">{t("realm.sourceInstanceHint")}</span>
        </label>

        <label class="flex flex-col gap-[6px]">
          <span class="text-[12px] text-muted">{t("realm.expiry")}</span>
          <Select value={expiry()} onChange={setExpiry} options={EXPIRY_OPTIONS()} />
        </label>

        <div class="flex justify-end gap-[8px] mt-[4px]">
          <Button variant="ghost" onClick={props.onClose}>
            {t("realm.cancel")}
          </Button>
          <Button variant="primary" disabled={busy() || !name().trim() || !picked()} onClick={() => void submit()}>
            {busy() ? t("realm.creating") : t("realm.createSubmit")}
          </Button>
        </div>
      </div>
    </Dialog>
  );
};

const JoinRealmDialog: Component<{
  open: boolean;
  onClose: () => void;
  onJoined: (r: RealmSummary) => void;
}> = (props) => {
  const [code, setCode] = createSignal("");
  const [busy, setBusy] = createSignal(false);

  async function submit() {
    if (busy() || !code().trim()) return;
    setBusy(true);
    try {
      const r = await api.realmJoin(code().trim());
      if (!r) {
        toast({ type: "error", message: t("realm.joinBadCode") });
        return;
      }
      toast({ type: "success", message: t("realm.joinedToast", { name: r.name }) });
      setCode("");
      props.onJoined(r);
    } catch (e) {
      toast({ type: "error", message: t("realm.opError", { err: String(e) }) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={props.open} onClose={props.onClose} label={t("realm.joinTitle")}>
      <div class="p-[20px] flex flex-col gap-[14px]">
        <Heading size="sub" as="h2" class="m-0">
          {t("realm.joinTitle")}
        </Heading>
        <label class="flex flex-col gap-[6px]">
          <span class="text-[12px] text-muted">{t("realm.joinCodeLabel")}</span>
          <input
            class={`${INPUT} font-mono tracking-[0.16em] uppercase`}
            type="text"
            maxLength={6}
            placeholder={t("realm.joinCodePlaceholder")}
            value={code()}
            onInput={(e) => setCode(e.currentTarget.value.toUpperCase())}
            onKeyDown={(e) => e.key === "Enter" && void submit()}
          />
        </label>
        <div class="flex justify-end gap-[8px] mt-[4px]">
          <Button variant="ghost" onClick={props.onClose}>
            {t("realm.cancel")}
          </Button>
          <Button variant="primary" disabled={busy() || !code().trim()} onClick={() => void submit()}>
            {busy() ? t("realm.joining") : t("realm.joinSubmit")}
          </Button>
        </div>
      </div>
    </Dialog>
  );
};

export default Realms;
