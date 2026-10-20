# `install.rs` 对抗性审查

> 审查对象：`crates/mcdl-core/src/install.rs`（编排层）
> 审查方式：逐行读 + 真机装 5 个版本 + 与 Mojang 官方 JSON 逐字段 diff + 故意制造失败
> 行号基准：本次审查时的工作区状态（`install.rs` 705 行 / `types.rs` 474 行 / `download/mod.rs` 768 行）。
> 若后续行号漂移，按「函数名 + 引用片段」定位。

**结论**：编排层整体是可靠的。四类问题里，第 1 类（阶段计划）在审查过程中已被你修掉并复核通过；
第 3 类（版本 JSON schema）没有致命缺陷，但有一组「非规范字段」和 4 个缺失字段；
第 4 类（逻辑漏洞）有两处会留下磁盘垃圾。**发现 1 个高危缺陷，但根因不在 install.rs，而在 `types.rs` 的反序列化。**

| 级别 | 问题 | 影响面 |
| --- | --- | --- |
| **高** | H1 1.6–1.7.2 的虚拟资源永远不会铺开（`virtual` 字段名不匹配） | 这些版本装完没有声音/语言/音乐，且不报错 |
| 中 | M2 批量下载失败会残留 `.part` 文件 | 每次失败残留最多「并发数」个半截文件 |
| 中 | M3 加载器安装失败会留下半成品 `versions/<id>/` | 其它启动器会列出这个点不开的版本 |
| 低 | L1 `write_version_json` 缺 4 个顶层字段 + 3 个子字段 | 兼容性/信息缺失，不影响启动 |
| 低 | L2 `libraries` 写出 `null`/空数组/空对象 | 非规范 JSON，Gson 容忍、严格解析器不容忍 |
| 低 | L3 `arguments` 里的规则写出 `"os": null, "features": null` | 同上 |
| 低 | L4 1.5.2 的重复库被合并（13 → 12 条） | 两条内容完全相同，无害 |
| 低 | L5 `download_counted` 靠 mtime 判断改写 | 粗粒度时间戳卷上统计漏计 |
| 低 | L6 显式 `--java` 不校验主版本 | 用户传错版本要等到启动才炸 |
| 低 | L7 `materialize_assets` 只判存在、不校验内容 | 虚拟资产损坏后永远不修复 |
| 低 | L8 `Assets` 阶段在无任务时不会发事件 | 阶段轨道留空号（边角） |
| 低 | L9 资源索引在任何阶段事件之前下载 | 慢网络下 UI 停在「读取版本清单」 |
| 低 | L10 D8 目录差分只统计净增长 | 覆盖写/删除会少计；并发安装会串账 |
| 低 | L11 `report.files` 不含铺开的虚拟资源 | 1.5.2 少报 749 个文件 |

---

## 高

### H1. 1.6–1.7.2 的虚拟资源永远不会铺开

**位置**

- 根因：`types.rs:103-111`

  ```rust
  pub struct AssetIndex {
      pub objects: HashMap<String, AssetObject>,
      #[serde(default)]
      pub map_to_resources: bool,
      #[serde(default)]
      pub virtual_: bool,      // ← JSON 里的键是 "virtual"，不是 "virtual_"
  }
  ```

- 消费点：`install.rs:429-435`（选布局）、`install.rs:127-129`（铺开）、`install.rs:444-489`（`materialize_assets`）

**现象**：`serde` 默认按字段名精确匹配，官方索引里写的是 `"virtual": true`，所以 `virtual_` 永远解析成
`false`，`AssetLayout::Virtual` 分支**永远进不去**，`assets/virtual/<索引名>/` 不会被创建。
`map_to_resources` 名字对得上，所以 1.5.2 那条路径是好的。

**证据**（真装 1.6.4，exit 0）：

```
$ mcdl install 1.6.4 --dir ./tmp-review-164
  info  使用本机 Java 8（Azul Zulu）
  版本目录：1.6.4
  新增文件：616 个，共 117.7 MB，用时 114.9s

$ ls tmp-review-164/assets
indexes  objects            ← 没有 virtual/

$ python -c "…读 assets/indexes/legacy.json…"
index keys: ['objects', 'virtual'] | objects: 1120 | virtual: True
```

即：索引明确要求虚拟布局、1120 个对象全都下好了，但一份都没铺开。1.6.4 / 1.7.2 进游戏会没有声音、
语言文件、唱片音乐——而且安装过程**完全成功**，用户只会觉得"这版本有问题"。

