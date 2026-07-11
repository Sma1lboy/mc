import clsx from "clsx";
import { Button } from "../Button";
import { Panel } from "../Panel";
import { Heading } from "../Typography";
import { Dialog } from "../Dialog";
import { Spinner } from "../Spinner";
import { Tag } from "../Tag";
import { Toggle } from "../Toggle";
import { setCurrentPage } from "../../store";
import { t } from "../../i18n";
import type { InstanceSummary, RealmMember } from "../../ipc/bindings";
import { Caret, PersonTile, roleLabel } from "./shared";
import { avatarTone, avatarInitial } from "../../util/avatar";
import { LobbyBlock } from "./LobbyBlock";
import { useRealmManage } from "./useRealmManage";

export function RealmManage({ instance, onChanged }: { instance: InstanceSummary; onChanged?: () => void }) {
  const {
    rid, socialOn, kobeSignedIn, myId,
    summary, members, membersLoading, role, isOwner, canPush,
    memberIds, friendIds, friendById, onlineMemberCount,
    removeExtras, setRemoveExtras, busy, progress,
    confirmKind, setConfirmKind, membersOpen, setMembersOpen,
    inviteQuery, setInviteQuery, inviteMatches, plan,
    runSync, pushManifest, copyCode, setRole, removeMember, invite,
    doLeave, doDisband, code, realmName, mySynced,
  } = useRealmManage(instance, onChanged);

  return (
    <div className="flex flex-col gap-[12px]">
      {/* 头部:领域身份(头像 + 名称 + 角色 + 版本·成员数副行)+ 可复制加入码 chip */}
      <Panel variant="sunken" className="p-[16px] flex items-center justify-between gap-[12px] flex-wrap">
        <div className="flex items-center gap-[10px] min-w-0">
          <PersonTile name={realmName} />
          <div className="flex flex-col min-w-0 gap-[2px]">
            <div className="flex items-center gap-[8px] min-w-0">
              <Heading size="sub" as="h3" className="m-0 text-[14px] truncate">
                {realmName}
              </Heading>
              <Tag>{roleLabel(role)}</Tag>
            </div>
            {summary && (
              <span className="text-[11px] text-muted tabular-nums">
                {t("realm.manifestVersion", { version: summary.manifest_version })}
                {mySynced != null && (
                  <> · {mySynced > 0 ? t("realm.syncedTo", { version: mySynced }) : t("realm.notSynced")}</>
                )}
                {" · "}
                {t("realm.memberCount", { n: (members ?? []).length })}
              </span>
            )}
          </div>
        </div>
        {code && (
          <button
            type="button"
            className="inline-flex items-center gap-[8px] font-mono text-[14px] text-accent tracking-[0.16em] tabular-nums bg-window shadow-input px-[10px] py-[5px] cursor-pointer hover:brightness-110 [-webkit-app-region:no-drag]"
            title={t("realm.copyCode")}
            onClick={() => void copyCode()}
          >
            <span>{code}</span>
            <svg className="w-[13px] h-[13px] shrink-0 opacity-70" viewBox="0 0 24 24" fill="none" aria-hidden="true">
              <rect x="9" y="9" width="11" height="11" rx="1.5" stroke="currentColor" strokeWidth="1.8" />
              <path d="M5 15V5.5A1.5 1.5 0 0 1 6.5 4H15" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
            </svg>
          </button>
        )}
      </Panel>

      {/* 联机大厅(EasyTier 虚拟局域网):社交开启时出现 */}
      {socialOn && <LobbyBlock realmId={rid} instanceId={instance.id} />}

      {/* 同步状态(自动检测 + 自动同步;破坏性需确认) */}
      <Panel variant="sunken" className="p-[16px] flex flex-col gap-[10px]">
        {busy && <p className="text-[13px] text-accent">{t("realm.syncing")}</p>}
        {!busy && plan && (
          plan.download.length === 0 && plan.remove.length === 0 ? (
            <p className="text-[13px] text-accent">{t("realm.planUpToDate")}</p>
          ) : (
            <>
              {/* 一句话状态:领域有更新,点下面同步。 */}
              <p className="text-[13px] text-strong">{t("realm.syncPending")}</p>
              <div className="flex flex-wrap gap-[8px] text-[12px]">
                {plan.download.length > 0 && (
                  <span className="bg-window shadow-input px-[8px] py-[3px]">{t("realm.planDownload", { count: plan.download.length })}</span>
                )}
                {plan.remove.length > 0 && (
                  <span className="bg-window shadow-input px-[8px] py-[3px]">{t("realm.planRemove", { count: plan.remove.length })}</span>
                )}
                {plan.manual.length > 0 && (
                  <span className="bg-window shadow-input px-[8px] py-[3px]">{t("realm.planManual", { count: plan.manual.length })}</span>
                )}
              </div>
              {/* 移除开关:仅当有「领域之外的 mod」可移除时出现。 */}
              {plan.remove.length > 0 && (
                <div className="flex items-center justify-between text-[13px] text-fg mt-[4px]">
                  <div className="flex flex-col gap-[2px] min-w-0 pr-[12px]">
                    <span>{t("realm.removeExtras")}</span>
                    <span className="text-[11px] text-muted">{t("realm.removeExtrasHint")}</span>
                  </div>
                  <Toggle checked={removeExtras} onChange={setRemoveExtras} disabled={busy} />
                </div>
              )}
              {/* 同步按钮:只要有待同步项就显示(不再只在有移除时出现)。 */}
              <Button variant="primary" className="self-start mt-[4px]" disabled={busy} onClick={() => void runSync(removeExtras)}>
                {t("realm.applyChanges")}
              </Button>
              {plan.manual.length > 0 && (
                <div className="text-[12px] text-muted mt-[4px]">
                  <div className="mb-[4px]">{t("realm.manualList")}</div>
                  <ul className="list-disc pl-[18px] flex flex-col gap-[2px]">
                    {plan.manual.map((f) => (
                      <li key={f.path} className="break-all text-faint">
                        {f.path.replace(/^mods\//, "")}
                      </li>
                    ))}
                  </ul>
                </div>
              )}
            </>
          )
        )}
        {progress && (
          <div className="h-[6px] w-full bg-window shadow-input rounded-none overflow-hidden">
            <div
              className="h-full bg-accent transition-[width] duration-150 ease-app"
              style={{ width: `${progress.total > 0 ? Math.round((progress.current / progress.total) * 100) : 8}%` }}
            />
          </div>
        )}
        {canPush && (
          <div className="pt-[10px] mt-[2px] border-t border-titlebar flex items-center justify-between gap-[10px] flex-wrap">
            <span className="text-[12px] text-muted leading-[1.6] min-w-0">{t("realm.pushHint")}</span>
            <Button variant="ghost" disabled={busy} onClick={() => void pushManifest()}>
              {busy ? t("realm.pushing") : t("realm.pushAction")}
            </Button>
          </div>
        )}
      </Panel>

      {/* 成员(可折叠;上移到邀请之前) */}
      <Panel variant="sunken" className="p-0 overflow-hidden">
        <button
          type="button"
          className="w-full flex items-center gap-[8px] px-[16px] py-[12px] bg-transparent border-none cursor-pointer text-left hover:bg-panel-2 transition-[background-color] duration-[var(--dur)] ease-app"
          onClick={() => setMembersOpen((o) => !o)}
        >
          <Caret open={membersOpen} />
          <span className="text-[12px] text-sub font-display tracking-[0.5px]">{t("realm.members")}</span>
          <span className="text-[11px] text-faint tabular-nums">{(members ?? []).length}</span>
          {onlineMemberCount > 0 && (
            <span className="text-[11px] text-accent tabular-nums">{t("friend.onlineCount", { n: onlineMemberCount })}</span>
          )}
          {/* 折叠时:在标题行右侧叠展成员头像(peek)。 */}
          {!membersOpen && (
            <>
              <span className="flex-1" />
              <span className="flex items-center -space-x-[6px] pr-[2px]">
                {(members ?? []).slice(0, 6).map((m) => {
                  const nm = m.username || m.user_id.slice(0, 8);
                  return (
                    <span
                      key={m.user_id}
                      className="w-[20px] h-[20px] grid place-items-center shadow-raised font-display text-[10px] text-[#1a1b12] ring-2 ring-panel"
                      style={{ backgroundColor: avatarTone(nm) }}
                      aria-hidden="true"
                    >
                      {avatarInitial(nm)}
                    </span>
                  );
                })}
                {(members ?? []).length > 6 && (
                  <span className="text-[11px] text-faint pl-[10px] tabular-nums">+{(members ?? []).length - 6}</span>
                )}
              </span>
            </>
          )}
        </button>
        {membersOpen && (
          <div className="px-[8px] pb-[10px]">
            {membersLoading ? (
              <div className="flex justify-center p-[12px]">
                <Spinner />
              </div>
            ) : (
              (members ?? []).map((m: RealmMember) => {
                const nm = m.username || m.user_id.slice(0, 8);
                // 同时是好友的成员:借好友列表(store 轮询维护在线/活动)展示在线状态。
                const friend = friendById.get(m.user_id);
                const isFriend = friend !== undefined;
                return (
                  <div
                    key={m.user_id}
                    className="flex items-center gap-[10px] px-[8px] py-[6px] group hover:bg-panel-2 transition-[background-color] duration-[var(--dur)] ease-app"
                  >
                    <PersonTile name={nm} online={friend?.online} pip={isFriend} />
                    <div className="flex flex-col min-w-0 flex-1">
                      <span className="text-[13px] text-fg truncate">
                        {nm}
                        {m.user_id === myId && <span className="text-muted"> {t("realm.you")}</span>}
                      </span>
                      {friend?.online && (
                        <span className="text-[11px] text-accent truncate">
                          {friend.activity ? t("friend.playing", { name: friend.activity ?? "" }) : t("friend.idle")}
                        </span>
                      )}
                      <span className="text-[11px] text-faint truncate">
                        {m.synced_version > 0 ? t("realm.syncedTo", { version: m.synced_version }) : t("realm.notSynced")}
                      </span>
                    </div>
                    {m.user_id !== myId && friendIds.has(m.user_id) && <Tag>{t("realm.friendTag")}</Tag>}
                    <Tag>{roleLabel(m.role)}</Tag>
                    {isOwner && m.role !== "owner" && (
                      <div className="flex items-center gap-[6px] shrink-0">
                        <button
                          className="text-[12px] text-muted hover:text-fg bg-transparent border-none cursor-pointer disabled:opacity-50"
                          disabled={busy}
                          onClick={() => void setRole(m.user_id, m.role === "member" ? "admin" : "member")}
                        >
                          {m.role === "member" ? t("realm.promote") : t("realm.demote")}
                        </button>
                        <button
                          className="text-[12px] text-danger-text hover:underline bg-transparent border-none cursor-pointer disabled:opacity-50"
                          disabled={busy}
                          onClick={() => void removeMember(m.user_id)}
                        >
                          {t("realm.removeMember")}
                        </button>
                      </div>
                    )}
                  </div>
                );
              })
            )}
          </div>
        )}
      </Panel>

      {/* 邀请好友(owner/admin · 社交开启时):不折叠,直接一个搜索框,输入才出名字 */}
      {canPush && socialOn && kobeSignedIn && (
        <Panel variant="sunken" className="p-[16px] flex flex-col gap-[8px]">
          <span className="text-[12px] text-sub font-display tracking-[0.5px]">{t("realm.inviteTitle")}</span>
          <input
            className="h-[32px] px-[10px] rounded-none text-[13px] text-fg bg-sidebar shadow-input w-full placeholder:text-faint focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
            type="text"
            placeholder={t("realm.inviteSearchPlaceholder")}
            value={inviteQuery}
            onChange={(e) => setInviteQuery(e.currentTarget.value)}
          />
          {inviteQuery.trim().length > 0 &&
            (inviteMatches.length > 0 ? (
              <div className="flex flex-col gap-[2px] max-h-[200px] overflow-y-auto">
                {inviteMatches.map((f) => {
                  const nm = f.username || f.id.slice(0, 8);
                  return (
                    <div key={f.id} className={clsx("flex items-center gap-[10px] px-[8px] py-[6px]", { "opacity-70": !f.online })}>
                      <PersonTile name={nm} online={f.online} pip />
                      <div className="flex flex-col min-w-0 flex-1">
                        <span className="text-[13px] text-fg truncate">{nm}</span>
                        <span className="text-[11px] text-faint truncate">
                          {f.online
                            ? f.activity
                              ? t("friend.playing", { name: f.activity })
                              : t("friend.idle")
                            : t("friend.offline")}
                        </span>
                      </div>
                      {memberIds.has(f.id) ? (
                        <span className="text-[12px] text-faint">{t("realm.invited")}</span>
                      ) : (
                        <button
                          className="text-[12px] text-accent hover:underline bg-transparent border-none cursor-pointer disabled:opacity-50"
                          disabled={busy}
                          onClick={() => void invite(f.id)}
                        >
                          {t("realm.invite")}
                        </button>
                      )}
                    </div>
                  );
                })}
              </div>
            ) : (
              <p className="text-[12px] text-faint px-[2px]">{t("friend.noResults")}</p>
            ))}
        </Panel>
      )}

      {/* 退出 / 解散 */}
      <div className="flex justify-end">
        <Button variant="danger" disabled={busy} onClick={() => setConfirmKind(isOwner ? "disband" : "leave")}>
          {isOwner ? t("realm.disband") : t("realm.leave")}
        </Button>
      </div>

      <Dialog
        open={confirmKind !== null}
        onClose={() => setConfirmKind(null)}
        label={confirmKind === "disband" ? t("realm.disband") : t("realm.leave")}
      >
        <div className="p-[20px] flex flex-col gap-[16px]">
          <p className="text-[14px] text-fg leading-[1.6]">
            {confirmKind === "disband"
              ? t("realm.confirmDisband", { name: summary?.name ?? instance.name })
              : t("realm.confirmLeave", { name: summary?.name ?? instance.name })}
          </p>
          <div className="flex justify-end gap-[8px]">
            <Button variant="ghost" onClick={() => setConfirmKind(null)}>
              {t("realm.cancel")}
            </Button>
            <Button variant="danger" disabled={busy} onClick={() => void (confirmKind === "disband" ? doDisband() : doLeave())}>
              {confirmKind === "disband" ? t("realm.disband") : t("realm.leave")}
            </Button>
          </div>
        </div>
      </Dialog>

      {/* 未登录兜底(理论上能进到这里说明实例有 realm 绑定但会话过期) */}
      {!kobeSignedIn && (
        <p className="text-[12px] text-muted">
          {t("realm.needLogin")}{" "}
          <button
            className="text-accent hover:underline bg-transparent border-none cursor-pointer p-0"
            onClick={() => setCurrentPage("settings")}
          >
            {t("realm.goLogin")}
          </button>
        </p>
      )}
    </div>
  );
}
