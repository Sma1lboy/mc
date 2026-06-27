import { Component, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { Button } from "./Button";
import { Checkbox } from "./Checkbox";
import { Icon } from "./Icon";
import { LinkedAccountsSection } from "./LinkedAccountsSection";
import { kobeUser, kobeLogin, kobeSignup, kobeLogout, kobeDisplayName } from "../store";
import { api } from "../ipc/api";
import type { KobeCredentials } from "../ipc/bindings";
import { t } from "../i18n";

/** 用户名规则:3–24 位,字母/数字/下划线/连字符。 */
const USERNAME_RE = /^[A-Za-z0-9_-]{3,24}$/;

const INPUT =
  "h-[34px] px-[12px] rounded-none text-[13px] text-fg bg-sidebar shadow-input w-full " +
  "placeholder:text-faint focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent";

/**
 * KobeAccountChip —— 顶栏的 kobeMC 账号入口(从设置里提升到最外层)。
 * 未登录:一个「登录」chip,点开下拉里登录/注册。已登录:头像+名字 chip,下拉里显示
 * 账号信息 + 退出(好友入口在 Phase 2 接到这里)。
 */
export const KobeAccountChip: Component = () => {
  const [open, setOpen] = createSignal(false);
  let rootEl: HTMLDivElement | undefined;

  onMount(() => {
    const onDoc = (e: MouseEvent) => {
      if (rootEl && !rootEl.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    onCleanup(() => document.removeEventListener("mousedown", onDoc));
  });

  return (
    <div ref={rootEl} class="relative">
      <button
        type="button"
        class="inline-flex items-center gap-[7px] h-[30px] pl-[6px] pr-[8px] border border-titlebar bg-panel-2 text-[12px] text-fg cursor-pointer transition-[background-color] duration-[var(--dur)] ease-app hover:bg-panel-3 [-webkit-app-region:no-drag]"
        onClick={() => setOpen((o) => !o)}
        title={t("kobe.title")}
      >
        {/* 头像色块:草方块标(登录态实色,未登录灰)。 */}
        <span
          class="w-[18px] h-[18px] shrink-0 grid grid-rows-[6px_1fr] shadow-input overflow-hidden"
          aria-hidden="true"
        >
          <span class={kobeUser() ? "bg-accent" : "bg-faint"} />
          <span class={kobeUser() ? "bg-[#7a5b3a]" : "bg-panel-3"} />
        </span>
        <span class="max-w-[120px] truncate">
          {kobeUser() ? kobeDisplayName(kobeUser()!) : t("kobe.loginAction")}
        </span>
        {/* 下拉指示 caret */}
        <svg class="w-[10px] h-[10px] shrink-0 text-muted" viewBox="0 0 12 12" fill="none" aria-hidden="true">
          <path d="M3 4.5 6 7.5 9 4.5" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
      </button>

      {/* 下拉体保持挂载、用 hidden 切换显隐(不再 <Show when={open()}> 销毁重建):
          这样关掉再打开时好友/请求来自 store 缓存、搜索框文字与滚动位置都保留,不再每次重拉。 */}
      <div
        class="absolute right-0 top-[calc(100%+6px)] w-[300px] bg-panel border border-titlebar shadow-raised rounded-none z-[200] p-[16px]"
        classList={{ hidden: !open() }}
      >
        <Show when={kobeUser()} fallback={<LoginForm onDone={() => setOpen(false)} />}>
          <div class="flex items-center gap-[10px] mb-[12px]">
            <span class="grid place-items-center w-[40px] h-[40px] bg-sidebar shadow-input text-accent shrink-0">
              <Icon name="user" size={20} />
            </span>
            <div class="flex flex-col min-w-0 flex-1">
              <span class="text-[14px] text-strong truncate">{kobeDisplayName(kobeUser()!)}</span>
              <Show when={kobeUser()!.email}>
                <span class="text-[12px] text-muted truncate">{kobeUser()!.email}</span>
              </Show>
            </div>
          </div>
          <Button
            variant="ghost"
            class="w-full justify-center"
            onClick={() => {
              void kobeLogout();
              setOpen(false);
            }}
          >
            {t("kobe.logoutAction")}
          </Button>

          <LinkedAccountsSection />
        </Show>
      </div>
    </div>
  );
};

const LoginForm: Component<{ onDone: () => void }> = (props) => {
  const [saved, setSaved] = createSignal<KobeCredentials[]>([]);
  const [showForm, setShowForm] = createSignal(false);
  const [mode, setMode] = createSignal<"login" | "signup">("login");
  const [email, setEmail] = createSignal("");
  const [password, setPassword] = createSignal("");
  const [username, setUsername] = createSignal("");
  const [remember, setRemember] = createSignal(false);
  const [auto, setAuto] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  // 拉记住的账号列表;有则先显示账号选择器,没有就直接进表单。
  onMount(() => void refreshSaved(true));

  async function refreshSaved(initial = false): Promise<void> {
    try {
      const list = await api.kobeListCredentials();
      setSaved(list);
      if (initial) setShowForm(list.length === 0);
    } catch {
      if (initial) setShowForm(true);
    }
  }

  // 用记住的某个账号一键登录。
  async function quickLogin(c: KobeCredentials): Promise<void> {
    if (busy()) return;
    setBusy(true);
    setError(null);
    try {
      await kobeLogin(c.email, c.password);
      props.onDone();
    } catch (err) {
      setError(t("kobe.errAuth", { err: String(err) }));
    } finally {
      setBusy(false);
    }
  }

  async function forget(c: KobeCredentials, ev: Event): Promise<void> {
    ev.stopPropagation();
    try {
      await api.kobeRemoveCredentials(c.email);
    } finally {
      await refreshSaved();
    }
  }

  async function toggleAuto(c: KobeCredentials, ev: Event): Promise<void> {
    ev.stopPropagation();
    try {
      await api.kobeSetAutoLogin(c.email, !c.auto_login);
    } finally {
      await refreshSaved();
    }
  }

  async function submit(e: Event): Promise<void> {
    e.preventDefault();
    if (busy()) return;
    setError(null);
    const mail = email().trim();
    if (!mail || !password()) {
      setError(t("kobe.errEmptyCreds"));
      return;
    }
    const name = username().trim();
    // 注册必须提供合法用户名:它同时是展示名 + 好友用户名(单一身份)。
    if (mode() === "signup" && !USERNAME_RE.test(name)) {
      setError(t("kobe.usernameInvalid"));
      return;
    }
    setBusy(true);
    try {
      if (mode() === "signup") {
        await kobeSignup(mail, password(), name);
      } else {
        await kobeLogin(mail, password());
      }
      // 记住密码 / 自动登录:勾了就存进 keyring(按 email 去重);没勾不动列表。
      if (remember()) await api.kobeSaveCredentials(mail, password(), auto());
      props.onDone();
    } catch (err) {
      setError(t("kobe.errAuth", { err: String(err) }));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div class="flex flex-col gap-[10px]">
      <div class="text-[13px] text-strong font-display mb-[2px]">{t("kobe.title")}</div>

      {/* 记住的账号列表:点行一键登录,右侧切换自动登录 / 移除。 */}
      <Show when={saved().length > 0}>
        <div class="flex flex-col gap-[4px]">
          <For each={saved()}>
            {(c) => (
              <div class="flex items-center gap-[6px] bg-sidebar shadow-input px-[8px] py-[6px]">
                <button
                  type="button"
                  class="flex items-center gap-[8px] flex-1 min-w-0 bg-transparent border-none p-0 cursor-pointer text-left disabled:opacity-50"
                  disabled={busy()}
                  onClick={() => void quickLogin(c)}
                  title={t("kobe.loginAction")}
                >
                  <span class="grid place-items-center w-[24px] h-[24px] bg-panel-2 shadow-input text-accent shrink-0">
                    <Icon name="user" size={14} />
                  </span>
                  <span class="text-[13px] text-fg truncate">{c.email}</span>
                </button>
                <button
                  type="button"
                  class="shrink-0 text-[11px] px-[5px] py-[2px] bg-transparent border-none cursor-pointer hover:underline"
                  classList={{ "text-accent": c.auto_login, "text-faint": !c.auto_login }}
                  title={c.auto_login ? t("kobe.autoLoginOn") : t("kobe.autoLoginOff")}
                  onClick={(ev) => void toggleAuto(c, ev)}
                >
                  {t("kobe.autoShort")}
                </button>
                <button
                  type="button"
                  class="shrink-0 grid place-items-center w-[18px] h-[18px] text-faint hover:text-danger-text bg-transparent border-none cursor-pointer"
                  title={t("kobe.forget")}
                  onClick={(ev) => void forget(c, ev)}
                >
                  <Icon name="close" size={12} />
                </button>
              </div>
            )}
          </For>
        </div>
      </Show>

      {/* 账号选择器模式下,提供「用其它账号登录 / 注册」入口切到表单。 */}
      <Show
        when={showForm()}
        fallback={
          <button
            type="button"
            class="self-center bg-transparent border-none p-0 text-[12px] text-accent cursor-pointer hover:underline"
            onClick={() => setShowForm(true)}
          >
            {t("kobe.useAnother")}
          </button>
        }
      >
        <form class="flex flex-col gap-[10px]" onSubmit={submit}>
          <Show when={mode() === "signup"}>
            <input
              class={INPUT}
              type="text"
              autocomplete="username"
              maxLength={24}
              placeholder={t("kobe.usernamePlaceholder")}
              value={username()}
              onInput={(ev) => setUsername(ev.currentTarget.value)}
            />
          </Show>
          <input
            class={INPUT}
            type="email"
            autocomplete="email"
            placeholder={t("kobe.emailPlaceholder")}
            value={email()}
            onInput={(ev) => setEmail(ev.currentTarget.value)}
          />
          <input
            class={INPUT}
            type="password"
            autocomplete={mode() === "signup" ? "new-password" : "current-password"}
            placeholder={t("kobe.passwordPlaceholder")}
            value={password()}
            onInput={(ev) => setPassword(ev.currentTarget.value)}
          />
          {/* 记住密码 / 自动登录(自动登录隐含记住密码)。 */}
          <div class="flex items-center gap-[16px]">
            <Checkbox
              label={t("kobe.rememberPassword")}
              checked={remember()}
              onChange={(v) => {
                setRemember(v);
                if (!v) setAuto(false);
              }}
            />
            <Checkbox
              label={t("kobe.autoLogin")}
              checked={auto()}
              onChange={(v) => {
                setAuto(v);
                if (v) setRemember(true);
              }}
            />
          </div>
          <Show when={error()}>
            <p class="text-[12px] text-danger-text">{error()}</p>
          </Show>
          <Button variant="primary" type="submit" disabled={busy()} class="w-full justify-center">
            {busy() ? t("kobe.working") : mode() === "signup" ? t("kobe.signupAction") : t("kobe.loginAction")}
          </Button>
          <button
            type="button"
            class="self-center bg-transparent border-none p-0 text-[12px] text-accent cursor-pointer hover:underline"
            onClick={() => {
              setMode(mode() === "login" ? "signup" : "login");
              setError(null);
            }}
          >
            {mode() === "login" ? t("kobe.switchToSignup") : t("kobe.switchToLogin")}
          </button>
        </form>
      </Show>
    </div>
  );
};