**期望行为**：`AssetIndex` 能认出 `"virtual"`；`plan_assets` 返回 `AssetLayout::Virtual`；`materialize_assets`
把 1120 个对象铺到 `assets/virtual/legacy/<资源名>`。

**建议**（二选一，改一处即可）：

1. `types.rs:109` 加 `#[serde(rename = "virtual", default)]`（推荐，根因在这里）；
2. 或者 `install.rs` 里不依赖 `AssetIndex`，直接看 `detail.assets` 是否等于 `"legacy"`（不推荐，治标）。

**覆盖范围**：`assetIndex.id == "legacy"` 的版本（1.6.x、1.7.2；1.7.10 用的是自己的索引，没有 virtual 标记，不受影响）。

---

## 中

### M2. 批量下载失败会残留 `.part` 文件

**位置**：`download/mod.rs:69-166`（`run_batch`，`try_collect` 在 146 行）、`download/mod.rs:179-234`（`fetch_with_retry`）、
触发点 `install.rs:95-102`（依赖库）、`install.rs:113-120`（资源文件）。

**现象**：`buffer_unordered(...).try_collect()` 在**第一个错误**处返回，其余还在飞的 future 被 drop。
`fetch_with_retry` 里的 `remove_file(tmp)` 只在自己的错误分支里执行，被取消的那些任务没有机会清理，
于是留下 `*.part`。

**证据**（把 `libraries/org/lwjgl` 做成普通文件，让 `create_dir_all` 必失败）：

```
$ mcdl install 1.21.1 --no-assets --dir ./tmp-review-err
错误：读写文件失败（./tmp-review-err\libraries\org/lwjgl/lwjgl-freetype/3.3.3）：当文件已存在时，无法创建该文件。(os error 183)
EXIT=1

$ find tmp-review-err -name "*.part" | wc -l
13
$ du -ch $(find tmp-review-err -name "*.part") | tail -1
5.7M    total
```

**影响**：不破坏正确性——重装时 `File::create(tmp)` 会截断重写，实测「移除阻塞后重跑」exit 0 且 `.part` 归零。
但每次失败会在 `libraries/` 里留下最多「并发数」个半截文件，用户看不到、也不知道能删。

**期望行为**：`run_batch` 返回错误前，清掉本批次所有 `*.part`（或给 tmp 包一个 Drop guard）。

**备注**：这是 `download.rs` 的实现问题，但触发点是 `install.rs` 的两处 `run_batch`，所以记在这里。

### M3. 加载器安装失败会留下半成品 `versions/<id>/`

**位置**：`install.rs:151-172`（加载器阶段）、`loader/mod.rs:67`（`loader::install`）、`loader/mod.rs:111`（`before` 快照）。

**现象**：Forge / NeoForge 的 installer 会**先写出版本目录和 json，再去下载库、跑 processors**。
它中途失败（exit 1）时，`versions/<新 id>/` 已经躺在磁盘上了。我们只是把错误往上抛，不做清理。

**证据**（NeoForge 21.1.264，本机 `maven.neoforged.net` 不可达）：installer 正在逐个库超时时，
`ls tmp-neo/versions` 已经能看到 `1.21.1` 和 `neoforge-21.1.264`；最终 exit 1，目录留下。

**影响**

- 其它启动器（或我们未来的启动页）扫 `versions/` 会把它列成一个可选版本，点了必然启动失败；
- 我们自己的重装**不会**把它当成"已安装"（`install.rs` 每次都重新调 `loader::install`），
  但如果某次 installer 恰好 exit 0 却没重写目录，`pick_version_id` 的「期望名已存在」兜底会把旧目录当结果返回
  （`loader/mod.rs` 第 3 档兜底）——这时会把半成品当成功。

**期望行为**：`loader::install` 失败时，用装前的 `before` 差集删掉本次新增的版本目录（或写 `.mcdl-incomplete` 标记，
由扫描方过滤）。删除范围只限 `versions/` 里新增的目录，不要动 `libraries/`（那是可复用的下载成果）。

---

## 低

### L1. `write_version_json` 与官方 JSON 的字段差异

**位置**：`install.rs:524-599`

真装 1.13.2 后与官方 `1.13.2.json` 逐字段对比（官方 26431 字节 / 我们 33616 字节，差异主要来自 pretty-print）：

