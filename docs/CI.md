# CI 与发布

这个仓库有三道门禁 + 一套打 tag 出包的流程。文件：

| 文件 | 作用 |
| --- | --- |
| `.github/workflows/ci.yml` | push 到 `main` / 所有 PR：rust（fmt + clippy + test）、web（pnpm build）、tauri（cargo check） |
| `.github/workflows/release.yml` | 打 `v*` tag：CLI 三平台二进制 + Tauri 三平台安装包，落到 draft release |
| `rustfmt.toml` | 格式化基线（stable-only 选项） |
| `clippy.toml` | clippy 基线（`msrv = "1.82"`，与 `Cargo.toml` 的 `rust-version` 一致） |

---

## 1. 三道门禁

```bash
cargo fmt --all -- --check                              # 格式
cargo clippy --workspace --all-targets -- -D warnings  # lint（警告即错误）
cargo test --workspace                                  # 测试
```

`--workspace` 包含 `app/src-tauri`，所以 Linux 上跑 clippy / test 需要 Tauri 的系统依赖（`ci.yml` 的 `rust` job 里已经装了，见 §2）。

### 当前状态（2026-10-20 实测）

| 门禁 | 结果 | 说明 |
| --- | --- | --- |
| `cargo fmt --check` | ❌ 红 | **56 个 hunk / 9 个文件 / 303 行**。仓库还没做过全量格式化，见 §5 |
| `cargo clippy -D warnings` | ❌ 红 | **2 条**，都在 `crates/mcdl-core/src/java/mod.rs`：`:100` `needless_lifetimes`（`pick<'a>` 的生命周期可以省略）、`:359` `manual_pattern_char_comparison`（`split(\|c\| c == '.' \|\| …)` 改成 `split(['.', '_', '-', '+'])`） |
| `cargo test --workspace` | ✅ 绿 | 56 passed / 0 failed / 9 ignored |
| `cargo build --release -p mcdl` | ✅ | 4,151,296 字节（3.96 MiB），LTO + strip 后 |

**格式化那一步在 CI 里是「只提示不拦」**：`cargo fmt --check` 这一步带 `continue-on-error: true`，所以 PR 不会因为历史格式被挡。等 §5 的格式化提交落地后删掉这个标记即可。

clippy 的 2 条警告修掉之后，`rust` job 就是全绿的硬门禁。

### 另外两个 job 的本地验证

- `web`：`cd app && pnpm install --frozen-lockfile && pnpm build` → 通过（pnpm 11.25.0 / Node 22.22.3）。
- `tauri`：`cargo check -p mcdl-app` → 通过（前提是 `app/dist` 存在；CI 里会先 `pnpm build`）。

---

## 2. GitHub Actions

### `ci.yml`

| job | 跑什么 | 为什么 |
| --- | --- | --- |
| `rust` | `cargo fmt --check`（提示）、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace` | 引擎 + CLI + Tauri 壳的编译、lint、单测 |
| `web` | `pnpm install --frozen-lockfile` + `pnpm build`（在 `app/` 下） | 前端能不能构建（`tsc && vite build`） |
| `tauri` | 构建前端 + `cargo check -p mcdl-app` | 桌面壳能不能编译；**不做打包**（Linux 上打 Tauri 包要装一堆系统依赖，容易红，留给 release） |

要点：

- `rust` 和 `tauri` 都会先 `apt-get install` Tauri 的 Linux 依赖（`libwebkit2gtk-4.1-dev`、`libayatana-appindicator3-dev`、`librsvg2-dev`、`libxdo-dev` 等）。`rust` 也要装，因为 `--workspace` 会编译 `mcdl-app`。
- `tauri` job 必须先 `pnpm build`：`tauri.conf.json` 的 `frontendDist` 是 `../dist`，而 `app/dist` 被 `.gitignore` 忽略，不构建前端 `cargo check` 会因为找不到 dist 而失败。
- 缓存用 `Swatinem/rust-cache@v2`（`rust` 与 `tauri` 用不同的 `shared-key`，避免互相覆盖）；pnpm 的缓存由 `actions/setup-node` 的 `cache: pnpm` 负责。
- `concurrency` 会在同一分支连续 push 时取消旧的 run。

### `release.yml`

`v*` tag 触发，三个 job：

1. `cli`（矩阵）：`cargo build --release -p mcdl --target …`，产物 `mcdl` / `mcdl.exe`，上传 artifact。
   - linux `x86_64-unknown-linux-gnu`
   - macOS `aarch64-apple-darwin` + `x86_64-apple-darwin`
   - windows `x86_64-pc-windows-msvc`
2. `tauri`（矩阵 ubuntu / macOS / windows）：`tauri-apps/tauri-action@v0`，用 `tagName` 把安装包挂到 draft release。
3. `publish`：取回 CLI artifact，用 `softprops/action-gh-release@v2` 挂到**同一个** tag 的 draft。

> 这份流程**没有在本地跑过**（需要 GitHub runner），只做了 YAML 校验。
> 首次真跑注意：`tauri` 和 `publish` 都会写同一个 tag 的 draft，谁先创建都行（另一方会往上加资产）；极小概率两个 job 同时创建撞 422，重跑一次 `publish` 即可（幂等）。

---

## 3. 为什么 `#[ignore]` 的联网测试不进 CI

测试里有 **9 个** `#[ignore]` 用例，全部需要真实网络（Mojang / Fabric / Forge / Adoptium）：

