import { Component, createSignal, onCleanup, onMount, Show } from "solid-js";
import { Button } from "./Button";
import { Icon } from "./Icon";
import { FriendsSection } from "./FriendsSection";
import { LinkedAccountsSection } from "./LinkedAccountsSection";
import { kobeUser, kobeLogin, kobeSignup, kobeLogout, kobeDisplayName } from "../store";
import { t } from "../i18n";

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
        class="inline-flex items-center gap-[7px] h-[26px] px-[10px] bg-panel-2 shadow-sunken text-[12px] text-fg cursor-pointer hover:brightness-110 transition-[filter] duration-150"
        onClick={() => setOpen((o) => !o)}
        title={t("kobe.title")}
      >
        <span class="grid place-items-center w-[16px] h-[16px] text-accent shrink-0">
          <Icon name="user" size={14} />
        </span>
        <span class="max-w-[120px] truncate">
          {kobeUser() ? kobeDisplayName(kobeUser()!) : t("kobe.loginAction")}
        </span>
      </button>

      <Show when={open()}>
        <div class="absolute right-0 top-[calc(100%+6px)] w-[300px] bg-panel border border-titlebar shadow-raised rounded-none z-[200] p-[16px]">
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

            <FriendsSection />
          </Show>
        </div>
      </Show>
    </div>
  );
};

const LoginForm: Component<{ onDone: () => void }> = (props) => {
  const [mode, setMode] = createSignal<"login" | "signup">("login");
  const [email, setEmail] = createSignal("");
  const [password, setPassword] = createSignal("");
  const [name, setName] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  async function submit(e: Event) {
    e.preventDefault();
    if (busy()) return;
    setError(null);
    const mail = email().trim();
    if (!mail || !password()) {
      setError(t("kobe.errEmptyCreds"));
      return;
    }
    setBusy(true);
    try {
      if (mode() === "signup") {
        await kobeSignup(mail, password(), name().trim() || mail.split("@")[0]);
      } else {
        await kobeLogin(mail, password());
      }
      props.onDone();
    } catch (err) {
      setError(t("kobe.errAuth", { err: String(err) }));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form class="flex flex-col gap-[10px]" onSubmit={submit}>
      <div class="text-[13px] text-strong font-display mb-[2px]">{t("kobe.title")}</div>
      <Show when={mode() === "signup"}>
        <input
          class={INPUT}
          type="text"
          autocomplete="nickname"
          placeholder={t("kobe.namePlaceholder")}
          value={name()}
          onInput={(ev) => setName(ev.currentTarget.value)}
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
  );
};
