# v0.2.0 · 正式版

第二个版本，第一个正式版。整合包、导入、安装选项、版本隔离都在这一版里。

## 新增

**整合包**
- 搜索、详情、版本选择，一键装到指定 `.minecraft`
- **本地导入**：手上已有 `.mrpack` 或 CurseForge 导出的 zip，可以直接选文件或拖进窗口
- Modrinth 无需 key；CurseForge 需要自备 API key（官方接口要求），key 只存本机
- 安装全程有读数：速度、剩余时间、文件数、阶段轨道

**导入**
- 扫描本机已有的 PCL / PCL CE / HMCL / BakaXL / Prism / MultiMC / 官方启动器
- 复制版本目录、mods、config 到目标，同名跳过不覆盖，可取消

**安装选项**
- 快照：正式版 / 全部（含测试版）筛选
- Fabric API：装完自动按 MC 版本取对应版本进 `mods/`
- OptiFine：Forge + 1.13 以上自动装；其余版本提供「下载并打开官方安装器」的降级路径
- 自定义版本名：装完改名并重写 `id`，校验 Windows 非法字符，失败可回滚

**版本隔离（默认开）**
- 模组进 `versions/<版本id>/mods/`，不和其他版本混；关闭则进根 `mods/`
- 只有 PCL / HMCL 这类支持版本隔离的启动器能读到；官方启动器只读根目录
- 目前只隔离 `mods/`，`config/`、`saves/` 仍共享

## 界面

- 导航改为图标 + 中文 + 英文三段式，六个分区
- 版本、加载器、目录三种选择统一为同一套自绘控件（不再用原生 `<select>`）
- 表单控件按角色统一：任何页面的输入框 / 下拉 / 多行框同一套样式
- 主操作吸底；Ctrl+1…4 切换分区
- 全部文案收进 `app/src/i18n/zh-CN.ts` 并重写了一遍

## 修复

- **整合包进度看不见**：读数面板渲染在 DOM 顶部、安装按钮在页面底部，点下去后面板被插到视口上方 1398px 处。改成左列表 / 右 sticky 栏，点安装和看进度在同一块地方
- **加载器识别**：改读版本 JSON 的 `libraries` 并沿 `inheritsFrom` 往父版本找，再对只有家族库的 Forge/NeoForge 兜底。之前 PCL 装的 Forge 整合包会被认成原版
- **下载卡死**：大文件总超时放宽后僵死连接会静默占住 worker（进度停在 N/N-1）。加 30 秒无字节即判失败重试
- **取消残留**：取消或出错时清掉本次新造的 `versions/<id>/`
- **1.6–1.7.2 虚拟资源**：`AssetIndex.virtual_` 缺 serde rename，`assets/virtual/` 永远不铺开（装完没声音没语言）
- **版本 JSON**：不再写出 `natives: null` 这类空字段

## 已知问题

- **只有 Windows 构建**
- **NeoForge 需要能访问 `neoforged.forgecdn.net`**（官方源已指向该 CDN）
- **1.6 及更早的版本多占一份磁盘**（按资源名铺开 `resources/`）
- **整合包只隔离 `mods/`**，配置和存档仍共享，同目录装两个包会互相覆盖配置
- **「下载游戏本体」那几秒没有逐字节进度**，进度条会退化成走格样式
- 安装包没有代码签名，Windows 可能弹 SmartScreen
- CurseForge 安装路径尚未用真 key 验证

## 使用前请注意

MCDL 不托管、不镜像、不分发任何 Minecraft 游戏文件。所有文件均从 Mojang 官方服务器
（或你手动开启的 BMCLAPI 镜像）获取。

**使用本工具获取 Minecraft 文件的前提，是你已经拥有对应的正版授权。**
本项目与 Mojang Studios、Microsoft、网易无关。

## 下载哪个

| 文件 | 用途 |
| --- | --- |
| `MCDL_0.2.0_x64-setup.exe` | 桌面版安装包（推荐） |
| `MCDL_0.2.0_x64_en-US.msi` | MSI，适合批量部署 |
| `mcdl-v0.2.0-windows-x86_64.exe` | 命令行版，单文件 |

校验和见 `SHA256SUMS.txt`。
