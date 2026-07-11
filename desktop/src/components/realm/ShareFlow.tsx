import { useEffect, useState } from "react";
import { Button } from "../Button";
import { Panel } from "../Panel";
import { Heading } from "../Typography";
import { Dialog } from "../Dialog";
import { Select } from "../Select";
import { Tag } from "../Tag";
import { toast } from "../Toast";
import { api, onRealmSyncProgress } from "../../ipc/api";
import { activeRoot, refreshInstances, useAppStore } from "../../store";
import { t } from "../../i18n";
import type { InstanceSummary, SyncReport } from "../../ipc/bindings";
import { roleLabel } from "./shared";

/* ---------- 非领域实例:分享入口 ---------- */

export function ShareEntry({ instance, onChanged }: { instance: InstanceSummary; onChanged?: () => void }) {
  const [open, setOpen] = useState(false);
  return (
    <Panel variant="sunken" className="p-[16px] flex items-center justify-between gap-[12px] flex-wrap">
      <div className="min-w-0">
        <div className="flex items-center gap-[8px]">
          <Heading size="sub" as="h3" className="m-0 text-[14px]">
            {t("realm.title")}
          </Heading>
        </div>
        <p className="text-[12px] text-muted mt-[4px] leading-[1.6]">{t("realm.shareHint")}</p>
      </div>
      <Button variant="ghost" onClick={() => setOpen(true)}>
        {t("realm.shareAction")}
      </Button>
      <ShareDialog instance={instance} open={open} onClose={() => setOpen(false)} onShared={onChanged} />
    </Panel>
  );
}

function ShareDialog({
  instance,
  open,
  onClose,
  onShared,
}: {
  instance: InstanceSummary;
  open: boolean;
  onClose: () => void;
  onShared?: () => void;
}) {
  const [name, setName] = useState(instance.name);
  const [expiry, setExpiry] = useState("0");
  const [busy, setBusy] = useState(false);
  const kobeSignedIn = useAppStore((s) => s.kobeUser !== null);

  async function submit() {
    if (busy || !name.trim()) return;
    if (!kobeSignedIn) {
      toast({ type: "error", message: t("realm.needLogin") });
      return;
    }
    setBusy(true);
    try {
      const secs = parseInt(expiry, 10) || 0;
      const r = await api.realmCreate(
        activeRoot(),
        instance.id,
        name.trim(),
        instance.mc_version,
        instance.loader ?? "vanilla",
        instance.loader_version ?? null,
        secs > 0 ? secs : null,
      );
      toast({ type: "success", message: t("realm.createdToast", { name: r.name }) });
      onClose();
      onShared?.();
    } catch (e) {
      toast({ type: "error", message: t("realm.opError", { err: String(e) }) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onClose={onClose} label={t("realm.shareTitle")}>
      <div className="p-[20px] flex flex-col gap-[14px]">
        <Heading size="sub" as="h2" className="m-0">
          {t("realm.shareTitle")}
        </Heading>
        {!kobeSignedIn && <p className="text-[12px] text-danger-text">{t("realm.needLogin")}</p>}
        <label className="flex flex-col gap-[6px]">
          <span className="text-[12px] text-muted">{t("realm.nameLabel")}</span>
          <input
            className="h-[38px] px-[14px] rounded-none text-[13px] text-fg bg-sidebar shadow-input w-full placeholder:text-faint focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
            type="text"
            value={name}
            onChange={(e) => setName(e.currentTarget.value)}
          />
        </label>
        <label className="flex flex-col gap-[6px]">
          <span className="text-[12px] text-muted">{t("realm.expiry")}</span>
          <Select
            value={expiry}
            onChange={setExpiry}
            options={[
              { value: "0", label: t("realm.expiryNever") },
              { value: "86400", label: t("realm.expiry1d") },
              { value: "604800", label: t("realm.expiry7d") },
              { value: "2592000", label: t("realm.expiry30d") },
            ]}
          />
        </label>
        <div className="flex justify-end gap-[8px] mt-[4px]">
          <Button variant="ghost" onClick={onClose}>
            {t("realm.cancel")}
          </Button>
          <Button variant="primary" disabled={busy || !name.trim()} onClick={() => void submit()}>
            {busy ? t("realm.creating") : t("realm.shareAction")}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

/* ---------- pending:开始同步 ---------- */

export function BeginEntry({ instance, onChanged }: { instance: InstanceSummary; onChanged?: () => void }) {
  const r = instance.realm!;
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<{ current: number; total: number } | null>(null);
  useEffect(() => onRealmSyncProgress((p) => setProgress({ current: p.current, total: p.total })), []);

  async function begin() {
    if (busy) return;
    setBusy(true);
    setProgress({ current: 0, total: 0 });
    try {
      const report: SyncReport = await api.realmBegin(r.realm_id, activeRoot(), instance.id);
      refreshInstances();
      onChanged?.();
      toast({
        type: report.failed.length ? "error" : "success",
        message: report.failed.length ? t("realm.syncFailed", { count: report.failed.length }) : t("realm.beginDone"),
      });
    } catch (e) {
      toast({ type: "error", message: t("realm.opError", { err: String(e) }) });
    } finally {
      setBusy(false);
      setProgress(null);
    }
  }

  return (
    <Panel variant="sunken" className="p-[20px] flex flex-col gap-[12px]">
      <div className="flex items-center gap-[8px] flex-wrap">
        <Heading size="sub" as="h3" className="m-0 text-[14px]">
          {r.name || t("realm.title")}
        </Heading>
        <Tag>{roleLabel(r.role)}</Tag>
        {r.code && <span className="font-mono text-[12px] text-accent tracking-[0.12em]">{r.code}</span>}
      </div>
      <p className="text-[12px] text-muted leading-[1.6]">{t("realm.beginHint")}</p>
      <Button variant="primary" className="self-start" disabled={busy} onClick={() => void begin()}>
        {busy ? t("realm.syncing") : t("realm.beginAction")}
      </Button>
      {progress && (
        <div className="h-[6px] w-full bg-window shadow-input rounded-none overflow-hidden">
          <div
            className="h-full bg-accent transition-[width] duration-150 ease-app"
            style={{ width: `${progress.total > 0 ? Math.round((progress.current / progress.total) * 100) : 8}%` }}
          />
        </div>
      )}
    </Panel>
  );
}
