# 加载器安装与 Java 运行时：维护笔记

> 面向后来维护 `crates/mcdl-core/src/loader/` 和 `java/` 的人。
> 下面每条结论都来自真机实测（Windows + Zulu JDK），不是从别的启动器抄的。
> 协议/命令行依据只有两个：各 installer 自己的 `--help` 和 Mojang 的公开元数据。

## 0. 一条设计红线

**不自己实现 Forge 的 processors，一律调用官方 installer 的 headless 模式。**

好处：许可证干净（调用二进制不传染 LGPL-2.1）、行为和官方完全一致、Mojang 改格式时我们不用跟着改。
代价：进度粒度只能靠解析 installer 的 stdout；失败时只能把它的输出原样带给用户。

所以 `loader::install()` 的职责只有四件事：**查版本 → 下 installer → 起 java → 认产物目录**。

## 1. 四个加载器对照表

| 加载器 | 版本列表来源 | installer 来源 | 产物目录 |
| --- | --- | --- | --- |
| Fabric | `https://meta.fabricmc.net/v2/versions/loader/{mc}` | `meta.fabricmc.net/v2/versions/installer` 给的 `url`（兜底 1.1.2） | `versions/fabric-loader-{loader}-{mc}` |
| Quilt | `https://meta.quiltmc.org/v3/versions/loader/{mc}` | `meta.quiltmc.org/v3/versions/installer`（兜底 0.15.1） | `versions/quilt-loader-{loader}-{mc}` |
| Forge | `https://maven.minecraftforge.net/net/minecraftforge/forge/maven-metadata.xml` | `.../forge/{mc}-{forge}/forge-{mc}-{forge}-installer.jar` | `versions/{mc}-forge-{forge}` |
| NeoForge | `https://maven.neoforged.net/releases/net/neoforged/neoforge/maven-metadata.xml` | `.../neoforge/{ver}/neoforge-{ver}-installer.jar` | `versions/neoforge-{ver}`（≤1.20.1 为 `versions/{mc}-neoforge-{ver}`，未在本机实测） |

### 1.1 Fabric

```bash
java -Dfile.encoding=UTF-8 -Dsun.stdout.encoding=UTF-8 -Dsun.stderr.encoding=UTF-8 \
  -jar fabric-installer-1.1.2.jar client \
  -dir <游戏目录绝对路径> -mcversion 1.21.1 -loader 0.19.5
```

- `-dir` **必须已经存在**，否则直接 `FileNotFoundException: Launcher directory not found`。`GamePaths::prepare()` 负责建。
- installer 会自己往 `launcher_profiles.json` 里加 profile（日志里的 `Creating profile`），不需要我们做。
- `stable` 字段别当"最新版"用：1.21.1 下 `0.20.1` 是 `stable:false`，`0.19.5` 才是 `stable:true`。
  自动安装要优先 stable（`pick_loader_version`），但 UI 列表仍按版本从新到旧排。

### 1.2 Quilt

**Quilt installer 的 CLI 和 Fabric 不一样**（虽然它是 Fabric installer 的分叉）：

```bash
java ... -jar quilt-installer-0.15.1.jar install client 1.21.1 0.30.1 --install-dir=<绝对路径>
```

不是 `client -dir ...`。照 Fabric 抄会直接 `Invalid argument`。

### 1.3 Forge

```bash
java ... -jar forge-1.21.1-52.1.20-installer.jar --installClient <游戏目录绝对路径>
```

- `--installClient [File]` 的 File 就是 `.minecraft` 目录（`--help` 原文："Install the client files to the specified directory"），不需要改 cwd。
- 用户显式选了 BMCLAPI 时额外加 `--mirror https://bmclapi2.bangbang93.com/maven`，**实测有效**：53 个库改走镜像。
- **坑：Forge installer 会顺手写一个原版 `versions/<mc>/` 目录**（里面是原版 json）。装完做差集会同时看到
  `1.21.1` 和 `1.21.1-forge-52.1.20`，识别逻辑必须能排除纯原版名，否则会把原版目录当成加载器结果返回。
- Forge 1.21.1 的 installer 用 Java 17 也能跑完（它只是 patch jar），但**启动**需要 Java 21，Java 版本要按版本 JSON 的 `javaVersion` 给。

### 1.4 NeoForge

```bash
java ... -jar neoforge-21.1.264-installer.jar --installClient <游戏目录绝对路径>
```

- 1.20.2+ 坐标：`net/neoforged/neoforge`，版本形如 `21.1.264`（`{mc minor}.{mc patch}.{build}`）；
  ≤1.20.1 还叫 `net/neoforged/forge`，版本形如 `1.20.1-47.1.106`。两条路径都实现了。
- **NeoForge 的 installer 必须能直连 `maven.neoforged.net`**：依赖库由它自己下载。
- **BMCLAPI 的 `--mirror` 对 NeoForge 无效**（实测 0 个库走镜像，全部仍打 `maven.neoforged.net`）。
  也就是说：用 `--source bmclapi` 能让我们这边下到 installer，但 installer 自己那一步救不回来。
  这条已经写进失败提示里，别让用户以为是我们的 bug。

