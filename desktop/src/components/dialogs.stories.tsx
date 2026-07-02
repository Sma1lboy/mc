import { useEffect, useState, type ReactNode } from "react";
import type { Story, StoryDefault } from "@ladle/react";
import { BlockedFilesDialog, CrashDialog, NewInstanceDialog, ImportModpackDialog } from ".";
import { ProjectDetailPanel } from "./ProjectDetailPanel";
import { setCrashReport, type CrashReport } from "../store";
import { ToastContainer } from "./Toast";

/**
 * 模态对话框在 Ladle 里若强制 open 且 onClose 无操作,会因焦点陷阱 / 遮罩而卡住整页
 * (无法关闭 → 无法继续点)。这个 harness 用本地 open 态让故事里的对话框可关可重开:
 * 初始打开可看,点取消 / 遮罩即关,关后出现「打开对话框」按钮再开。
 */
function DialogHarness({ children }: { children: (open: boolean, close: () => void) => ReactNode }) {
  const [open, setOpen] = useState(true);
  return (
    <>
      {!open && (
        <button
          type="button"
          onClick={() => setOpen(true)}
          className="px-[14px] py-[8px] rounded-none bg-accent text-accent-text shadow-raised text-[13px] font-medium cursor-pointer"
        >
          打开对话框
        </button>
      )}
      {children(open, () => setOpen(false))}
    </>
  );
}

/* ============================================================================
 * dialogs.stories —— 对话框 / 详情面板的隔离预览(Ladle)。
 *
 * 这些组件接后端(创建实例 / 导入 / 安装版本 / 打开日志)。Ladle 里无 Tauri,故:
 *   · BlockedFilesDialog —— 纯 props,直接喂 mock 完整渲染。
 *   · CrashDialog —— 无 props,读 store.crashReport;故事里用 setCrashReport 种一份
 *     mock 崩溃报告再渲染(关闭按钮会清 store,重挂载即回填)。
 *   · NewInstanceDialog / ImportModpackDialog —— 打开时会 api.* 拉版本 / 导入,均
 *     .catch 兜底不崩;这里只看**打开态的表单外壳**,动作是 no-op。
 *   · ProjectDetailPanel —— useAsync(api.modrinthProject/Versions) 在无后端下 reject
 *     → 落到「暂无简介 / 暂无版本」的优雅空壳(而非崩溃),看的是头部 + 版本区骨架。
 * 真正依赖运行时抓不到的内部数据、无法有意义呈现的复杂对话框已跳过(见文件末注释)。
 * ========================================================================== */

export default {
  title: "Components / Dialogs",
} satisfies StoryDefault;

// —— BlockedFilesDialog(纯 props)—————————————————————————————————————

export const Blocked: Story = () => (
  <DialogHarness>
    {(open, close) =>
      open && (
        <BlockedFilesDialog
          instanceId="tech-survival"
          blocked={[
            {
              name: "Optifine_1.20.1_HD_U_I6.jar",
              website_url: "https://optifine.net",
              target_dir: "mods",
              required: true,
            },
            {
              name: "SomeCurseForgeMod-3.2.1.jar",
              website_url: "https://www.curseforge.com/minecraft/mc-mods/some-mod",
              target_dir: "mods",
              required: false,
            },
          ]}
          skipped={["client-only-config.zip", "readme.txt"]}
          onClose={close}
        />
      )
    }
  </DialogHarness>
);
Blocked.storyName = "BlockedFilesDialog · 需手动下载的文件";

// —— CrashDialog(store 驱动:种一份 mock 崩溃报告)———————————————————————

const MOCK_CRASH: CrashReport = {
  id: "tech-survival",
  name: "科技生存 1.20.1",
  mcVersion: "1.20.1",
  loader: "fabric",
  loaderVersion: "0.15.7",
  code: 1,
  category: "out_of_memory",
  reason: "游戏因内存不足退出(Java heap space)。",
  suggestions: ["在设置里把内存分配调高到 6G 以上", "移除占用显存较大的高清材质包"],
  matched: "java.lang.OutOfMemoryError: Java heap space",
  logTail:
    "[12:04:51] [Render thread/ERROR]: Reported exception thrown!\njava.lang.OutOfMemoryError: Java heap space\n\tat net.minecraft...\n[12:04:52] [Render thread/INFO]: Stopping!",
};

export const Crash: Story = () => {
  // CrashDialog 无 props、只读 store;挂载即种入 mock,卸载时清理。
  useEffect(() => {
    setCrashReport(MOCK_CRASH);
    return () => setCrashReport(null);
  }, []);
  return (
    <>
      <CrashDialog />
      <ToastContainer />
    </>
  );
};
Crash.storyName = "CrashDialog · 崩溃诊断(内存不足)";

// —— NewInstanceDialog(打开态表单外壳)————————————————————————————————

export const NewInstance: Story = () => (
  <DialogHarness>{(open, close) => <NewInstanceDialog open={open} onClose={close} />}</DialogHarness>
);
NewInstance.storyName = "NewInstanceDialog · 新建实例表单(打开态)";

// —— ImportModpackDialog(打开态外壳)—————————————————————————————————

export const ImportModpack: Story = () => (
  <DialogHarness>
    {(open, close) => (
      <ImportModpackDialog open={open} root="/mock/root" onClose={close} onImported={() => {}} />
    )}
  </DialogHarness>
);
ImportModpack.storyName = "ImportModpackDialog · 导入整合包(打开态)";

// —— ProjectDetailPanel(无后端 → 优雅空壳)————————————————————————————

/** 面板是 absolute inset-0,故事里套一个相对定位的画布容器给它铺满。 */
export const ProjectDetail: Story = () => (
  <div className="relative h-[520px] w-full overflow-hidden shadow-input">
    <ProjectDetailPanel
      projectId="sodium"
      title="Sodium"
      target="mod"
      instanceId="tech-survival"
      mcVersion="1.20.1"
      loader="fabric"
      onClose={() => {}}
      onInstalled={() => {}}
    />
  </div>
);
ProjectDetail.storyName = "ProjectDetailPanel · 详情面板(无后端空壳)";

/* ----------------------------------------------------------------------------
 * 跳过(SKIPPED)—— 依赖运行时数据 / Tauri,拿掉后端后无法有意义呈现,不硬塞假内壳:
 *   · AccountDialog / SkinDialog —— 账号登录(设备码流 / 皮肤上传)全流程依赖 Tauri。
 *   · InstanceManageDialog —— 内含大量 api.* 资源子面板(mods/世界/日志),空壳无意义。
 *   · ExportModpackDialog —— 需要选中实例 + 磁盘扫描。
 *   · JoinRealmDialog / RealmPanel / ServersPanel / DownloadQueue /
 *     NotificationCenter / FriendsSection —— 均绑 mc-server / store 实时流。
 * -------------------------------------------------------------------------- */
