# 引擎端到端（E2E）矩阵测试报告

- 测试对象：`crates/mcdl-core`（meta / net / download / java）+ `crates/mcdl-cli`
- 构建形态：`cargo build -p mcdl`（debug），直接用 `target/debug/mcdl.exe` 跑，避免 cargo 开销干扰计时
- 环境：Windows，rustc 1.97.1；本机 Java 仅 `D:\java\zulu17`（OpenJDK 17.0.19 / Azul Zulu）
- 测试日期：2026-10-20
- 产物：`tmp-e2e/`（日志 + 两个脚本），所有安装目录与 `tmp-test/` 已删除
- 原则：只跑、只记，**没有改动 `crates/` 下任何源码**。发现的问题按「复现 → 实际 → 期望 → 严重级别」写在下面，由 Lead 分派。
- **复验轮次**：本报告含两轮。**§1 是修复前（第一轮）**；**§8 是修复后（第二轮）复验**——D1（core-net 修）、D2 / D3 / D4a / D4b / D5 / D6a / D6c / D7（Lead 与 core-loader 修）全部在 §8 复验通过；加载器两条（fabric / forge）与第二轮新发现的 **D8** 也在 §8。第二轮使用独立构建目录 `target-e2e`，与 core-loader 的 `target/` 互不干扰。

---

## 1. 矩阵结果（第一轮，修复前）

| # | 用例 | 结果 | 引擎耗时 | 报告 | 磁盘实测 | 备注 |
|---|---|---|---|---|---|---|
| 1 | `1.20.4` 全量（官方源） | ✅ | 173.2s | 3853 文件 / 708.8 MB | 743,772,815 B | 64 个库（与独立 oracle 一致）；3811 个 asset 对象全部落盘且大小正确 |
| 2 | `1.21.1 --no-assets`（官方源，首次） | ✅ | 21.9s | 71 文件 / 89.2 MB | 93,636,582 B | 70 个库（与 oracle 一致）；client.jar SHA1 逐字节一致 |
| 3 | `1.21.1 --no-assets` 二次安装 | ✅ | 1.5s | 0 文件 / 0.0 MB | 不变 | 幂等 |
| 4 | `26.3` 全量（最新正式版） | ✅ | 217.4s | 5222 文件 / 585.6 MB | 614,745,283 B | 74 个库（oracle 一致）；5147 个 asset 对象全部正确；client.jar 41,483,720 B SHA1 一致 |
| 5 | `1.12.2` 全量（老版本） | ✅ | 62.9s | 1326 文件 / 172.5 MB | 182,884,248 B | natives 解压出 11 个 dll、无 META-INF；assets 1290 对象；**但库数 37 < 官方 39（见 D1）** |
| 6 | `1.5.2` 全量（pre-1.6 资产布局） | ⚠️ | 60.0s | 478 文件 / 51.7 MB | 55,927,300 B | 468 个唯一 asset 对象全部正确；**虚拟布局未生成（见 D3）** |
| 7 | `1.13.2 --no-assets` | ⚠️ | 27.7s | 44 文件 / 57.7 MB | 65,827,966 B | 45 个库只解析出 37 个；**java-objc-bridge 的 natives 被去重吃掉（见 D1）** |
| 8 | `1.21.1 --no-assets --source bmclapi` | ✅ | 25.7s | 71 文件 / 89.2 MB | 93,636,582 B | 逐文件 SHA1 与官方源安装**完全相同**；只访问 BMCLAPI（见 §3） |
| 9 | 损坏修复（1 库 + client.jar，同尺寸改 1 字节） | ⚠️ | 16.6s | 1 文件 / 0.1 MB | 不变 | 两个文件都被正确修复（SHA1 恢复），**但 client.jar 的 26.8 MB 修复未计入报告（见 D4b）** |
| 10 | 中断续装（下到 23% 强杀） | ✅ | 30.6s | 43 文件 / 81.5 MB | 93,636,582 B | 跳过 28 / 新下 42；`.part` 全部清理；最终与干净安装逐字节相同 |
| 11 | `install 1.99.9`（不存在） | ✅ | 5s | — | — | exit 1，`Error: 版本清单里没有 1.99.9` |
| 12 | `--loader bogus`（非法加载器） | ✅ | <1s | — | — | exit 1，`Error: 不认识的加载器：bogus` |
| 13 | `--java /nope/java.exe` | ✅ | 6s | — | — | exit 1，`Error: 指定的 Java 不存在：…` |
| 14 | `--dir` 指向一个已存在的文件 | ✅ | <1s | — | — | exit 1，`读写文件失败（…）：当文件已存在时…` |
| 15 | `--concurrency 0` | ✅ | 1.5s | 0 / 0.0 MB | 不变 | 内部按 1 处理，未崩；日志仍打印 `concurrency=0`（D6b） |
| 16 | `--source bogus` | ✅ | 1.5s | 0 / 0.0 MB | 不变 | 静默回退官方源，无提示（D6a） |
| 17 | `--loader fabric` / `--loader forge` | ⏸ 第一轮未测 | — | — | — | 当时 `loader/mod.rs` 还有 4 处 `todo!()`；**第二轮已补测，见 §8.3** |

