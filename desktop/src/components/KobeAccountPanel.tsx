import { Component, createSignal, Show } from "solid-js";
import { Panel } from "./Panel";
import { Button } from "./Button";
import { Heading } from "./Typography";
import { toast } from "./Toast";
import { Icon } from "./Icon";
import { kobeUser, kobeLogin, kobeSignup, kobeLogout, kobeDisplayName } from "../store";
import { t } from "../i18n";

// 表单输入框统一样式(与 AccountDialog 一致的石质暗底深凹倒角)。
const INPUT =
  "h-[38px] px-[14px] rounded-none text-[13px] text-fg bg-sidebar shadow-input w-full " +
  "placeholder:text-faint transition-[box-shadow] duration-150 ease-app " +
  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent";

/**
 * KobeAccountPanel —— 登录 / 注册我们自己的 kobeMC 后端账号(区别于游戏内 MC 账号)。
 *
 * 这是连到 mc-server 的账号,解锁临时领域(realms)mod 同步等服务端能力,与游戏账号正交。
 * 会话存活在后端 ServerClient 的 cookie jar(进程内),故当前仅维持本次运行;重启需重登(MVP)。
 * 已登录时展示账号信息 + 退出;未登录时一个登录/注册切换表单。
 */
export const KobeAccountPanel: Component = () => {
  const [mode, setMode] = createSignal<"login" | "signup">("login");
  const [email, setEmail] = createSignal("");
  const [password, setPassword] = createSignal("");
  const [name, setName] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  function reset() {
    setEmail("");
    setPassword("");
    setName("");
    setError(null);
  }

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
      reset();
    } catch (err) {
      setError(t("kobe.errAuth", { err: String(err) }));
    } finally {
      setBusy(false);
    }
  }

  async function logout() {
    if (busy()) return;
    setBusy(true);
    try {
      await kobeLogout();
    } catch (err) {
      toast({ type: "error", message: t("kobe.errAuth", { err: String(err) }) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Panel variant="sunken" class="p-[20px]">
      <Heading size="sub" as="h2" class="mb-[6px]">
        {t("kobe.title")}
      </Heading>
      <p class="text-[12px] text-muted mb-[14px] leading-[1.55]">{t("kobe.subtitle")}</p>

      <Show
        when={kobeUser()}
        fallback={
          <form class="flex flex-col gap-[10px]" onSubmit={submit}>
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

            <Button variant="primary" type="submit" disabled={busy()} class="self-start">
              {busy()
                ? t("kobe.working")
                : mode() === "signup"
                  ? t("kobe.signupAction")
                  : t("kobe.loginAction")}
            </Button>

            <button
              type="button"
              class="self-start bg-transparent border-none p-0 text-[12px] text-accent cursor-pointer hover:underline"
              onClick={() => {
                setMode(mode() === "login" ? "signup" : "login");
                setError(null);
              }}
            >
              {mode() === "login" ? t("kobe.switchToSignup") : t("kobe.switchToLogin")}
            </button>
          </form>
        }
      >
        <div class="flex items-center gap-[12px]">
          <span class="grid place-items-center w-[40px] h-[40px] rounded-none bg-sidebar shadow-input text-accent shrink-0">
            <Icon name="user" size={20} />
          </span>
          <div class="flex flex-col min-w-0 flex-1">
            <span class="text-[14px] text-strong truncate">{kobeDisplayName(kobeUser()!)}</span>
            <Show when={kobeUser()!.email}>
              <span class="text-[12px] text-muted truncate">{kobeUser()!.email}</span>
            </Show>
          </div>
          <Button variant="ghost" disabled={busy()} onClick={() => void logout()} class="shrink-0">
            {t("kobe.logoutAction")}
          </Button>
        </div>
      </Show>
    </Panel>
  );
};