### 1.5 退出码约定

| 退出码 | 含义 | 我们的处理 |
| --- | --- | --- |
| `0` | 安装成功 | 扫 `versions/` 找产物 |
| 非 `0` | 安装失败 | 报错，带上最后 40 行 + 错误摘录 + 针对性提示 |

- Forge / NeoForge 失败**确实返回非零**（用 `--offline` 验证过，`A problem installing was detected` 后 exit 1），
  不会出现"失败但退出码 0"被误判成功的情况。
- 失败信息 = 最后 40 行（任务要求）+ 额外的「错误摘录」：因为 Forge 系失败时真正的异常栈在前面，
  末尾只剩一句 `There was an error during installation`，只给 40 行等于什么都没说。

## 2. 版本目录识别：为什么是「差集 + 关键字打分」

四个加载器的目录名规则不同，而且会变：

```
fabric-loader-{loader}-{mc}     e.g. fabric-loader-0.19.5-1.21.1
quilt-loader-{loader}-{mc}      e.g. quilt-loader-0.30.1-1.21.1
{mc}-forge-{forge}              e.g. 1.21.1-forge-52.1.20
neoforge-{ver}                  e.g. neoforge-21.1.264
{mc}-neoforge-{ver}             e.g. 1.20.1-neoforge-47.1.106（老版本）
```

**不要写死名字**。`pick_version_id()` 的做法：

1. 装之前把 `versions/` 下所有目录名快照成集合；
2. 装完再做差集，只考虑**新增且有 `<name>.json`** 的目录；
3. 打分挑最像加载器的：`(精确匹配期望名, 含加载器关键字, 含 MC 版本)` 三元组取最大；
4. 差集为空时（同一组合重复安装，installer 复用旧目录）退回按期望名直接找。

这样即使某天 Fabric 改名、或者 NeoForge 又换一套命名，也只是"精确匹配"那档失效，关键字那档仍然能兜住。

## 3. 幂等性（用户会反复装，目录炸了最难查）

实测同一目录连续装三次（fabric → fabric → forge）：

| 轮次 | `versions/` | 返回的 version_id | 新增文件 |
| --- | --- | --- | --- |
| fabric #1 | `1.21.1`、`fabric-loader-0.19.5-1.21.1` | `fabric-loader-0.19.5-1.21.1` | 71 |
| fabric #2 | 同上，**没有** `-1`/`-2` 后缀目录 | 同第一次 | 0 |
| forge #1 | 追加 `1.20.4`、`1.20.4-forge-49.2.13` | `1.20.4-forge-49.2.13` | 43 |

- 两次 fabric 安装后，两个 json 的 sha1 **完全一致**；装 forge 之后 fabric 的 json 也没被改。
- 原版 `versions/<mc>/<mc>.json` 由编排层（install.rs）在最后写回，内容稳定；加载器本身不碰它。

支撑幂等的三个点：差集识别（不会新建重复目录）、installer 缓存按版本命名、`pick_version_id` 的兜底分支。

## 4. Java 运行时（`java/mod.rs`）

### 4.1 `detect()`

扫描顺序：`JAVA_HOME`（本身 + `<home>/jre` + **父目录的同级目录**）→ `PATH` → 各平台常见目录
（`Program Files\{Java,Eclipse Adoptium,Zulu,Microsoft,Amazon Corretto,BellSoft,Semeru,GraalVM,AdoptOpenJDK}`、
`%LOCALAPPDATA%\Programs\{Eclipse Adoptium,Microsoft,Java}`、`%APPDATA%\.minecraft\runtime`（官方启动器的 JRE）、
`~/.jdks`、`~/.gradle/jdks`、`~/.sdkman/candidates/java`）。

- 每个候选都真跑一次 `java -version` 解析主版本 / vendor / arch，跑不起来的跳过。
- 去重两轮：canonicalize 去同路径；`(major, version, vendor)` 去"JDK8 的 bin/java 与 jre/bin/java"这种同运行时副本。
- 找到的每个运行时都打一条 info（`发现 Java major=21 vendor="Azul Zulu" path=...`），方便用户自查"为什么选这个"。

### 4.2 `pick()` 的语义（曾经在这里翻过车）

```rust
runtimes.iter().filter(|j| j.major >= major).min_by_key(|j| j.major)
```

- 需求 8、本机 17 → 返回 17（版本高可以跑低版本）。
- 需求 21、本机只有 8/17 → **返回 `None`**，调用方据此触发下载。
  **绝对不能**"全都低于需求就退回最大的那个"：那会让 1.21.1 用 Java 17 启动，字节码直接报
  `UnsupportedClassVersionError`，而且永远不会触发自动下载。

### 4.3 `required_major()`

| MC | Java |
| --- | --- |
| ≤ 1.16.5 | 8 |
| 1.17 – 1.20.4 | 17 |
| 1.20.5 – 1.21.x | 21 |
| 26.x / 认不出（快照） | 21（真实需求以版本 JSON 的 `javaVersion.majorVersion` 为准） |