补充 1：`1.21.1` 的资产总量是 826 MB，为省带宽这一条用 `--no-assets` 跑；资产路径已由 1.20.4 / 26.3 / 1.12.2 / 1.5.2 四条覆盖。

补充 2（构建版本）：上表 1/4/5/6/7 是用 19:01 的构建跑的，当时 `java::detect()` 只认到 Java 17。测试结束前 `core-loader` 更新了 `detect()`/`pick()`，我用新构建复跑了 `1.21.1 --no-assets`（`使用本机 Java 21`，71 文件 / 89.2 MB / 34.6s，目录与旧构建逐字节相同），并单独验证了 `26.3 → Java 25`、`1.12.2 → Java 8`。Java 选择不影响安装产物，只影响日志与耗时，因此上表的文件数/字节数仍然有效。

---

## 2. report 准确性

`report.bytes` 与「实际下载产物」的对比（下载产物 = libraries + assets + client.jar，不含解压出来的 natives、生成的版本 JSON、launcher_profiles.json）：

| 用例 | 下载产物 (B) | 其中 asset index | 扣掉 index 后 | report 显示 | 结论 |
|---|---|---|---|---|---|
| 1.20.4 | 743,718,493 | 437,886 | 743,280,607 = 708.85 MB | 708.8 MB | ✅（差值 = 打印精度） |
| 26.3 | 614,674,427 | 597,035 | 614,077,392 = 585.63 MB | 585.6 MB | ✅ |
| 1.12.2 | 181,000,502 | 143,136 | 180,857,366 = 172.48 MB | 172.5 MB | ✅ |
| 1.5.2 | 54,237,451 | 74,091 | 54,163,360 = 51.65 MB | 51.7 MB | ✅ |
| 1.13.2（无 assets） | 60,455,594 | 0 | 60,455,594 = 57.66 MB | 57.7 MB | ✅ |
| 1.21.1（无 assets） | 93,576,406 | 0 | 93,576,406 = 89.24 MB | 89.2 MB | ✅ |

**结论**：`report.bytes` 在数值上可靠（误差只来自 CLI 打印的一位小数），但存在一个系统性缺口——**asset index 文件的字节从未被计入**（见 D4a）。二次安装一律 0 文件 / 0 字节（§1 #3、#10 后续复跑）。

---

## 3. 下载源（BMCLAPI）取证

只看日志里的 URL 不够（库文件走 `run_batch`，成功时没有逐文件 URL 日志），所以做了两组实证：

1. **死代理负对照**：`HTTPS_PROXY=http://127.0.0.1:9` + `NO_PROXY` 放行 mojang 域名，`--source official` → 5s 失败，错误里是 `GET https://libraries.minecraft.net/...`。证明「官方源确实走 libraries.minecraft.net」且错误信息带 URL。
2. **本地 CONNECT 代理记录全部访问域名**（`tmp-e2e/connect-proxy.py`，只记 `host:port`，不改流量），`--source bmclapi` 跑完整安装：

