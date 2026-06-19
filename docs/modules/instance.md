# 模块 · 实例管理

> 实例 = 一个隔离的游戏环境(自己的版本、mods、存档、配置、设置)。这是现代启动器对比官启的核心体验。

## 1. 实例隔离

每个实例有独立的工作目录,互不污染。两种隔离粒度:
- **完全隔离**(推荐):每实例独立 `.minecraft`,mods/saves/config/resourcepacks 全独立。
- **版本隔离**(PCL 默认可选):共享 libraries/assets,但 mods/saves/config 按版本分目录(`versions/<id>/PCL/`)。

完全隔离更干净,代价是磁盘占用(libraries/assets 可用硬链接/共享 store 优化,Prism 的做法)。

## 2. 目录结构(参考 Prism 完全隔离)

```
instances/<实例名>/
├── instance.cfg              # 实例设置(内存、Java、窗口…)
├── mmc-pack.json            # 组件配置(版本系统)
├── icon.png                 # 实例图标
└── .minecraft/
    ├── versions/            # 版本 jar + json
    ├── libraries/           # 依赖库(可全局共享)
    ├── assets/              # 资源(可全局共享)
    ├── mods/                # loader mods
    ├── coremods/            # coremods(老 Forge)
    ├── resourcepacks/       # 资源包
    ├── shaderpacks/         # 光影
    ├── datapacks/           # 数据包
    ├── saves/               # 世界存档
    ├── screenshots/         # 截图
    ├── logs/                # 游戏日志
    └── options.txt          # 游戏设置
```

> 共享资源优化:libraries/assets 放全局 store,实例内用符号链接/硬链接引用,省磁盘。

## 3. 实例配置(两级)

- **全局默认**:内存、Java、下载源、窗口大小的默认值。
- **实例覆盖**:每个实例可覆盖全局(`instance.cfg`),如这个整合包要更大内存、固定某 Java。

## 4. 资源子管理

每类资源一个 FolderModel(扫描目录 + 解析元数据 + 启用/禁用):

| 资源 | 操作 | 元数据来源 |
|------|------|-----------|
| Mods | 启用/禁用(改后缀 `.disabled`)、删除、看依赖 | jar 内 `fabric.mod.json` / `mods.toml` |
| 资源包 | 安装、排序 | `pack.mcmeta` |
| 光影 | 安装 | 文件名 |
| 数据包 | 安装 | `pack.mcmeta` |
| 世界 | 重命名、删除、备份、复制 | `level.dat`(名称、游戏模式、种子、最后游玩时间) |
| 截图 | 查看、删除 | 文件 |

> Prism `minecraft/mod/`(各 FolderModel)、`minecraft/World.h`/`WorldList`;PCL `PageInstance*`。

## 5. 实例操作

- 创建(原版 / 从整合包 / 复制现有 / 导入)
- 复制(可选择复制哪些:mods/saves/config…,Prism `InstanceCopyPrefs`)
- 导入/导出(打包分享 `.zip`/`.mrpack`)
- 重命名、改图标、分组
- 从其他启动器迁移(官启 launcher_profiles、MultiMC/HMCL 实例)
- 创建桌面快捷方式(直接启动某实例)

## 6. 自研要点

1. **默认完全隔离**,libraries/assets 用全局共享 store + 链接优化磁盘。
2. **instance.cfg 用简单 KV**(ini/json),全局设置同结构,实例覆盖全局。
3. **资源管理统一抽一个 FolderModel 基类**(扫描 + watch 文件变化 + 启用禁用),各资源类型继承。
4. **世界备份**是高频刚需,做成一键 + 自动定期。
5. 实例迁移能极大降低用户切换成本,优先支持从官启和 HMCL 导入。
