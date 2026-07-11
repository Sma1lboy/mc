import { useLang } from "../i18n";
import type { InstanceSummary } from "../ipc/bindings";
import { ShareEntry, BeginEntry } from "./realm/ShareFlow";
import { RealmManage } from "./realm/RealmManage";

export function RealmPanel({ instance, onChanged }: { instance: InstanceSummary; onChanged?: () => void }) {
  useLang();
  const realm = instance.realm;
  return (
    <div className="px-[28px] py-[14px]">
      {!realm ? (
        <ShareEntry instance={instance} onChanged={onChanged} />
      ) : instance.installed ? (
        <RealmManage instance={instance} onChanged={onChanged} />
      ) : (
        <BeginEntry instance={instance} onChanged={onChanged} />
      )}
    </div>
  );
}

export default RealmPanel;