```
16 bmclapi2.bangbang93.com:443
 9 cmcc.mirrors.ustc.edu.cn:443      ← BMCLAPI 的镜像后端
 7 mirrors.cernet.edu.cn:443
 4 unnys3.sakura.sld.tw:443  ...（其余为 BMCLAPI 的 CDN 节点）
 1 piston-meta.mojang.com:443        ← 清单 + 版本 JSON（按设计永远官方）
 1 piston-data.mojang.com:443        ← 见下
```

**`libraries.minecraft.net` 一次都没出现** → 依赖库确实走了镜像。
`piston-data.mojang.com` 的那一次是 BMCLAPI 自己 302 过去的：`/version/1.21.1/client` → `/v1/objects/<sha1>/client.jar` → `https://piston-data.mojang.com/...`。也就是说**镜像本身不托管 client.jar，会重定向回官方 CDN**；引擎请求的是镜像地址，行为正确，只是拿不到加速。另外 71 个文件的 SHA1 与官方源安装逐文件完全一致（`sha1` 全等），内容没有被镜像改写。

---

## 4. 问题清单

### D1（高 / macOS 会装坏）`inheritsFrom` 合并按 maven name 去重，吃掉同名库的 natives 变体

- **复现**：`mcdl install 1.13.2 --no-assets --dir tmp-e2e/x`（或 1.12.2 全量）
- **实际**：
  - 日志 `版本 JSON 解析完成 version=1.13.2 libraries=37`，而官方 JSON 有 45 个库；1.12.2 是 37 vs 39。
  - 写出的 `versions/1.13.2/1.13.2.json` 里 `ca.weblite:java-objc-bridge:1.0.0` 只剩 1 条且 `natives: null`。
- **影响面**（采样 18 个版本）：`1.12.2 / 1.13.2 / 1.14.4 / 1.15.2 / 1.16.5 / 1.17.1 / 1.18.2` 共 7 个版本的 `java-objc-bridge` 会丢 natives。官方 JSON 里这个名字有两条记录，一条带 `natives`（macOS 的 dylib）、一条不带，后者在后 → 被覆盖。
- **后果**：macOS 上这些版本 natives jar 会下载（artifact 还在），但 `extract_natives` 因为 `natives` 表为空而跳过 → 缺 `libobjc-bridge.dylib`，启动即崩（Windows/Linux 不受影响，所以本次矩阵里没暴露成失败）。
- **根因**：`meta/mod.rs::merge_detail` 无条件按 `l.name` 去重。本意是「子版本覆盖父版本的同名库」，但单版本解析（base 为空）时也生效了。
- **建议**：去重键从 `name` 改成 `(name, artifact.path, natives 形态)`——只有「同名 + 同一个落盘路径 + 同样的 natives 形态」才算覆盖；`download` 层已经按 `dest` 去重，不会因此重复下载。或者仅在 `base.libraries` 非空（真有父版本）时才做覆盖。
- **归属**：`meta/mod.rs`（core-net）。任务期间未改，等指令。
- **状态**：✅ 已修复（第二轮）。`same_library` 去重键改为 `(name, artifact.path, natives 表)`；1.13.2 `libraries` 从 37 回到官方 45、1.12.2 从 37 回到 39，写出的 JSON 里 objc-bridge 恢复两条（一条带 natives）。详见 §8.1。

### D2（中高 / 其它启动器无法启动）写出的版本 JSON 缺 `arguments` / `minecraftArguments`

- **复现**：任意版本安装后看 `versions/<id>/<id>.json`
- **实际**：只有 8 个顶层字段 `id / type / mainClass / assets / assetIndex / downloads / libraries / javaVersion`。1.12.2、1.5.2 没有 `minecraftArguments`，1.13+/1.20.4/26.3 没有 `arguments`；`logging` 也没有。
- **期望**：`write_version_json` 的注释写的是「这样官方启动器、PCL 之类也能直接认这个实例」。但启动命令完全依赖 `arguments`（1.13+）或 `minecraftArguments`（≤1.12.2），缺了就无法启动；`logging` 缺了则 log4j 配置参数指向不存在的文件。
- **建议**：把 `jvm_args` / `game_args` / `legacy_args` / `logging` 一并写回（`Arg`、`ArgValue` 已有 `Serialize`，直接 `serde_json::to_value` 即可）。
- **归属**：`install.rs::write_version_json`（Lead）。
- **状态**：✅ 已修复（第二轮）。1.13.2 写出 `arguments`（game 20 / jvm 9）+ `logging` + `assetIndex.totalSize`；1.12.2 写出 `minecraftArguments`（不混写 `arguments`）。详见 §8.2。

