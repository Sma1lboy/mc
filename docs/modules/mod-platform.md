# 模块 · Mod / 整合包平台

> 接入 CurseForge / Modrinth 做 Mod 搜索安装、依赖解析、整合包导入。

## 1. 平台抽象(ResourceAPI 模式)

不同平台 API 不同,但操作一致,抽象一个统一接口(Prism `ResourceAPI`):

```
interface ResourceAPI:
    searchProjects(query, filters) -> [IndexedPack]      # 搜索
    getProjectInfo(id) -> IndexedPack                    # 详情
    getProjectVersions(id, gameVersion, loader) -> [IndexedVersion]  # 版本列表
    getDependencies(version) -> [Dependency]             # 依赖
```

每个平台(Modrinth / CurseForge / FTB / Technic …)实现这套接口,各自处理 API 调用和 JSON 解析。上层只面向抽象,新增平台不影响 UI。

## 2. 两大平台 API

| 平台 | 搜索 | 详情 | 版本 | 备注 |
|------|------|------|------|------|
| **Modrinth** | `GET /v2/search` | `GET /v2/project/{id}` | `GET /v2/project/{id}/version` | 开放、现代、无需 key,**优先接** |
| **CurseForge (Flame)** | `GET /v1/mods/search` | `GET /v1/mods/{id}` | `GET /v1/mods/{id}/files` | **需 API key**,生态最大 |

过滤维度:游戏版本、loader(fabric/forge/quilt/neoforge)、资源类型(mod/资源包/光影/数据包/整合包)。

> 国内可走 McIM 等镜像加速 CurseForge/Modrinth 的文件下载(C4 特性)。

## 3. 统一数据模型

```
IndexedPack { id, name, description, author, iconUrl, type, dependencies[] }
IndexedVersion { id, versionNumber, gameVersions[], loaders[], files[], dependencies[] }
Dependency { projectId, type: REQUIRED|OPTIONAL|INCOMPATIBLE|EMBEDDED }
ModLoaderType: Forge | Fabric | Quilt | NeoForge | LiteLoader | ...
```

> Prism `modplatform/ModIndex.h`;PCL `ResourceProject/`(Curseforge + Modrinth 各一套 model)。

## 4. 依赖解析

装一个 mod 时递归解析它的依赖:

```
resolve(targetMod, mcVersion, loader):
  for dep in targetMod.dependencies where type == REQUIRED:
      candidate = 在平台找 dep 项目里, 兼容 mcVersion + loader 的最新版本
      if 已安装且满足 → skip
      elif 找到 → 加入待安装, 递归 resolve(candidate)
      else → 标记 unresolved(报告给用户)
  去重 + 冲突检测(INCOMPATIBLE)
→ { toInstall[], satisfied[], unresolved[] }
```

> PCL-CE `ResourceProject/ModDependencyResolver` 是清晰的参考(返回 ToInstall/Satisfied/Unresolved 三类)。

## 5. 整合包导入

支持的格式:

| 格式 | 标识文件 | 来源 |
|------|----------|------|
| Modrinth | `modrinth.index.json` | Modrinth |
| CurseForge | `manifest.json` | CurseForge |
| MultiMC / Prism | `mmc-pack.json` + `instance.cfg` | MultiMC 系 |
| MCBBS | `mcbbs.packmeta` | 国内社区 🌟 |

导入流程:
```
解压包 → 读 manifest(MC 版本 / loader / mod 文件列表 / overrides)
→ 创建实例(写组件配置)
→ 并发下载 manifest 里的 mod 文件(按 projectId+fileId 或直链)
→ 把 overrides/ 覆盖进实例目录(配置、资源包等)
→ 安装 loader
```

> Prism `modplatform/{modrinth,flame,...}/*InstanceCreationTask`;PCL `ModModpack`(4 格式)。

## 6. 自研要点

1. **先接 Modrinth**(开放、无 key、格式干净),CurseForge 后补(要申请 key、合规)。
2. **ResourceAPI 抽象先行**,即使一开始只有一个平台,也按接口写,后面加平台零成本。
3. **依赖解析要给用户可见的预览**(将安装哪些、哪些冲突、哪些没找到),别静默装。
4. **整合包的 overrides 要小心覆盖**用户已有文件,导入到隔离实例最安全。
5. 本地 mod 管理(启用/禁用 = 改 `.jar`/`.jar.disabled` 后缀、读 mod 元数据)是配套功能。
