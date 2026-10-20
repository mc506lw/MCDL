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

CI 里实际跑的就是这两条（`rust` job 只管引擎 + CLI）：

```bash
cargo fmt --all -- --check                                       # 格式（硬门禁）
cargo clippy -p mcdl-core -p mcdl --all-targets -- -D warnings   # lint（警告即错误）
cargo test -p mcdl-core -p mcdl                                  # 测试
```

想在本地跑全工作区（含 Tauri 壳）就把 `-p …` 换成 `--workspace`，但 Linux 上那样会需要 Tauri 的系统依赖（见 §2 的 `tauri` job）。

### 当前状态（2026-10-20 实测）

| 门禁 | 结果 | 说明 |
| --- | --- | --- |
| `cargo fmt --check` | ✅ 绿 | `fee1736 style: 统一格式（cargo fmt，无逻辑改动）` 落地，全工作区干净 |
| `cargo clippy -D warnings` | ✅ 绿 | `5d010ec fix(java): 修 clippy 的两条 lint` 之后 0 警告 |
| `cargo test` | ✅ 绿 | 57 passed / 0 failed / 9 ignored |
| `cargo build --release -p mcdl` | ✅ | 4,151,296 字节（3.96 MiB），LTO + strip 后 |

三个 job 现在都是硬门禁，没有 `continue-on-error`。

### 另外两个 job 的本地验证

- `web`：`cd app && pnpm install --frozen-lockfile && pnpm build` → 通过（pnpm 11.25.0 / Node 22.22.3）。
- `tauri`：`cargo check -p mcdl-app` → 通过（前提是 `app/dist` 存在；CI 里会先 `pnpm build`）。

---

## 2. GitHub Actions

### `ci.yml`

| job | 跑什么 | 为什么 |
| --- | --- | --- |
| `rust` | `cargo fmt --check` + `cargo clippy -p mcdl-core -p mcdl --all-targets -- -D warnings` + `cargo test -p mcdl-core -p mcdl` | 只管引擎与 CLI，不装 GTK，跑得快 |
| `web` | `pnpm install --frozen-lockfile` + `pnpm build`（在 `app/` 下） | 前端能不能构建（`tsc && vite build`） |
| `tauri` | 构建前端 + `cargo check -p mcdl-app` | 桌面壳能不能编译；**不做打包**（Linux 上打 Tauri 包要装一堆系统依赖，容易红，留给 release） |

要点：

- **只有 `tauri` job 装 Tauri 的 Linux 依赖**（`libwebkit2gtk-4.1-dev`、`libayatana-appindicator3-dev`、`librsvg2-dev`、`libxdo-dev` 等）。`rust` job 收窄成 `-p mcdl-core -p mcdl` 之后不需要 GTK，改引擎的 PR 几十秒就能出结果。
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
| 本地 clippy 绿、CI 红 | CI 在 Linux 上编译 `#[cfg(unix)]` / `#[cfg(target_os = "linux")]` 分支，Windows 本地根本看不到那些代码 | 按 CI 报的文件行号改；改完在本地 `cargo clippy -p mcdl-core -p mcdl --all-targets` 至少确认没引入新问题 |
| `cargo fmt --check` 报一堆 `Incorrect newline style` | `rustfmt.toml` 要 LF，而 Windows 的 `core.autocrlf=true` 把检出转成了 CRLF（**旧克隆**才会，`.gitattributes` 已经强制 `.rs` 用 LF） | 重新 `git checkout -- .` 让属性生效，或 `git rm -r --cached . && git reset --hard`；再不行临时 `cargo fmt --all` 只会写回 LF，不产生内容改动 |
| `cargo check -p mcdl-app` 报找不到 dist | `app/dist` 不入库 | 先 `cd app && pnpm install && pnpm build` |
| Linux 上 `cargo check` 报 `webkit2gtk` / `appindicator` 找不到 | 缺 Tauri 系统依赖 | 照 `ci.yml` 里 `tauri` job 的 `apt-get install` 列表装 |
| `pnpm install --frozen-lockfile` 失败 | 本地 pnpm 大版本与 lockfile 不匹配（lockfileVersion 9.0，pnpm 11） | `corepack use pnpm@11` 或 `npm i -g pnpm@11` |

