# v0.1.0-beta.1 · 测试版

第一个公开测试版。**能装游戏，但请当作测试品用**——接口和界面还会变。

## 这是什么

MCDL 是一个 Minecraft **安装器**：从 Mojang 官方服务器取游戏文件，按你的选择装上模组加载器。
它不启动游戏，也不托管任何游戏文件。

## 这一版有什么

- **原版全版本安装**：按 `rules` 精确过滤依赖库，逐个校验 SHA1
- **四个模组加载器**：Fabric / Quilt / Forge / NeoForge，全部真装验证过
- **Java 运行时**：本机没有合适的版本时从 Adoptium 取一份
- **安装前预估**：点开始之前就告诉你这次要下多少
- **版本管理**：扫描磁盘列出已装版本，可单独删除
- **多目录**：常用的 `.minecraft` 目录可收藏、切换
- **命令行**：`mcdl` 单文件 4.1 MB，没有界面也能装

## 下载哪个

| 文件 | 用途 |
| --- | --- |
| `MCDL_0.1.0_x64-setup.exe` | 桌面版安装包（推荐，NSIS） |
| `MCDL_0.1.0_x64_en-US.msi` | 桌面版安装包（MSI，适合批量部署） |
| `mcdl-v0.1.0-beta.1-windows-x86_64.exe` | 命令行版，单文件，放进 PATH 即可用 |

## 实测数据

| 场景 | 耗时 | 体积 |
| --- | --- | --- |
| 1.20.4 原版全量（3853 文件） | 173 s | 708.8 MB |
| 26.3 原版全量（5222 文件） | 217 s | 585.6 MB |
| 1.12.2 原版全量 | 63 s | 172.5 MB |
| 1.21.1 + Fabric | 152 s | 93.6 MB |
| 1.21.1 + NeoForge | 136 s | 176.8 MB |
| 二次安装（命中缓存） | 1.5 s | 0 文件 |

Windows / x86_64 / 千兆网。所有 `client.jar` 的 SHA1 与官方一致。

## 已知问题

- **只有 Windows 构建**。Linux / macOS 理论可用，但没有实测，也没打包。
- **界面文案还没接进 i18n**。`app/src/i18n/zh-CN.ts` 里已经是全部文案，但只有命令栏接线了，
  改字典里的其他条目暂时不生效。见 `docs/I18N.md` 的状态表。
- **NeoForge 需要能访问 `neoforged.forgecdn.net`**。官方源已指向该 CDN，原站
  `maven.neoforged.net` 在国内常解析到污染地址。
- **1.6 及更早的版本会多占一份磁盘**（按资源名铺开 `resources/`，1.5.2 约多占 50 MB）。
- 设置还存在 `localStorage`，没有落成配置文件。
- 安装包没有代码签名，Windows 可能弹 SmartScreen 提示。

## 使用前请注意

MCDL 不托管、不镜像、不分发任何 Minecraft 游戏文件。所有文件均从 Mojang 官方服务器
（或你手动开启的 BMCLAPI 镜像）获取。

**使用本工具获取 Minecraft 文件的前提，是你已经拥有对应的正版授权。**
本项目与 Mojang Studios、Microsoft、网易无关。

## 校验

```
8a4c8010…  MCDL_0.1.0_x64-setup.exe
dfdc9399…  MCDL_0.1.0_x64_en-US.msi
0732358b…  mcdl-v0.1.0-beta.1-windows-x86_64.exe
```

完整校验和见 `SHA256SUMS.txt`。
