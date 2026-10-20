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

## 用法

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