| 差异 | 官方 | 我们 | 影响 |
| --- | --- | --- | --- |
| `time` / `releaseTime` | 有 | 无 | 启动器显示"未知时间"，不影响启动 |
| `minimumLauncherVersion` | 有（1.13.2 = 21） | 无 | 严格启动器可能按 0 处理；**缺失比写错安全** |
| `complianceLevel` | 有（0） | 无 | 只影响资源合规级别提示 |
| `downloads.server`（1.5.2 还有 `windows_server`） | 有 | 无 | 只影响"用这个版本开服"，我们本来只装客户端 |
| `javaVersion.component` | `"jre-legacy"` / `"java-runtime-delta"` | 只有 `majorVersion` | 官方启动器靠 component 选 JRE；PCL/HMCL 只看 majorVersion |
| `logging.client.type` | `"log4j2-xml"` | 无 | 少数启动器靠 type 决定怎么替换 `${path}` |

**根因**：`VersionDetail`（`types.rs:68-83`）和 `LoggingFile`（`types.rs:120-123`）根本没建模这些字段，
`write_version_json` 只能写它拿得到的东西。所以要补就得先扩 `types.rs` + `parse_detail`，属于跨模块改动。

**判断**：**不影响启动**。`arguments` / `minecraftArguments` / `logging.argument` / `logging.file` / `assetIndex` /
`downloads.client` / `mainClass` 都与官方一致（见「已检查无问题」），这四个字段是信息性/兼容性的。

### L2. `libraries` 写出大量 `null` 与空容器

**位置**：`install.rs:588`（`serde_json::to_value(&detail.libraries)`），源头 `types.rs:128-168` 的 `#[serde(default)]` 但没有 `skip_serializing_if`。

**证据**（1.13.2，45 条库）：

```
值为 null/[]/{} 的库字段计数: {'extract': 43, 'natives': 37, 'rules': 43}
downloads.classifiers: {}   → 45 条全都有
downloads.artifact: null    → 1.5.2 里 3 条（只有 classifiers 的 jinput-platform 等）
```

对比官方第一条：

```json
// 官方
{"name":"com.mojang:patchy:1.3.9","downloads":{"artifact":{…}}}
// 我们
{"downloads":{"artifact":{…},"classifiers":{}},"extract":null,"name":"com.mojang:patchy:1.3.9","natives":null,"rules":[]}
```

**影响**：Gson 系（PCL/HMCL）把 `null` 当缺省，能正常读；严格类型解析器（Rust/Python 的 `dict`→结构体）遇到
`"natives": null` 或 `"artifact": null` 会直接报类型错误。**目前不会坏，但它让我们的 JSON 比官方更"脏"**，
以后接第三方启动器或写严格校验时会踩。

**建议**：在 `types.rs` 给这几个字段加 `#[serde(skip_serializing_if = "Option::is_none")]` /
`skip_serializing_if = "Vec::is_empty"` / `skip_serializing_if = "HashMap::is_empty"`（只影响序列化，不影响反序列化）。
注意 `LibraryArtifact.url` 是 `String` 不是 `Option`，它不会变 null（实测所有版本 artifact 都有 url）。

### L3. `arguments` 里的规则多写 `"os": null, "features": null`

**位置**：`install.rs:559-568`，源头 `types.rs:162-168`。

**证据**（1.13.2 `arguments.game[18]`）：

```json
// 官方
{"rules":[{"action":"allow","features":{"is_demo_user":true}}],"value":"--demo"}
// 我们
{"rules":[{"action":"allow","features":{"is_demo_user":true},"os":null}],"value":"--demo"}
```

去掉 null 之后，1.13.2 的 `arguments`（game 20 条、jvm 9 条）与官方**逐条完全相等**。同 L2，属于规范化问题。

### L4. 1.5.2 的重复库被合并（13 → 12 条）

官方 1.5.2 里 `net.java.jinput:jinput-platform:2.0.5` 出现了两次，两条**完全相同**（我逐字段比过）。
`meta::merge_detail` 的 `same_library` 去重把它们并成一条。行为正确，只是提醒：**这不是丢数据**，
是官方 JSON 自己重复了；两个条目的内容一模一样，启动时 classpath 去重也不会多任何东西。

### L5. `download_counted` 靠 mtime 判断"是否被改写"

**位置**：`install.rs:295-317`

**逻辑**：下载前后各取一次 `(len, modified)`；`mtime` 变了就算"重写过"，按新大小计；没变就算 0。

**风险**：

- **粗粒度时间戳**：FAT32/exFAT 是 2 秒精度，部分 SMB/NAS 是 1 秒。同一 tick 内完成的"删了重下"会被判成没变，
  于是 `bytes` 少计、`downloaded` 也不 +1（`install.rs:136-138`）。用户看到的"新增文件数"会偏小。
- `modified()` 在两边都失败时（`None == None`）也会判成没变。
- 反过来，如果文件被写成**更小**的内容，`new_len.max(len)` 会高估。

