# 07 · 目录模型与便携性(决策)

> 决策:启动器本体**独立于所有实例之外**存在,但通过「游戏根目录多根列表 + 自动检测」机制管理散落的实例,**包括检测自身所在/同级目录是不是一个 MC 实例**。对标 PCL 的 `McFolder` 行为。
> 关联:[modules/instance.md](./modules/instance.md)、[refs/pcl2.md](./refs/pcl2.md)(`McFolderListLoader`)。

---

## 1. 三层概念(必须分清)

```
① 启动器本体(Launcher Binary)
     独立可执行文件。不住在任何实例里。可放 Program Files / Applications / 任意文件夹。
        │ 管理
        ▼
② 启动器数据(Launcher Data)
     accounts / settings / 下载的 Java / 元数据缓存。
     默认:系统 app-data(集中式);便携模式:放在 exe 旁。
        │ 引用一个列表
        ▼
③ 游戏根目录(Game Root / "MC Folder",可多个)
     每个是一个 .minecraft 式根:versions/ libraries/ assets/ saves/ ...
     来源:自动检测 + 手动添加。
        │ 包含
        ▼
④ 实例/版本(Instance,可多个)
     某个根目录下的具体可启动版本。
```

**关键**:① 永远独立于 ④。启动器**发现并接管**实例,而不是**住在**实例里。

---

## 2. 游戏根目录的发现(启动时)

按优先级注册多个根,形成一个列表(用户在 UI 里可切换/管理):

| 优先级 | 来源 | 说明 |
|--------|------|------|
| 1 | **当前/同级目录** | 看启动器 exe 所在目录:若它本身含 `versions/`,或含 `.minecraft/`,或同级有 `.minecraft/` → 注册为根。**便携 / sibling 检测**。 |
| 2 | **官方目录** | Win `%APPDATA%\.minecraft` · macOS `~/Library/Application Support/minecraft` · Linux `~/.minecraft` |
| 3 | **用户自定义** | 设置里手动添加的任意路径,持久化 |
| 4 | **兜底** | 一个都没有 → 在启动器数据目录下创建一个默认根 |

> "某目录是不是 MC 实例"的判定:存在 `versions/<id>/<id>.json`,或存在 `.minecraft/versions/`。检测要轻(只看目录结构,不解析全部 json)。

```rust
pub struct GameRoot { pub name: String, pub path: PathBuf, pub kind: RootKind }
pub enum RootKind { Portable, Official, Custom, Default }

/// 启动时扫描,返回所有发现的根
pub fn discover_roots(exe_dir: &Path, data_dir: &Path) -> Vec<GameRoot> {
    let mut roots = vec![];
    if let Some(r) = detect_mc_dir(exe_dir) { roots.push(r.portable()); }  // ①
    if let Some(r) = official_minecraft_dir() { roots.push(r.official()); } // ②
    roots.extend(load_custom_roots(data_dir));                             // ③
    if roots.is_empty() { roots.push(default_root(data_dir)); }            // ④
    roots
}
fn detect_mc_dir(dir: &Path) -> Option<GameRoot> {
    for cand in [dir.to_path_buf(), dir.join(".minecraft")] {
        if cand.join("versions").is_dir() { return Some(GameRoot::at(cand)); }
    }
    None
}
```

---

## 3. 两种使用形态(同一套机制涌现)

### 集中式(普通用户,默认)
- 启动器装在 Program Files / Applications。
- 启动器数据 → 系统 app-data。
- 实例 → 启动器管理的中央根(default root)。
- 用户基本无感知目录,UI 里管理实例即可。

### 便携式(整合包分发 / U 盘 / 多开 / 进阶用户)
- 启动器 exe 丢进任意文件夹,旁边有/没有 `.minecraft` 都行。
- 检测到同级实例 → 直接接管(①)。
- **portable 标记**:exe 旁放一个标记文件(如 `portable.txt` 或 `launcher.portable`)→ 启动器数据也写到 exe 旁,而非系统 app-data。
- 整个文件夹可整体拷贝/带走,环境自洽。

```rust
/// 决定启动器数据放哪
pub fn resolve_data_dir(exe_dir: &Path) -> PathBuf {
    if exe_dir.join("portable.txt").exists() || exe_dir.join(".portable").exists() {
        exe_dir.join("launcher-data")     // 便携:跟着 exe 走
    } else {
        system_app_data_dir()             // 集中:系统目录
    }
}
```

---

## 4. 隔离与共享(配合 instance.md)

- 一个根目录下可有多个实例;实例之间用 [instance.md](./modules/instance.md) 的隔离策略(完全隔离 / 版本隔离)。
- 跨根目录:各根完全独立(便携包里的根 vs 官方根互不干扰)。
- `libraries/assets` 的共享 store 是**每个根内部**的优化,不跨根(便携包要自洽,不能依赖外部 store)。

---

## 5. 设计要点

1. **`discover_roots` 做成核心层纯函数**,输入 exe 目录 / 数据目录,输出根列表 —— 可单测,CLI/UI 共用。
2. **检测要轻且安全**:只看目录结构判定,不在启动时解析全部版本 json(那是进入某根后再做)。
3. **portable 判定优先**:有标记文件就一切跟着 exe 走,保证整包可移动。
4. **官方目录跨平台**:三平台路径不同,封装一个函数。
5. **UI 暴露根管理**:像 PCL 那样允许添加/删除/切换游戏目录(对应左图标栏或设置页)。
6. **与官启/HMCL 共存**:接管官方 `.minecraft` 时不破坏其结构,生成的 `versions/<id>` 兼容,允许双启动器并用。

---

## 6. 一句话回答原始决策

**启动器独立于所有实例之外**(① 永不住在实例里),**同时**通过「同级目录检测 + 多根列表」**自动发现并接管**当前/官方/自定义目录里的实例 —— 集中式和便携式用同一套机制,由 portable 标记和检测结果自然区分。✅ 两者都做。