### D3（中 / pre-1.6 没有声音和语言）资产虚拟布局未实现

- **复现**：`mcdl install 1.5.2 --dir tmp-e2e/x`（`1.5.2` 索引 `map_to_resources: true`；`1.6.4` 索引 `virtual: true`）
- **实际**：`assets/` 下只有 `objects/`（468 个唯一对象全部校验通过）和 `indexes/pre-1.6.json`；没有 `assets/virtual/legacy/`，也没有把对象铺到 `assets/<name>`。
- **期望**：`AssetIndex.map_to_resources` → 把对象复制/硬链到 `assets/<name>`；`AssetIndex.virtual_` → 铺到 `assets/virtual/<index id>/<name>`。这是 `docs/ARCHITECTURE.md` 里「assets 为 pre-1.6 的虚拟布局」明确要求的能力，`types.rs` 也预留了两个字段，但目前没人使用。
- **后果**：≤1.5.2 以及 1.6.x 启动后没有声音、没有语言文件。
- **归属**：`install.rs::plan_assets`（Lead）。
- **状态**：✅ 已修复（第二轮）。`map_to_resources` → `<gameDir>/resources/<资源名>`；`virtual` → `assets/virtual/<索引名>/`。1.5.2 实测铺开 749 个文件（索引里 749 个名字全在）。详见 §8.2。

### D4（低 / 记账不准）`report` 漏计两类下载

- **D4a：asset index 从未计入**
  - 复现：任意带 assets 的安装，对比 `report.bytes` 与磁盘。
  - 实际：`plan_assets` 里先 `download_one` 把索引落盘，随后第 5 步 `download_counted` 看到文件已存在 → `added = 0`。1.20.4 漏 437,886 B、26.3 漏 597,035 B、1.12.2 漏 143,136 B（0.06%–0.1%，在 1% 验收线内但语义错）。
  - 建议：索引只在一个地方下载；`download_counted` 应返回真实写入字节，而不是用「前后文件大小差」推断。
- **D4b：同尺寸修复会漏计文件和字节**
  - 复现：装完 1.21.1 后，把 `client.jar` 中间 1 个字节取反（大小不变），重装。
  - 实际：`report` 显示 `新增 1 个文件，共 0.1 MB`（那 1 个是库），client.jar 明明重下了 26.8 MB，`files` 和 `bytes` 都没算——因为 `download_counted` 用 `after - before = 0`，并且 `if added > 0` 才 `downloaded += 1`。
  - 期望：修复场景应报 2 个文件 / 26.9 MB。
  - 建议：同 D4a，让 `download_one` 返回实际写入字节。
- **归属**：`install.rs`（Lead）。
- **状态**：✅ 已修复（第二轮）。1.12.2 全量的 report 从 `172.5 MB / 1326 文件` 变成 `172.6 MB / 1327 文件`（索引 143,136 B 计入）；同尺寸改坏 client.jar 后重装，report 从 `1 文件 / 0.1 MB` 变成 `1 文件 / 9.7 MB`（client.jar 10,180,113 B 正确计入）。详见 §8.2。

### D5（中高，**测试结束前已被修复并复验**）Java 版本不匹配时静默降级，不触发自动获取

> 状态：`core-loader` 在 E2E 期间更新了 `java::detect()` 与 `java::pick()`（19:27–19:31）。下面是发现时（旧构建）的现象，以及修复后的复验结果。

