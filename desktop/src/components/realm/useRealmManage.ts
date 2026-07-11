import { useEffect, useRef, useState } from "react";
import { toast } from "../Toast";
import { api, onRealmSyncProgress } from "../../ipc/api";
import { activeRoot, refreshInstances, refreshFriends, useAppStore } from "../../store";
import { useAsync } from "../../util/useAsync";
import { t } from "../../i18n";
import type { InstanceSummary, SyncReport } from "../../ipc/bindings";

/**
 * useRealmManage —— 领域管理页的全部状态与动作(summary/成员/差异计划/同步/
 * 角色/邀请/退出解散)。原样收拢自 RealmManage 内联逻辑,行为不变。
 */
export function useRealmManage(instance: InstanceSummary, onChanged?: () => void) {
  const rid = instance.realm!.realm_id;
  const myId = useAppStore((s) => s.kobeUser?.id);
  // 好友列表来自 store(单一真相 + 连续 30s 轮询),用于「邀请好友」与成员的好友标记。
  const friendsList = useAppStore((s) => s.friends);
  const socialOn = useAppStore((s) => s.socialEnabled);
  const kobeSignedIn = useAppStore((s) => s.kobeUser !== null);

  const { data: summary, refetch: refetchSummary } = useAsync(() => api.realmGet(rid), [rid]);
  const { data: members, loading: membersLoading, refetch: refetchMembers } = useAsync(() => api.realmMembers(rid), [rid]);
  const role = summary?.role ?? instance.realm!.role;
  const isOwner = role === "owner";
  const canPush = role === "owner" || role === "admin";

  // 面板打开时主动拉一次好友,保证 store 缓存新鲜(后续在线状态/活动由 store 轮询维护)。
  useEffect(() => {
    void refreshFriends();
  }, []);
  const memberIds = new Set((members ?? []).map((m) => m.user_id));
  const friendIds = new Set((friendsList ?? []).map((f) => f.id));
  // 成员 → 好友映射(用于在成员行展示在线/活动);非好友成员取不到则无 pip。
  const friendById = new Map((friendsList ?? []).map((f) => [f.id, f] as const));
  // 折叠头里的「在线 N」:统计同时是好友且在线的成员数。
  const onlineMemberCount = (members ?? []).filter((m) => friendById.get(m.user_id)?.online).length;

  // 在线好友优先,其次按用户名排序,便于优先邀请在线好友。
  const sortedFriends = [...(friendsList ?? [])].sort((a, b) => {
    if (!!a.online !== !!b.online) return a.online ? -1 : 1;
    return (a.username || a.id).localeCompare(b.username || b.id);
  });

  const [removeExtras, setRemoveExtras] = useState(false);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<{ current: number; total: number } | null>(null);
  const [confirmKind, setConfirmKind] = useState<"leave" | "disband" | null>(null);

  // 成员可折叠(默认展开);邀请不折叠,直接一个搜索框,输入才出名字。
  const [membersOpen, setMembersOpen] = useState(true);
  const [inviteQuery, setInviteQuery] = useState("");
  // 邀请:在自己的好友里按用户名过滤(空输入不出名单),在线优先。
  const inviteMatches = (() => {
    const q = inviteQuery.trim().toLowerCase();
    if (!q) return [];
    return sortedFriends.filter((f) => (f.username || f.id).toLowerCase().includes(q));
  })();

  // 自动检测差异:随 (领域, 清单版本) 自动重算(summary 未就绪时不打后端)。
  const planKey = summary ? { rid, iid: instance.id, mv: summary.manifest_version } : null;
  const { data: plan, refetch: refetchPlan } = useAsync(
    () => (planKey ? api.realmPlanSync(planKey.rid, activeRoot(), planKey.iid) : Promise.resolve(undefined)),
    [rid, instance.id, summary?.manifest_version],
  );

  useEffect(() => onRealmSyncProgress((p) => setProgress({ current: p.current, total: p.total })), []);

  function fail(e: unknown) {
    toast({ type: "error", message: t("realm.opError", { err: String(e) }) });
  }

  // 纯新增差异自动同步;有移除(破坏性)留给显式确认。去重避免死循环。
  const autoKeyRef = useRef("");
  useEffect(() => {
    if (!plan || busy) return;
    const key = `${rid}:${plan.version}`;
    if (plan.download.length > 0 && plan.remove.length === 0 && autoKeyRef.current !== key) {
      autoKeyRef.current = key;
      void runSync(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [plan, busy, rid]);

  async function runSync(remove: boolean) {
    if (busy) return;
    setBusy(true);
    setProgress({ current: 0, total: 0 });
    try {
      const report: SyncReport = await api.realmSync(rid, activeRoot(), instance.id, remove);
      refreshInstances();
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
    setBusy(true);
    try {
      const version = await api.realmPushManifest(
        rid,
        activeRoot(),
        instance.id,
        instance.mc_version,
        instance.loader ?? "vanilla",
        instance.loader_version ?? null,
      );
      await refetchSummary();
      await refetchPlan();
      toast({ type: "success", message: t("realm.pushDone", { version }) });
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  }

  async function copyCode() {
    const c = summary?.code ?? instance.realm!.code;
    if (!c) return;
    try {
      await navigator.clipboard.writeText(c);
      toast({ type: "success", message: t("realm.copied") });
    } catch (e) {
      fail(e);
    }
  }

  async function setRole(uid: string, r: string) {
    setBusy(true);
    try {
      await api.realmSetRole(rid, uid, r);
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
      await api.realmRemoveMember(rid, uid);
      await refetchMembers();
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  }

  async function invite(uid: string) {
    if (busy) return;
    setBusy(true);
    try {
      await api.realmInvite(rid, uid);
      await refetchMembers();
      toast({ type: "success", message: t("realm.inviteDone") });
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  }

  async function doLeave() {
    if (!myId) return;
    setConfirmKind(null);
    setBusy(true);
    try {
      await api.realmLeave(rid, myId, activeRoot(), instance.id);
      refreshInstances();
      onChanged?.();
    } catch (e) {
      fail(e);
      setBusy(false);
    }
  }

  async function doDisband() {
    setConfirmKind(null);
    setBusy(true);
    try {
      await api.realmDisband(rid, activeRoot(), instance.id);
      refreshInstances();
      onChanged?.();
    } catch (e) {
      fail(e);
      setBusy(false);
    }
  }

  const code = summary?.code ?? instance.realm!.code ?? "";
  const realmName = summary?.name ?? instance.realm!.name ?? t("realm.title");
  // 我自己的同步进度(members 已含每人 synced_version;undefined = 成员列表未就绪)。
  const mySynced = (members ?? []).find((m) => m.user_id === myId)?.synced_version;
  return {
    rid, myId, socialOn, kobeSignedIn,
    summary, members, membersLoading, role, isOwner, canPush,
    memberIds, friendIds, friendById, onlineMemberCount, sortedFriends,
    removeExtras, setRemoveExtras, busy, progress,
    confirmKind, setConfirmKind, membersOpen, setMembersOpen,
    inviteQuery, setInviteQuery, inviteMatches, plan,
    runSync, pushManifest, copyCode, setRole, removeMember, invite,
    doLeave, doDisband, code, realmName, mySynced,
  };
}