**影响**：只影响 `InstallReport` 的统计，不影响安装结果。修法要么让 `download::download_one` 返回实际写入字节数
（契约变更），要么在前后各算一次 SHA1（大文件太贵）。**建议记录在案，暂不改**。

### L6. 显式 `--java` 不校验主版本

**位置**：`install.rs:258-266`

用户传 `--java <Java 8>` 装 1.21.1 时，我们只检查路径存在就直接用。之后 vanilla 阶段能过，
加载器 installer 或游戏启动才会炸（`UnsupportedClassVersionError`），错误现场离"用户传错参数"很远。

**期望**：拿到 `major` 后，如果显式路径能探测出版本且低于需求，至少打一条 `warn`（不必失败，用户可能有特殊理由）。

### L7. `materialize_assets` 只判存在、不校验内容

**位置**：`install.rs:464-482`

`if dst.exists() { continue; }` —— 虚拟资产被截断/损坏后永远不会修复；单文件 `copy` 失败只 `warn` 不失败，
最终 `Ok(())`。1.6/1.7 的"某个声音没声"会非常难查。

**建议**：至少把失败条数汇总成一条 `warn`；要更稳就比对源对象的大小。

### L8. `Assets` 阶段在无任务时不会发事件

**位置**：`install.rs:111-124`

`plan_stages` 只要 `!skip_assets` 就排进 `Assets`，但 `run` 只在 `!asset_jobs.is_empty()` 时 `begin(Assets)`。
目前所有正式版都有非空 assetIndex，所以是边角；但如果哪天遇到 `assetIndex` 缺失或 0 对象的版本，
阶段轨道会从 `2/5` 直接跳到 `4/5`。建议无条件 `begin(Assets)`，内部再判空（与 Java 阶段"计划了就要发"的处理保持一致）。

### L9. 资源索引的下载发生在任何阶段事件之前

**位置**：`install.rs:87-91` → `plan_assets` 内部 `install.rs:413`

索引（1.21.1 是 449 KB）在 `begin(Libraries)` 之前就下完了，这段时间 UI 还停在「读取版本清单」。
影响很小（一次请求），但严格说属于"阶段外的工作"。

### L10. D8 目录差分只统计净增长

**位置**：`install.rs:155-168` + `tree_stats`（`install.rs:626-647`）

- 用 `after.1.saturating_sub(before.1)`，所以 installer **覆盖写**已有文件（Forge 会重打 client jar）或
  **删除**文件时只会少计/不计，不会多计——方向是安全的；
- `tree_stats` 统计的是整个 `libraries/` + `versions/`。如果同一目录有并发安装（我们没有目录锁），
  另一路的写入会被算进本路。
- 实测校准：全新目录装 Fabric（--no-assets）报「新增文件：80 个」，`libraries/`+`versions/` 实际 81 个文件，
  差的 1 个正是 `write_version_json` 在快照之后写的 `versions/1.21.1/1.21.1.json`。**常规路径准确。**

### L11. `report.files` 不含铺开的虚拟资源

1.5.2 真装：`新增文件：479 个`（唯一对象 + 库 + 本体），但实际还铺开了 **749 个** `resources/` 文件。
口径上"新增文件"应该含它们（D8 的思路可以复用）。只影响展示。

---

## 已检查，无问题（明确覆盖面）

下面这些我**主动验证过**，确认当前实现是对的，不需要改：

1. **`skip_assets` 时不会调 `plan_assets`**：`install.rs:87-91` 是 if/else 分支；实测 `install 1.13.2 --no-assets`
   之后 `assets/` 下 0 个文件（索引也没下）。
2. **依赖库下到一半失败，不会写出版本 JSON**：`write_version_json` 是第 9 步（`install.rs:175`），
   前面任何 `?` 都直接返回。实测失败后 `versions/` 是空目录，没有 `<id>.json`。
3. **单文件失败会清 `.part`**：`download.rs:218/225` 在两条错误分支都删了 tmp；批量取消的残留见 M2。
4. **重试可恢复**：制造失败后移除阻塞重跑，exit 0，`.part` 归零，库和本体都补齐。
5. **`maven_to_path` / `arch_bits`**：与 Maven 布局、`${arch}` 替换规则一致，单测覆盖。
6. **`rules_allow` 在 `plan_libraries` 与 `extract_natives` 两处用法一致**：都先过滤再取 classifier，
   不会出现"下载了但没解压"或反之。