- **发现时的现象**：本机当时只探测到 Java 17，装 `1.21.1`（JSON 要 21）、`26.3`（要 25）、`1.12.2`（要 8）日志都是 `使用本机 Java 17`，Java 阶段从不出现，`report.java` 也是 17。`ensure()`（Adoptium 自动下载）永远不会被触发。
- **根因**：旧 `java::pick` 在「没有 ≥ 要求的运行时」时 `or_else(|| runtimes.iter().max_by_key(...))` 无条件退回最大版本；`install.rs` 的 `needs_java_download` 和 `resolve_java` 都信这个结果。
- **后果（旧构建）**：安装「成功」但启动必失败（26.3 要 Java 25、1.21.1 要 21），日志里没有任何提示。
- **修复后复验**（当前构建，`java::pick` 只返回 `major >= 要求`，`detect()` 现在能扫到 5 个 JDK）：
  - `1.21.1 --no-assets` → `使用本机 Java 21（Azul Zulu）`，71 文件 / 89.2 MB，与旧构建产出的目录**逐字节相同**（93,636,582 B）
  - `26.3` → `使用本机 Java 25（Azul Zulu）`
  - `1.12.2` → `使用本机 Java 8（Azul Zulu）`
  - 若本机一个合适的都没有，现在会规划 `Java` 阶段并落到 `ensure()`（Adoptium），逻辑上正确。
- **归属**：已由 `java/mod.rs`（core-loader）修复，`install.rs` 无需再改。

### D6（低 / 观感与可诊断性）

- **D6a**：`--source bogus` 静默回退官方源，没有任何提示（`mcdl-cli/src/main.rs`）。建议 warn 一句「未知下载源 'bogus'，已使用官方源」。
  - **状态**：✅ 已修复。现在输出 `警告：不认识的下载源 \`bogus\`，已回退到官方源（可选：official / bmclapi）`。
- **D6b**：`--concurrency 0` 实际按 1 跑，但 `download` 的日志打印 `concurrency=0`（日志用的是原始参数，clamp 只用在 `buffer_unordered`）。一行可修，属 core-net 的 `download/mod.rs`，等指令。
  - **状态**：✅ 已修复（core-net）。改为 `concurrency.clamp(1, 64)` 后再打日志：`--concurrency 0` → `concurrency=1`，`--concurrency 1000` → `concurrency=64`。
- **D6c**：错误输出重复：`Error: 读写文件失败（…）：当文件已存在时…` 后面又跟 `Caused by: 当文件已存在时…`（`thiserror` 的 Display 已经包含 source，`anyhow` 又打了一遍）。CLI 层可以 `{}` 代替 `{:?}`。
  - **状态**：✅ 已修复。现在只输出一行 `错误：读写文件失败（…）：当文件已存在时，无法创建该文件。 (os error 183)`。

### D7（低 / 少量耗时）`detect()` 每次安装跑两遍，日志刷 10 行

- **现象**：`install.rs` 的 `needs_java_download` 调一次 `java::detect()`，`resolve_java` 又调一次；每个扫到的 JDK 都以 INFO 打一行，一次安装日志里重复 10 行 `发现 Java ...`。
- **实测开销**：全缓存安装 2.5s（跑两遍 detect）vs 指定 `--java` 后 1.5s → 一次 detect ≈ 0.5s，两次 ≈ 1.0s。
- **建议**：`Installer::run` 里探测一次、把结果传下去；`发现 Java` 降到 debug 级。
- **归属**：`java/mod.rs`（core-loader）+ `install.rs`（Lead），低优先级。
- **状态**：✅ 已修复（第二轮）。`detect()` 每次安装只跑一遍，日志里 `发现 Java` 从 10 行降到 5 行；全缓存安装从 2.5s 降到 1.5s。

### D8（中 / 加载器安装的报告不准）`report` 不含加载器新增的文件

- **发现轮次**：第二轮（加载器首次可测之后）。
- **复现**：`mcdl install 1.20.4 --loader forge --no-assets --dir tmp-e2e/x`，对比 `report.bytes` 与磁盘。
- **实际**：forge 安装后磁盘 179,842,093 B，report 只有 82.9 MB（少报约 96 MB，偏差 54%）；fabric 少报约 4.8 MB。`report.files` 同样不含加载器新增文件。
- **期望**：加载器装完后把新增文件数 / 字节累加进 report（用户关心的是「这次安装实际占了多少盘」）。
- **建议**：`install.rs` 在 `loader::install` 返回后累加 `LoaderResult.files` 的大小；`LoaderResult` 也可以直接带 `bytes`。
- **归属**：`install.rs`（Lead）+ `loader/mod.rs`（core-loader，`files` 已有，只缺字节）。
- **状态**：⏳ 待修（不影响安装正确性，只影响报告）。

