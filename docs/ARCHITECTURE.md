# 架构与实现约定

本文件是**契约**：`crates/mcdl-core` 里的公开类型和函数签名就是协议，改动必须同步这里。

## 分层

```
crates/mcdl-core     ← 引擎，不依赖任何 UI
  meta/              Mojang 元数据：清单、版本 JSON、inheritsFrom、rules
  net/               HTTP 客户端、下载源地址
  download/          并发下载、SHA1、natives 解压
  loader/            Fabric / Quilt / Forge / NeoForge
  java/              运行时探测、版本推断、Adoptium 获取
  install.rs         编排层（唯一入口 Installer::run）
crates/mcdl-cli      ← 命令行，用来在不开窗口的情况下验证引擎
app/                 ← Tauri 2 + React 界面
```

依赖方向只有一个：`app` → `mcdl-core` ← `mcdl-cli`。引擎里不许出现任何 UI 概念。

## 三条硬规则

### 1. 只写自己的实现，不搬运他人代码

- PCL 是自定义协议（GitHub 显示 NOASSERTION），**一行都不能拷**；
- HMCL 是 GPL-3.0，拷了就要求整个仓库 GPL；
- 需要的协议知识去读 <https://minecraft.wiki/w/Client.json>，不要读启动器源码后逐行翻译（逐行翻译同样是衍生作品）。

### 2. 加载器安装一律调用官方 installer

不要自己实现 Forge 的 `processors`。官方 installer 支持 headless：

```
java -jar forge-installer.jar --installClient          # Forge / NeoForge
java -jar fabric-installer.jar client -dir <gameDir> -mcversion <v> -loader <v>
```

调用二进制不传染 LGPL，行为也与官方一致。安装前必须保证 `<gameDir>/launcher_profiles.json` 存在，
否则 Forge installer 会直接退出（`GamePaths::prepare` 已经负责补这个文件）。

### 3. 下载源策略

| 内容 | 官方源 | BMCLAPI |
| --- | --- | --- |
| `version_manifest_v2.json` | 总是官方 | 不用镜像 |
| 版本 JSON | 总是官方 | 不用镜像 |
| 依赖库 libraries | 官方 | 可选 |
| 资源文件 assets | 官方 | 可选 |
| 游戏本体 client.jar | 官方 | 可选 |

镜像只是加速，默认必须关闭，且界面上要写明「第三方镜像」。

## 公开 API（节选）

```rust
// 入口
let installer = Installer::new()?;
let report = installer.run(&InstallRequest { .. }, &sink).await?;

// 进度
pub trait ProgressSink: Send + Sync {
    fn emit(&self, progress: Progress);
}
```

`Progress` 只描述**已经发生的事实**，不携带百分比：总量在不同阶段是逐步确定的，
百分比由接收方自己算。

```rust
pub enum Progress {
    Stage { kind: StageKind, index: usize, total: usize, label: String },
    Files { kind: StageKind, done: usize, total: usize, bytes_done: u64, bytes_total: u64 },
    Log   { level: LogLevel, message: String },
}
```

## 各模块要做的事

### meta

- `fetch_manifest`：拉 `piston-meta.mojang.com/mc/game/version_manifest_v2.json`
- `fetch_detail`：拉版本 JSON，沿 `inheritsFrom` 递归合并（子覆盖父，libraries 追加）
- `rules_allow`：按顺序匹配 `os.name` / `os.arch` / `features`，最后命中者生效；无规则默认允许
- 老版本兼容：`minecraftArguments` 整串参数、`assets` 为 `pre-1.6` 的虚拟布局

### net

- `HttpClient`：超时、UA、gzip；错误里带上 URL
- 地址拼接：`library_url` / `asset_url` 按源切换

### download

- `run_batch`：固定 worker 池并发；已存在且 SHA1 通过的文件跳过
- 校验失败：删除重下一次，再失败报 `HashMismatch`
- `extract_natives`：解压到 `versions/<id>/natives`，跳过 `META-INF/` 与 `extract.exclude`

### loader

- 版本查询：Fabric/Quilt 走 meta API；Forge/NeoForge 走各自 maven 的 `maven-metadata.xml`
- 安装：下载 installer → 调 java → 解析新增的 `versions/<新 id>/` → 返回版本 id
- 失败时把 installer 的完整 stdout/stderr 带回去（这是用户唯一能自助排查的东西）

### java

- `detect`：`JAVA_HOME`、`PATH`、`C:\Program Files\Java\*`、`Eclipse Adoptium`、`~/.jdks`
- `required_major`：≤1.16.5 → 8；1.17–1.20.4 → 17；1.20.5–1.21.x → 21；26.x 以 JSON 为准
- `ensure`：Adoptium API `api.adoptium.net/v3/assets/latest/{major}/hotspot`，下载后解压到 `<gameDir>/runtime`

## 测试要求

- `meta`：`rules` 求值的表驱动单测（各 OS × arch × features）
- `paths`：目录布局单测（已有）
- `install::maven_to_path`：坐标转换单测
- 端到端：`cargo run -p mcdl -- install 1.21.1 --no-assets --dir ./tmp-test` 能真的装完

## 界面

见 `docs/UI.md`。风格来自 `endfield-ui` skill，变体卡写在那个文件开头。
