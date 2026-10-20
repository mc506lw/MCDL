# MCDL

Minecraft 安装器。装原版，装 Fabric / Quilt / Forge / NeoForge，装完就走。

开源、免费、无广告、不需要注册任何账号。

## 它做什么

- 从 Mojang 官方源下载游戏本体、依赖库、资源文件，逐个校验 SHA1
- 安装模组加载器：Fabric、Quilt、Forge、NeoForge
- 本机没有合适的 Java 时，从 Adoptium 取一份 JRE 放进游戏目录
- 命令行和图形界面共用同一个引擎

## 它不做什么

这些是刻意的取舍，不是还没做：

- **不托管任何游戏文件**。所有文件都从 Mojang 官方服务器取（或用户手动选择的第三方镜像）
- **不提供离线模式**。请使用自己的微软正版账号
- **不内置整合包、不建资源站**
- **不修改、不破解游戏本体**

## 实测数据

在 Windows / x86_64 / 千兆网下实测（不是估算）：

| 场景 | 耗时 | 体积 |
|---|---|---|
| 1.20.4 原版全量（3853 文件） | 173 s | 708.8 MB |
| 26.3 原版全量（5222 文件） | 217 s | 585.6 MB |
| 1.12.2 原版全量 | 63 s | 172.6 MB |
| 1.21.1 + Fabric | 152 s | 93.6 MB（不含资源） |
| 1.20.4 + Forge | 235 s | 179.8 MB（不含资源） |
| 二次安装（全部命中缓存） | 1.5 s | 0 文件 / 0 字节 |

所有 client.jar 的 SHA1 与官方一致；依赖库数量与按 `rules` 语义独立算出的结果完全吻合。

## 已知限制

- **NeoForge 需要能访问 `maven.neoforged.net`**。它的 installer 自己下载依赖库，BMCLAPI 的镜像对它无效——这是官方 installer 的行为，不是本项目能绕过的。
- **1.6 及更早的版本会多占一份磁盘**。这些版本按资源名读文件，需要把 `assets/objects` 再铺一份到 `resources/`（1.5.2 约多占 50 MB）。
- **不提供启动功能**。装完请用你自己的启动器打开 `versions/` 下的实例。
- 每次安装加载器都会重跑一次官方 installer（幂等，但重复安装仍需要几十秒）。

## 用法

从源码跑（克隆下来就能用，不需要安装）：

```bash
cargo run -p mcdl -- versions          # 等价于下面的 mcdl versions
cargo run -p mcdl -- install 1.21.1 --no-assets --dir ./tmp
```

想直接用 `mcdl` 这个名字，装一下：

```bash
cargo install --path crates/mcdl-cli
```

命令行：

```bash
# 看有哪些版本
mcdl versions --release-only

# 装原版
mcdl install 1.21.1 --dir "D:\Games\.minecraft"

# 装原版 + Fabric
mcdl install 1.21.1 --loader fabric --dir "D:\Games\.minecraft"

# 只装能启动的最小集合（跳过资源文件）
mcdl install 1.21.1 --no-assets --dir ./tmp
```

图形界面：

```bash
cd app
pnpm install
pnpm tauri dev
```

## 下载源

默认全部走 Mojang 官方源。界面和命令行都可以切到 BMCLAPI 社区镜像加速，
但版本清单和版本 JSON 永远走官方，避免镜像被改写。

## 结构

```
crates/mcdl-core    引擎（无 UI 依赖）
crates/mcdl-cli     命令行
app/                Tauri 2 + React 界面
docs/               架构与界面说明
```

细节见 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)。

## 关于许可证

引擎和界面是本项目自己写的。模组加载器的安装一律调用官方 installer 的 headless 模式：

- [MinecraftForge/Installer](https://github.com/MinecraftForge/Installer) — LGPL-2.1
- [FabricMC/fabric-installer](https://github.com/FabricMC/fabric-installer) — Apache-2.0

调用二进制，不复制源码。

界面字体 Saira 来自 [The Saira Project](https://github.com/Omnibus-Type/Saira)，SIL Open Font License 1.1，许可证随包附带。

## License

MIT OR Apache-2.0