---

## 5. 通过项（值得记一笔的验证）

- **规则过滤有独立 oracle**：用 Python 按同一套 `rules` 语义独立算出 windows/x86_64 的允许库数，与实际落盘文件数逐一对照：1.20.4 `64/64`、1.21.1 `70/70`、26.3 `74/74`、1.12.2 `32 artifacts + 3 natives = 35/35`。落盘里没有任何 `natives-linux` / `natives-macos` / `objc-bridge`。
- **内容校验**：1.20.4 / 26.3 / 1.21.1 / 1.12.2 / 1.5.2 的 client.jar SHA1 全部与官方一致；所有 asset 对象按索引逐个比对 hash 目录与 size，缺失 0 个、大小错误 0 个。
- **natives**：1.12.2 解出 11 个 dll（1.86 MB）、1.5.2 解出 9 个（1.68 MB）、1.13.2 解出 5.35 MB 的 Windows natives，`META-INF/` 一律没有落盘，`extract.exclude` 生效。
- **中断续装**：`Stop-Process -Force` 杀在 23%（日志 27/70，落盘 29 个完成 + 14 个 `.part`）后重装，`skipped=28 / downloaded=42`，`.part` 归零，最终目录与干净安装**逐字节相同**（93,636,582 B，73 个文件）。
- **损坏修复**：同尺寸篡改能被 SHA1 发现并重下修复；修复后第三次安装 0 文件 / 0 字节。
- **幂等**：三次重复安装都是 0 文件 / 0 字节，耗时从 21.9s 降到 1.5s。
- **异常路径**：不存在版本 / 非法加载器 / Java 路径不存在 / `--dir` 是文件，全部 exit 1 + 可读错误，**没有一次 panic**。

---

## 6. 测试方法与被踩过的坑

- **harness**：`tmp-e2e/harness.sh`（计时 + 解析 report + 统计磁盘 + `.part` 计数），每个用例一条命令；日志在 `tmp-e2e/logs/`。
- **镜像取证**：`tmp-e2e/connect-proxy.py`，一个只记录 `CONNECT host:port` 的最小隧道代理，配合 `HTTPS_PROXY` 使用。
- **坑 1：Git Bash 的 `kill -9` 杀不掉原生 Windows 进程**。第一次中断测试看起来「成功」了，实际上 mcdl 还在后台把剩下 9 个文件下完并改名，导致「中断现场」和「续装」的结论都是假的。改用 PowerShell `Start-Process -PassThru` + `Stop-Process -Force` 才是真强杀（`HasExited=True`）。
- **坑 2：`report` 的 MB 只有一位小数**（±0.05 MB），小安装（50–60 MB）上光打印误差就有 0.08%，比对时必须把这一项单独扣掉，否则会把精度问题误判成记账 bug。
- **坑 3：判断「库是否齐全」必须自己实现一遍 rules**，否则会把 27 个 macOS/Linux 专用库误报成「缺失」——我第一次的校验脚本就因为「规则不命中 → 默认允许」写错，报了 27 个假缺失。
- **坑 4：BMCLAPI 的库地址是 302 到各镜像节点**（`*.sakura.sld.tw`、`mirrors.cernet.edu.cn` 等），所以「只看到 bmclapi2 一个域名」不代表没走镜像；client.jar 则会 302 回官方 CDN。

---

## 7. 第一轮结论

- 原版安装主链路（元数据 → 规则过滤 → 并发下载 → SHA1 校验 → natives 解压 → 版本 JSON）在 1.5.2 / 1.12.2 / 1.13.2 / 1.20.4 / 1.21.1 / 26.3 六个版本上都能跑通，内容和官方逐字节一致；镜像源、断点续装、损坏修复、异常路径都符合预期。
- **会装坏的三件事**：D1（macOS 丢 natives，1.12.2–1.18.2）、D2（写出的 JSON 缺启动参数，所有版本）、D3（pre-1.6 没有虚拟资产布局）。
- **会误导用户的两件事**：D4（报告漏计 asset index 与同尺寸修复）、D5（Java 版本不匹配却静默成功——**已在测试结束前由 core-loader 修复并复验**）。
- **低优先级**：D6（未知下载源静默回退、`concurrency` 日志值、错误输出重复 `Caused by`）、D7（每次安装探测 Java 两遍，约 1s）。
- 加载器两条（fabric / forge）在测试期间 `loader/mod.rs` 从 4 处 `todo!()` 变成了 0 处，已具备测试条件——**第二轮已补测，见 §8.3**。