7. **BMCLAPI 源下 `downloads.client` 仍写官方 URL**：`plan_client` 只改下载地址，`write_version_json`
   写的是元数据里的官方 URL（`install.rs:548-557`），JSON 里不会出现镜像地址——正确。
8. **`arguments` 内容正确**：1.13.2 官方/我们 = game 20/20、jvm 9/9，去掉 null 字段后逐条完全相等。
9. **`minecraftArguments` 内容正确**：1.5.2、1.6.4 与官方字符串完全一致。
10. **`logging` 正确**：`argument` 与 `file`（id/url/sha1/size）与官方完全一致，只缺 `type`（L1）。
11. **`assetIndex` / `assets` / `mainClass` / `downloads.client` 与官方一致**（1.5.2、1.6.4、1.13.2 三个版本核对）。
12. **`libraries` 语义正确**：递归去掉 null/空容器后，1.13.2（45/45）、1.6.4（21/21）与官方逐条相同；
    1.5.2 只差那条重复项（L4）。
13. **阶段计划（第 1 类）已被你修复，复核通过**：`resolve_java` 现在拿 `&mut Vec<StageKind>`，
    预估偏低时 `plan.insert(1, Java)`，且"计划有 Java 但本机够用"时也会发一次 `检查 Java 运行时`，不留空号。
    实测 26.3（JSON 要 Java 25、字符串推断 21、机器上只暴露 Java 21）：
    ```
    [1/3] 读取版本清单
    [2/4] 获取 Java 运行时      ← 修复前这里是 [4/4]，后面还会跳回 [2/3]
    [3/4] 下载依赖库
    [4/4] 下载游戏本体
    ```
    序号已单调递增。唯一残留：第一个事件仍然是 `1/3`，`total` 在中途从 3 变 4（因为 JSON 要拿到才知道 Java 版本）。
    这是"不额外发一次元数据请求"的必然代价，UI 只要允许 `total` 增长即可；若要彻底消除，得在 `Meta` 之后
    先取一次版本 JSON 再规划（多一次请求，不建议）。
14. **幂等性**：同一目录连装 fabric→fabric→forge，版本 id 相同、第二次 0 新增、所有 json sha1 不变
    （task-5 已验，本轮未变）。
15. **`Installer::run` 的 future 是 `Send`**：`download.rs` 有编译期断言测试，Tauri 的 `spawn` 契约没破。
16. **`plan_stages` 两个单测与实现一致**（`skip_assets` 不排 Assets、有 loader 才排 Loader）。

---

## 建议的处理顺序

1. **H1**（`types.rs` 一行 `rename`）——唯一一个"装完不能玩"的问题，改一行，值得立刻做；
2. **M3**（加载器失败清理版本目录）——用户会看到点不开的版本，属于可信度问题；
3. **M2**（`.part` 残留）——不致命，但每次失败都留垃圾，适合顺手做；
4. **L1/L2/L3**（schema 规范化）——建议一起做：扩 `VersionDetail`/`LoggingFile` 字段 + 给库结构加
   `skip_serializing_if`，一次改完再复跑本报告的 diff 脚本；
5. 其余低危按需。

## 复现方式（证据可重跑）

```bash
# 1) H1：1.6.4 虚拟资源
mcdl install 1.6.4 --dir ./tmp-review-164
ls ./tmp-review-164/assets            # 没有 virtual/
python -c "import json;d=json.load(open('tmp-review-164/assets/indexes/legacy.json'));print(d.get('virtual'),len(d['objects']))"

# 2) M2：.part 残留（把 org/lwjgl 做成文件，让建目录必失败）
mkdir -p tmp-review-err/libraries/org && echo x > tmp-review-err/libraries/org/lwjgl
mcdl install 1.21.1 --no-assets --dir ./tmp-review-err ; find tmp-review-err -name '*.part' | wc -l

# 3) 阶段序号（伪造只含 Java 21 的 JAVA_HOME，装要 Java 25 的 26.3）
env -u JAVA_HOME PATH="/c/Windows/system32:/c/Windows" JAVA_HOME='E:\tmp\fakehome\zulu21' \
  mcdl install 26.3 --no-assets --dir ./tmp-review-263

# 4) schema diff：装 1.13.2 / 1.5.2 后与官方 JSON 比
mcdl install 1.13.2 --no-assets --dir ./tmp-review-1132
mcdl install 1.5.2 --dir ./tmp-review-152
```

本次审查保留了两个证据目录（可随时删）：`tmp-review-164/`（H1 证据）、`tmp-review-152/`（1.5.2 的
`resources/` 749 个文件，证明 `map_to_resources` 那条路是通的）。
