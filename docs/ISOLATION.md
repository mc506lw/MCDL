# 版本隔离

> 一句话：开着隔离，模组装进 `versions/<版本id>/mods/`，同目录装第二个整合包不会混；
> 关掉隔离，模组装进共享的 `mods/`，所有版本共用。
>
> **但能不能读到，取决于你用哪个启动器。** 官方启动器只读根目录 `mods/`；
> PCL、HMCL 这类支持「版本隔离」的启动器才会去读 `versions/<id>/mods/`。

## 这是什么，不是什么

隔离是**启动期**语义：游戏自己不会去别的地方找 `mods/`，是启动器按你选的版本决定
加载哪个目录。MCDL 只负责安装，不启动游戏，所以这个功能只有两件事：

1. **装的时候放对地方** —— 模组（含 Fabric API、OptiFine、整合包自带模组、导入的模组）
   写进当前版本自己的目录；
2. **说清楚给谁用** —— 设置页、版本页、整合包安装日志里都写明：只有支持版本隔离的
   启动器能读到隔离后的模组。

MCDL 不实现启动逻辑，也不能让官方启动器读到隔离目录。想让官方启动器也能玩，
就把开关关掉（模组回根目录 `mods/`）。

## 落点规则

唯一算法在 `crates/mcdl-core/src/isolate.rs::mods_dir(game_dir, version_id, isolated)`：

| 开关 | 模组目录 |
| --- | --- |
| 开（默认） | `<gameDir>/versions/<版本id>/mods/` |
| 关 | `<gameDir>/mods/` |

`<版本id>` 是 `versions/` 下真正的目录名，例如 `1.21.1`、
`fabric-loader-0.16.9-1.21.1`、或者用户改名后的中文名。加载器装完才知道这个名字，
所以整合包安装的落点计算排在**阶段 3**（写整合包文件）之前，不在阶段 1。

前端镜像：`app/src/lib/isolate.ts`，只用来显示落点和做输入校验，规则必须与 Rust 一致
（`selfCheck()` 里有对照用例）。

### 会走隔离的调用点（全部经过 `isolate::mods_dir`）

| 路径 | 文件 | 说明 |
| --- | --- | --- |
| 整合包模组下载 | `pack/install.rs::mrpack_jobs` | `mods/**` 走隔离，`config/`、`resourcepacks/` 等仍留根目录 |
| CurseForge 整合包 | `pack/install.rs::curseforge_jobs` | 清单里的文件全部是模组，直接进隔离目录 |
| 整合包 `overrides/` | `pack/mrpack.rs::extract_overrides_isolated` | `overrides/mods/**` 走隔离，其余目录留根目录 |
| Fabric API / QFAPI | `extras/fabric_api.rs::install` | |
| OptiFine（Forge mod） | `extras/optifine.rs::install` | |
| 导入 | `import.rs::import` | 源 `mods/` 复制进每个被导入版本的 `mods/`；没有版本可挂时退回共享目录并记一条警告 |

### 不走隔离的东西（有意为之）

- **`config/`、`saves/`、`resourcepacks/`、`shaderpacks/`**：PCL 的完整隔离也覆盖这些，
  但 MCDL 只管模组。整合包 `overrides/` 里的这些目录仍解到游戏根目录。
  → 副作用：同一个游戏目录装两个整合包时，配置/存档会互相覆盖。这是已知限制。
- **游戏本体、依赖库、资源文件**：本来就在共享位置，与隔离无关。

## 界面

- **设置页 → 版本隔离**：开关（默认开）+ 常驻提示「隔离后的模组只有 PCL / HMCL
  这类支持版本隔离的启动器能读到；官方启动器只读根目录的 mods/」。
- **版本页**：每个版本显示「隔离 / 共享」标记和它当前的模组目录；另有一栏解释为什么
  不提供自动迁移。
- **整合包安装日志**：阶段 3 开始时打印模组落点，例如
  `版本隔离：模组写入 versions/fabric-loader-0.16.9-1.21.1/mods/（只有 PCL / HMCL 这类启动器会读这个目录）`。

## 不做的迁移

不提供「把某个版本改成隔离」的一键迁移：根 `mods/` 里的 jar 属于哪个版本只能靠猜，
搬错会静默弄坏别的实例。想换落点就重新装一次那个整合包，或者手动把该版本专属的 jar
从 `mods/` 移到 `versions/<版本id>/mods/`。

## 安全边界

版本目录名会被当作一层目录拼进路径，所以 `isolate::validate_version_id` 拒绝：
`/`、`\`、`:`、`*`、`?`、`"`、`<`、`>`、`|`、控制字符、`.`、`..`、空串。
整合包索引里的相对路径（不可信输入）走 `isolate::place_pack_file`，再挡一次
绝对路径和 `..`。关掉隔离时不校验版本名（那时它不参与路径），所以调用方即使还不知道
版本目录名也能安全拿到共享目录。

## 验证

```bash
# 单测（含开/关、特殊版本名、路径穿越）
CARGO_TARGET_DIR=target-iso cargo test -p mcdl-core

# 前端类型检查
cd app && pnpm build

# 真跑：隔离开
target-iso/debug/mcdl --dir tmp-iso/iso install 1.21.1 --loader fabric --fabric-api --no-assets
ls tmp-iso/iso/versions/*/mods/

# 真跑：隔离关
target-iso/debug/mcdl --dir tmp-iso/shared install 1.21.1 --loader fabric --fabric-api --no-assets --no-isolate
ls tmp-iso/shared/mods/
```