---

## 8. 修复后复验（第二轮）

构建：`CARGO_TARGET_DIR=target-e2e cargo build -p mcdl`（与 core-loader 的 `target/` 隔离，避免抢 `mcdl.exe`）。日志：`tmp-e2e/logs/fix-*.log`、`loader-*.log`。

### 8.1 D1 修复复验（core-net 的 `meta/mod.rs`）

改法：`same_library()` 的去重键从 `name` 改成 `(name, artifact.path, natives 表)`，只有三者全同才当作同一条替换。

| 检查项 | 修复前 | 修复后 | 结论 |
|---|---|---|---|
| `1.13.2 --no-assets` 解析出的库数 | 37（官方 45） | **45** | ✅ 与官方一致 |
| `1.12.2 --no-assets` 解析出的库数 | 37（官方 39） | **39** | ✅ 与官方一致 |
| 写出 JSON 里 `java-objc-bridge` 记录数 | 1 条（`natives: null`） | **2 条**：一条 `{"osx":"natives-osx"}`、一条 `null` | ✅ macOS 的 dylib 保住了 |
| `1.12.2` 的 natives 解压 | 11 个 dll | **11 个 dll**（OpenAL32/64、SAPIWrapper_x64/x86、jinput-*、lwjgl/lwjgl64），无 `META-INF/` | ✅ 落到 `versions/1.12.2/natives/` |
| 单元测试 | — | 新增 `merge_keeps_same_name_with_different_natives`（同名不同 natives 形态两条都保留）+ 原有 `merge_replaces_same_named_library` 仍通过 | ✅ `cargo test -p mcdl-core` 52 passed / 0 failed |

`1.13.2` 写出的 JSON：顶层 45 条库，`objc-bridge` 两条（一条 `natives={"osx":"natives-osx"}` + classifier，一条 `natives=null`）。Windows 上这两条都被规则过滤掉，所以下载产物与修复前一致（44 文件 / 57.7 MB）；差异只在「解析出的库表」和 macOS 的解压行为上。

### 8.2 Lead 侧修复复验（D2 / D3 / D4a / D4b / D6 / D7）

| 检查项 | 复验结果 | 结论 |
|---|---|---|
| D2：`1.13.2` 写出的 JSON | 有 `arguments`（game 20 / jvm 9 条）、无 `minecraftArguments`；另有 `logging`、`assetIndex.totalSize=189,762,775` | ✅ |
| D2：`1.12.2` 写出的 JSON | 有 `minecraftArguments`（`--username ${auth_player_name} …`）、**无** `arguments`、有 `logging`、39 条库 | ✅ 两种形态不混写 |
| D2：`1.20.4` 原版 JSON（forge 安装后仍在） | `arguments` 保留、88 条库、`logging` 在 | ✅ 没被加载器步骤破坏 |
| D3：`1.5.2` 虚拟资产 | `已铺开老版本的虚拟资产 copied=749`；`resources/` 下 749 个文件，目录 `icons/ music/ newmusic/ newsound/ pe/ sound/ sound3/ streaming/` + `pack.mcmeta` + `READ_ME_I_AM_VERY_IMPORTANT`；抽查 `music/calm1.ogg` 2,530,812 B、`icons/icon_16x16.png` 3,665 B | ✅ 与索引 749 个名字完全对齐 |
| D3：目录结构 | `assets/{indexes,objects}` + `resources/` + `libraries/` + `versions/1.5.2/`，无多余目录 | ✅ |
| D4a：asset index 计入 | `1.12.2` 全量 report 从 `172.5 MB / 1326 文件` → **`172.6 MB / 1327 文件`**，正好等于下载产物 181,000,502 B（含索引 143,136 B） | ✅ |
| D4b：同尺寸修复 | 把 `1.12.2` 的 client.jar 中间 1 字节取反后重装：report **`1 文件 / 9.7 MB`**（client.jar 10,180,113 B），SHA1 恢复；再装一次 `0 文件 / 0.0 MB` | ✅ |
| D6a：`--source bogus` | `警告：不认识的下载源 \`bogus\`，已回退到官方源（可选：official / bmclapi）` | ✅ |
| D6b：`--concurrency` | `0` → 日志 `concurrency=1`（71 个文件单线程跑了 142.3s，未卡死）；`1000` → `concurrency=64` | ✅ |
| D6c：错误输出 | 只打一行 `错误：读写文件失败（…）：当文件已存在时，无法创建该文件。 (os error 183)`，无重复 `Caused by` | ✅ |
| D7：`detect()` 次数 | 日志里 `发现 Java` 从 10 行降到 **5 行**（每次安装只探测一次）；全缓存安装 2.5s → **1.5s** | ✅ |
| 幂等 | `1.12.2` 修复后再装：0 文件 / 0.0 MB（4.1s）；`fabric` 二次安装：0 文件 / 0.0 MB | ✅ |