| 用例 | 依赖 |
| --- | --- |
| `net::tests::fetch_official_manifest_over_network` | piston-meta |
| `meta::tests::fetch_detail_from_official_network` | piston-meta |
| `download::tests::download_real_file_then_skip_on_second_run` | piston-data（小文件） |
| `download::tests::download_reports_hash_mismatch_after_retry` | piston-data（故意写错哈希，重试 2 次） |
| `download::tests::download_real_client_jar` | piston-data（约 26 MB） |
| `java::tests::real_adoptium_ensure` | Adoptium（约 40 MB JRE） |
| `loader::tests::real_fabric_install` | Fabric maven + 本机 java |
| `loader::tests::real_fabric_version_lists` | Fabric meta API |
| `loader::tests::real_other_version_lists` | Forge / NeoForge / Quilt meta API |

原因：

- CI 的 runner 出网策略不受我们控制，官方 CDN 偶发慢/限流会让门禁随机变红；
- 这些用例会真下载几十 MB，既慢又费流量；
- `loader::real_fabric_install` 还要求 runner 上装了 Java，而 CI 里并没有装。

`cargo test --workspace` 默认就不跑它们（输出里 `56 passed; 9 ignored`），**CI 不需要额外参数**。想手动跑：

```bash
cargo test -p mcdl-core -- --ignored                          # 全部
cargo test -p mcdl-core -- --ignored download::tests::        # 只跑某个模块
```

---

## 4. 本地复现 CI 失败

按顺序跑 §1 的三条命令即可，CI 就是它们。几个常见差异：

| 现象 | 原因 | 处理 |
| --- | --- | --- |
| 本地 clippy 绿、CI 红 | CI 在 Linux 上编译 `#[cfg(unix)]` / `#[cfg(target_os = "linux")]` 分支，Windows 本地根本看不到那些代码 | 按 CI 报的文件行号改；改完在本地 `cargo clippy --workspace --all-targets` 至少确认没引入新问题 |
| `cargo check -p mcdl-app` 报找不到 dist | `app/dist` 不入库 | 先 `cd app && pnpm install && pnpm build` |
| Linux 上 `cargo check` 报 `webkit2gtk` / `appindicator` 找不到 | 缺 Tauri 系统依赖 | 照 `ci.yml` 里的 `apt-get install` 列表装 |
| `pnpm install --frozen-lockfile` 失败 | 本地 pnpm 大版本与 lockfile 不匹配（lockfileVersion 9.0，pnpm 11） | `corepack use pnpm@11` 或 `npm i -g pnpm@11` |

---

## 5. 格式化基线

`rustfmt.toml` 现在固定了三条（全部是 stable 选项）：

```toml
edition = "2021"
max_width = 100
newline_style = "Unix"
```

**为什么没有顺手跑 `cargo fmt --all`**：当前有 303 行格式差异，散在 9 个文件里（`loader/mod.rs` 26 处、`meta/mod.rs` 9 处、`install.rs` 6 处、`download/mod.rs` 6 处、`java/mod.rs` 4 处、`net/mod.rs` 2 处、`types.rs` / `paths.rs` / `mcdl-cli/src/main.rs` 各 1 处）。如果混在功能 PR 里，review 会完全被格式噪音淹没。

后续单独做一次（建议单独一个 PR、只有一个 commit）：

```bash
cargo fmt --all
git add -A && git commit -m "style: 全量 rustfmt（无逻辑改动）"
```

然后把 `.github/workflows/ci.yml` 里 `cargo fmt --check` 那一步的

```yaml
continue-on-error: true
```

删掉——从那一刻起格式就是硬门禁。

> 备选配置也试过：`use_small_heuristics = "Max"` 会让差异涨到 730 行，`max_width = 120` 涨到 590 行。
> 现有代码本来就贴着 rustfmt 默认值写，所以上面这份「默认 + 100 列」是差异最小的选择（303 行）。

---

## 6. 发布流程

```bash
git tag v0.1.0
git push origin v0.1.0
```

1. `release.yml` 被触发，四个 `cli` 构建 + 三个 `tauri` 构建并行跑；
2. 产物进入 **draft release**（`MCDL v0.1.0`）：
   - `mcdl-x86_64-unknown-linux-gnu`
   - `mcdl-aarch64-apple-darwin`、`mcdl-x86_64-apple-darwin`
   - `mcdl-x86_64-pc-windows-msvc.exe`
   - `.msi` / `.exe`（NSIS）、`.dmg` / `.app.tar.gz`、`.deb` / `.AppImage`（取决于 runner 与 `bundle.targets = "all"`）
3. 人工检查 draft：二进制能跑（`mcdl versions`）、安装包能装；
4. 在 GitHub 上点 Publish。

版本号来自 `Cargo.toml` 的 `workspace.package.version` 与 `app/src-tauri/tauri.conf.json` 的 `version`，打 tag 前记得两处一起改。

---

## 7. YAML 校验

```bash
python -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))"
python -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml'))"
```

两个文件都能解析。注意：PyYAML 按 YAML 1.1 会把裸 `on:` 读成布尔键 `True`（GitHub Actions 自己按 YAML 1.2 处理，不受影响）。想看到 `on` 就给它加引号，或者用 `yamllint`。

---

## 8. 已知风险 / TODO

- `release.yml` 未真跑过；第一次打 tag 时重点看 `tauri-action` 与 `publish` 的先后顺序。
- `rust` job 在 Linux 上会编译 `mcdl-app`，第一次跑要装 GTK 依赖 + 编译 Tauri，首轮约 5–8 分钟；之后靠 `Swatinem/rust-cache` 会快很多。如果嫌慢，可以把 `rust` job 收窄成 `-p mcdl-core -p mcdl-cli`，把 `mcdl-app` 完全交给 `tauri` job。
- clippy 目前红的 2 条都在 `java/mod.rs`，修掉后 `rust` job 即为硬门禁。
- 格式化提交（§5）落地前，`cargo fmt --check` 只提示不拦。