### 4.4 `ensure()`：布局与镜像回退

- 布局：`<dest>/jre-<major>/bin/java(.exe)`，`dest` 通常是 `<gameDir>/runtime`。
  **故意不扁平**：1.12 要 8、1.21 要 21，扁平布局会让两个版本互相覆盖。
- 来源：Adoptium API `https://api.adoptium.net/v3/assets/latest/{major}/hotspot?architecture=x64&image_type=jre&os=windows`，
  拿 `package.link`（GitHub Releases）流式下载 → 校验 PK 头 → 解压时剥掉统一顶层目录。
- **回退**：GitHub 在国区经常不可达。主源失败时自动换
  `https://mirrors.tuna.tsinghua.edu.cn/Adoptium/{major}/jre/{arch}/{os}/{name}.zip`
  （同一份 Temurin 产物，只是换了托管），并打 `warn` 日志 + 进度日志明说"改用清华镜像"。
  用户必须能知道这件事——UI 侧也有一行说明。
- 已存在 `jre-<major>/bin/java` 时直接复用，不重复下载（单测里连续调两次，第二次直接返回同一路径）。

## 5. 给 UI 的版本列表 API

`available_loader_versions(client, kind, mc)` 会被界面在下拉框里直接调用，所以：

| 保障 | 实现 |
| --- | --- |
| 进程内缓存 | 按 `来源|加载器|MC` 缓存 5 分钟；命中时 < 50µs（实测 23.7µs vs 冷查询 3.3s） |
| 超时 | 单次查询 10s 上限，超时返回 `Err`，不会把界面挂住 |
| 限条数 | 只返回前 20 条（`VERSION_LIST_LIMIT`） |
| 排序 | 严格从新到旧（自然版本比较：`52.1.20 > 52.1.9 > 52.1.2`，预发布版排在正式版后面） |
| 错误措辞 | 统一成「查不到 X 的版本列表：…」，**不会**出现「安装加载器失败」这种误导文案 |

注意两个刻意的分离：

- **列表顺序 ≠ 自动安装选谁**。列表严格从新到旧（UI 要这样），自动安装优先最新**稳定版**（`pick_loader_version`）。
- `supported_mc_versions` 也带缓存和超时，但**不截断**：版本选择器要能翻到 1.7.10。

## 6. 踩坑清单（不实测一定翻车）

1. **相对路径 + cwd**：installer 的 cwd 是游戏目录，传 `./tmp-x` 会被解析成 `<gameDir>/tmp-x` → `Unable to access jarfile`。
   统一用 `absolute()`，并且剥掉 Windows `canonicalize` 带来的 `\\?\` 前缀（Java 不认）。
2. **Windows 中文输出是 GBK**：不加 `-Dfile.encoding=UTF-8 -Dsun.stdout.encoding=UTF-8 -Dsun.stderr.encoding=UTF-8`
   抓到的是 `��������`。
3. **stdout/stderr 必须并发读**：只读一路，另一路管道写满后子进程永久阻塞在 write。
   本 crate 没开 tokio 的 `process` feature，所以用 `std::process` + 两个读线程 + `spawn_blocking`，
   按字节 `read_until` 再 lossy 解码（老 installer 吐 GBK 也不至于整行丢失）。
4. **Quilt 的 CLI 和 Fabric 不同**（见 1.2）。
5. **Forge 顺手写原版目录**（见 1.3）。
6. **maven-metadata.xml 的顺序不可信**：`52.1.2` 排在 `52.1.20` 后面。必须自己做自然版本排序。
7. **Fabric installer 要求 `-dir` 已存在**。
8. **`--mirror` 只有 Forge 认**，NeoForge 无视（见 1.4）。
9. **GitHub 不可达 → Adoptium 下载必挂**，需要镜像回退（见 4.4）。
10. **JDK8 有 `bin/java` 和 `jre/bin/java` 两份**，去重否则日志里一堆重复。
11. **失败输出末尾信息量极低**：Forge 系真正的异常栈在前面，必须额外摘错误行。
12. **GUI 子进程别弹黑框**：所有 java 调用都设 `CREATE_NO_WINDOW`。

## 7. 本地怎么验

```bash
# 单元测试（离线，默认全跑）
cargo test -p mcdl-core

# 联网验收（会真下载、真跑 installer）
cargo test -p mcdl-core -- --ignored --nocapture --test-threads=1

# 单独验某个加载器
cargo test -p mcdl-core -- --ignored --nocapture real_fabric_install
cargo test -p mcdl-core -- --ignored --nocapture real_adoptium_ensure
```

真装一遍（`--no-assets` 最快，跳过资源文件）：

```bash
cargo run -p mcdl -- install 1.21.1 --loader fabric --no-assets --dir ./tmp-fabric
cargo run -p mcdl -- install 1.20.4 --loader forge  --no-assets --dir ./tmp-forge
```

装完检查 `./tmp-*/versions/`：应该出现上面对照表里的目录，并且里面有同名 json。