> `.gitattributes` 里的 `*.rs text eol=lf` 不是可选项：没有它，Windows 上**全新克隆**出来的 `.rs` 是 CRLF，
> `cargo fmt --check` 会对每个文件报 `Incorrect newline style`，看起来像「仓库没格式化」，其实是行尾被 git 换了。

---

## 5. 格式化基线

`rustfmt.toml` 固定了三条（全部是 stable 选项）：

```toml
edition = "2021"
max_width = 100
newline_style = "Unix"
```

**已经做过一次全量格式化**，历史是分开的两步：

| commit | 内容 |
| --- | --- |
| `fee1736` `style: 统一格式（cargo fmt，无逻辑改动）` | 8 个文件的纯格式化 + 新增 `rustfmt.toml`（205+/118-，全是换行/缩进/尾逗号） |
| `5d010ec` `fix(java): 修 clippy 的两条 lint` | `java/mod.rs` 的 2 条 lint 修复 + 这个文件自己的格式化 |

之后所有人（和 CI）都用同一套规则：

```bash
cargo fmt --all          # 提交前跑一次
cargo fmt --all -- --check   # CI 里跑的就是这条，现在是硬门禁
```

`rustfmt.toml` 之前试过另外两套配置，差异反而更大（`use_small_heuristics = "Max"` → 730 行，`max_width = 120` → 590 行），
现有代码本来就贴着 rustfmt 默认值写，所以「默认 + 100 列」是差异最小的选择。

> 如果以后升级 rustfmt 导致大面积重排，仍然照这个做法：**单独一个 style commit，不要混进功能 PR**。

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
- `rust` job 只跑 `-p mcdl-core -p mcdl`，所以 `app/src-tauri` 的 lint 覆盖靠 `tauri` job 的 `cargo check`（不是 clippy）。如果以后想让桌面壳也过 clippy，在 `tauri` job 里加一步 `cargo clippy -p mcdl-app --all-targets -- -D warnings` 即可。
- Linux 上 `#[cfg(unix)]` 分支在本地 Windows 看不到，第一次 CI 跑红时按日志行号修（现在 clippy 是绿的，但这属于长期注意事项）。
- 三个门禁现在都是硬门禁；唯一「只写不跑」的是 `release.yml`。

---

## 9. 全新克隆验证（2026-10-20）

`git clone . /tmp/mcdl-fresh` 到一个空目录（工作树干净），然后**只照 README / docs 里写的命令**走一遍：

| 命令 | 结果 |
| --- | --- |
| `cargo test -p mcdl-core` | ✅ 57 passed / 0 failed / 9 ignored |
| `cargo run -p mcdl -- versions --release-only` | ✅ 正常列出 26.3 |
| `cargo run -p mcdl -- install 1.20.4 --no-assets --dir ./tmp-fresh` | ✅ 65 文件 / 82.9 MB，232 s，Java 17 |
| `cd app && pnpm install && pnpm build` | ✅ 通过（pnpm 11.25.0，6 s） |
| `cargo fmt --all -- --check` | ✅ 通过（**在 `.gitattributes` 落地之后**） |
| `cargo clippy -p mcdl-core -p mcdl --all-targets -- -D warnings` | ✅ 通过 |
| `cargo test -p mcdl-core -p mcdl` | ✅ 57 passed / 9 ignored |
| `cargo check -p mcdl-app` | ✅ 通过（前提：先 `pnpm build` 生成 `app/dist`） |

这次验证修掉的三类问题（都只动文档 / 仓库配置，没动代码）：

1. **`.gitattributes` 缺失**：Windows 全新克隆出来的 `.rs` 是 CRLF，`cargo fmt --check` 会对每个文件报 `Incorrect newline style`——看起来像「仓库没格式化」，其实是 `core.autocrlf` 换的行尾。加了 `*.rs text eol=lf` 之后新克隆直接绿。
2. **README 没说怎么拿到 `mcdl` 命令**：用法里直接写 `mcdl versions`，但克隆下来并没有这个可执行文件。补了「从源码跑（`cargo run -p mcdl -- …`）」和「`cargo install --path crates/mcdl-cli`」。
3. **`docs/ARCHITECTURE.md` 两处细节和代码对不上**：Forge installer 的命令行漏了目标目录（实际是 `--installClient <gameDir>`）；`ensure` 的落盘位置写的是 `<gameDir>/runtime`，实际是 `<gameDir>/runtime/jre-<major>`。顺手补了 Quilt 的 `install client … --install-dir=` 形态（它和 Fabric 不一样）。

