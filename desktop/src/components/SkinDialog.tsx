import { useEffect, useRef, useState } from "react";
import { open as openFile } from "@tauri-apps/plugin-dialog";
import { Spinner } from "./Spinner";
import { ErrorState } from "./ErrorState";
import { Dialog } from "./Dialog";
import { Icon } from "./Icon";
import { Button } from "./Button";
import { Heading } from "./Typography";
import { toast } from "./Toast";
import { useAsync } from "../util/useAsync";
import { api } from "../ipc/api";
import { cached, invalidate } from "../ipc/cache";
import { t, useLang } from "../i18n";
import type { ProfileSkins, CapeInfo } from "../ipc/types";

/**
 * SkinDialog —— 微软账号的皮肤 / 披风管理弹窗。
 *
 * 读取 profile 端点(用账号的 Minecraft access token):展示当前皮肤预览 + 模型变体、
 * 披风列表(高亮使用中、点击切换/隐藏),并提供「上传皮肤」(本地 PNG → classic/slim)。
 * 仅微软正版账号有此 API,离线 / 外置账号不应打开本弹窗。
 */
export function SkinDialog(props: {
  uuid: string;
  username: string;
  onClose: () => void;
}) {
  useLang();
  const { data: fetched, loading, error, refetch } = useAsync<ProfileSkins>(
    () => cached(`skinProfile|${props.uuid}`, () => api.skinProfile(props.uuid)),
    [props.uuid],
  );
  // 本地乐观覆盖:上传/换披风返回的最新 profile 直接落地(等价 Solid resource 的 mutate),
  // 免一次网络往返;重试(reload)时清空,回到网络值。
  const [override, setOverride] = useState<ProfileSkins | null>(null);
  const profile = override ?? fetched;
  const reload = () => {
    setOverride(null);
    refetch();
  };
  const [busy, setBusy] = useState(false);
  const [variant, setVariant] = useState<"classic" | "slim">("classic");

  const closed = useRef(false);
  useEffect(() => {
    closed.current = false;
    return () => {
      closed.current = true;
    };
  }, []);

  // 当前生效的皮肤 / 披风。
  const activeSkin = profile ? (profile.skins?.find((s) => s.state === "ACTIVE") ?? profile.skins?.[0]) : undefined;
  const activeCapeId = profile?.capes?.find((c) => c.state === "ACTIVE")?.id ?? null;

  // 加载到 profile 后把上传默认变体同步成当前皮肤的变体。
  useEffect(() => {
    const v = profile?.skins?.find((s) => s.state === "ACTIVE")?.variant;
    if (v === "slim" || v === "classic") setVariant(v);
  }, [profile]);

  async function pickAndUpload() {
    if (busy) return;
    let path: string | null;
    try {
      const sel = await openFile({
        multiple: false,
        directory: false,
        filters: [{ name: "PNG", extensions: ["png"] }],
      });
      path = Array.isArray(sel) ? (sel[0] ?? null) : sel;
    } catch {
      return;
    }
    if (!path) return;
    setBusy(true);
    try {
      const updated = await api.skinUpload(props.uuid, path, variant);
      if (closed.current) return;
      setOverride(updated);
      invalidate(`skinProfile|${props.uuid}`); // 皮肤已变,丢弃旧缓存,重开取最新
      toast({ type: "success", message: t("skin.uploaded") });
    } catch (e) {
      if (closed.current) return;
      toast({ type: "error", message: t("skin.uploadFailed", { err: String(e) }) });
    } finally {
      if (!closed.current) setBusy(false);
    }
  }

  async function chooseCape(capeId: string | null) {
    if (busy) return;
    if (capeId === activeCapeId) return;
    setBusy(true);
    try {
      const updated = await api.skinSetCape(props.uuid, capeId);
      if (closed.current) return;
      setOverride(updated);
      invalidate(`skinProfile|${props.uuid}`); // 披风已变,丢弃旧缓存,重开取最新
      toast({ type: "success", message: t("skin.capeUpdated") });
    } catch (e) {
      if (closed.current) return;
      toast({ type: "error", message: t("skin.capeFailed", { err: String(e) }) });
    } finally {
      if (!closed.current) setBusy(false);
    }
  }

  const variantLabel = (v: string | undefined) =>
    v === "slim" ? t("skin.variantSlim") : t("skin.variantClassic");

  return (
    <Dialog
      open
      onClose={props.onClose}
      label={t("skin.title")}
      contentClass="w-[460px] max-w-[calc(100vw-48px)] focus-visible:outline-none"
    >
      <div className="flex items-center justify-between px-[18px] py-[14px] bg-titlebar border-b border-titlebar">
        <Heading size="sub">{t("skin.title")}</Heading>
        <button
          className="border-none bg-transparent text-muted cursor-pointer p-[5px] rounded-none flex items-center transition-colors duration-150 hover:bg-panel-2 hover:text-fg"
          onClick={props.onClose}
          aria-label={t("skin.close")}
        >
          <Icon name="close" size={16} />
        </button>
      </div>

      {loading ? (
        <div className="flex flex-col items-center gap-[10px] p-[40px] text-muted text-[13px]">
          <Spinner />
          <span>{t("skin.loading")}</span>
        </div>
      ) : error != null ? (
        <div className="p-[18px]">
          <ErrorState
            compact
            message={t("skin.loadFailed", { err: String(error) })}
            onRetry={() => reload()}
          />
        </div>
      ) : (
        <div className="p-[18px] flex flex-col gap-[18px]">
          {/* 当前皮肤预览 + 变体 + 上传 */}
          <div className="flex gap-[16px]">
            <div className="w-[96px] h-[128px] shrink-0 grid place-items-center bg-sidebar shadow-input overflow-hidden">
              {activeSkin?.url ? (
                <img
                  src={activeSkin.url}
                  alt={props.username}
                  className="w-full h-full object-contain [image-rendering:pixelated]"
                />
              ) : (
                <Icon name="user" size={36} className="text-faint" />
              )}
            </div>
            <div className="min-w-0 flex-1 flex flex-col gap-[10px]">
              <div>
                <div className="text-[14px] font-semibold text-fg truncate">{props.username}</div>
                <div className="text-[12px] text-muted mt-[2px]">
                  {t("skin.variant")}:{variantLabel(activeSkin?.variant)}
                </div>
              </div>
              <div className="flex flex-col gap-[6px]">
                <div className="text-[11px] text-muted">{t("skin.chooseVariant")}</div>
                <div className="flex gap-[8px]">
                  {(["classic", "slim"] as const).map((v) => (
                    <button
                      key={v}
                      className="px-[12px] py-[6px] text-[12px] rounded-none shadow-input bg-sidebar text-fg cursor-pointer transition-[box-shadow,background-color] duration-150 hover:bg-panel-2 aria-[pressed=true]:shadow-pressed aria-[pressed=true]:bg-panel-2 aria-[pressed=true]:text-accent"
                      aria-pressed={variant === v}
                      onClick={() => setVariant(v)}
                      disabled={busy}
                    >
                      {variantLabel(v)}
                    </button>
                  ))}
                </div>
              </div>
              <Button variant="primary" onClick={pickAndUpload} disabled={busy}>
                {busy ? t("skin.uploading") : t("skin.upload")}
              </Button>
            </div>
          </div>

          {/* 披风列表 */}
          <div className="flex flex-col gap-[8px]">
            <div className="text-[13px] font-semibold text-fg">{t("skin.capes")}</div>
            {(profile?.capes?.length ?? 0) > 0 ? (
              <div className="grid grid-cols-3 gap-[10px]">
                {/* 不戴披风 */}
                <CapeTile
                  label={t("skin.capeNone")}
                  active={activeCapeId === null}
                  disabled={busy}
                  onClick={() => void chooseCape(null)}
                />
                {profile!.capes!.map((cape: CapeInfo) => (
                  <CapeTile
                    key={cape.id}
                    label={cape.alias || t("skin.capes")}
                    url={cape.url}
                    active={cape.state === "ACTIVE"}
                    activeText={t("skin.capeActive")}
                    disabled={busy}
                    onClick={() => void chooseCape(cape.id)}
                  />
                ))}
              </div>
            ) : (
              <div className="text-[12px] text-muted">{t("skin.noCapes")}</div>
            )}
          </div>
        </div>
      )}
    </Dialog>
  );
}

function CapeTile(props: {
  label: string;
  url?: string;
  active: boolean;
  activeText?: string;
  disabled?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      className="flex flex-col items-center gap-[6px] p-[8px] rounded-none bg-panel cursor-pointer transition-[box-shadow,background-color] duration-150 hover:bg-panel-2 aria-[pressed=true]:shadow-raised aria-[pressed=true]:bg-panel-2 disabled:opacity-50 disabled:pointer-events-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
      aria-pressed={props.active}
      onClick={props.onClick}
      disabled={props.disabled}
    >
      <span className="w-[48px] h-[64px] grid place-items-center bg-sidebar shadow-input overflow-hidden">
        {props.url ? (
          <img
            src={props.url}
            alt={props.label}
            className="w-full h-full object-contain [image-rendering:pixelated]"
          />
        ) : (
          <Icon name="close" size={18} className="text-faint" />
        )}
      </span>
      <span className="text-[11px] text-fg text-center truncate max-w-full leading-tight">{props.label}</span>
      {props.active && <span className="text-[10px] text-accent leading-none">{props.activeText ?? "✓"}</span>}
    </button>
  );
}

export default SkinDialog;