### 8.3 加载器矩阵（第二轮补测）

| 用例 | 结果 | 耗时 | 报告 | 磁盘 | 版本目录 | 备注 |
|---|---|---|---|---|---|---|
| `1.21.1 --loader fabric --no-assets` | ✅ | 121.7s | 71 文件 / 89.2 MB | 98,421,714 B | `fabric-loader-0.19.5-1.21.1` | fabric-installer 1.1.2，Java 21；8 条库全部落盘；`mainClass=net.fabricmc.loader.impl.launch.knot.KnotClient`、`inheritsFrom=1.21.1` |
| `1.21.1 --loader fabric`（二次） | ✅ | 52.9s | 0 文件 / 0.0 MB | 不变 | 同上 | 幂等；但仍会重跑一次官方 installer（见下方备注 2） |
| `1.20.4 --loader forge --no-assets` | ✅ | 234.6s | 65 文件 / 82.9 MB | 179,842,093 B | `1.20.4-forge-49.2.13` | forge-installer `--installClient`，Java 17；`mainClass=net.minecraftforge.bootstrap.ForgeBootstrap`、39 条库全部落盘、原版 JSON 未被破坏 |

- 两个安装器都是走官方 headless 模式，日志里没有真正的错误（Forge 的 `Can't Find Class: …` 是它自己的 processor 噪音，不是失败）。
- 产物目录 `versions/` 里同时有原版目录和加载器目录，符合「原版 + 加载器」的官方布局。
- 备注 1：**D8（第二轮新发现，中）**：`report.files / report.bytes` **不含加载器新增的文件**。fabric 少报约 4.8 MB（fabric 库 4.18 MB + installer 缓存 0.21 MB），forge 少报约 **96 MB**（磁盘 179.8 MB vs 报告 82.9 MB，偏差 54%）。加载器装完不更新 `outcome`，`LoaderResult.files` 只被用来填 `installed_version`。建议 `loader::install` 返回的 `files` 累加进 `report.files`、其大小累加进 `report.bytes`（或让 `LoaderResult` 直接带 `bytes`）。这条会打破「report 误差 < 1%」的验收线，但目前只影响加载器安装。
- 备注 2：fabric 二次安装虽然是 0 文件 / 0 字节，但仍花 52.9s——因为每次都重新跑官方 installer（它会重新校验/下载自己的库）。这不是缺陷（行为与官方一致），但如果以后要「已装则跳过」，可以在 loader 层先检查 `versions/<loader-id>/` 是否已存在。

### 8.4 第二轮结论

- 第一轮的 7 条（D1–D7）**全部修复并复验通过**；其中 D1 由 core-net 修，其余由 Lead / core-loader 修。
- 原版六版本 + 镜像源 + 断点续装 + 损坏修复 + 异常路径在第一轮已通过，第二轮抽查（1.12.2 / 1.13.2 / 1.5.2 / 1.20.4）无回归。
- 加载器两条（fabric / forge）**首次通过**：安装器调用、版本目录、库落盘、原版 JSON 保留都正确。
- 唯一遗留是 **D8**（加载器新增文件不计入 report），属记账问题，不影响安装正确性。
