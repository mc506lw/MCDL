//! OptiFine：只做**真实可用**的那部分，其余如实说明。
//!
//! 先说清楚现实约束（都是实测/读官方文件得出的）：
//!
//! - OptiFine 不在 Modrinth / CurseForge，只能从 `optifine.net` 下载；
//!   下载要经过 `adloadx?f=…` → `downloadx?f=…&x=…` 这一层跳转页。
//! - 官方 installer 的 `Main-Class` 是 `optifine.InstallerFrame`，它的 `main`
//!   完全忽略命令行参数（字节码里没有读 `args`），**没有 `--help` / `--install`**。
//! - 许可证不允许再分发：我们只从官方下载，缓存到 `<gameDir>/.mcdl/optifine/`，
//!   界面必须写明「文件来自 optifine.net 官方，本工具不托管」。
//!
//! 所以这里走的是「**不跑安装器**」的路线：
//!
//! - **Forge + 1.13 及以后**：OptiFine 的官方 jar 本身就是 Forge mod
//!   （内含 `META-INF/mods.toml`，`modLoader="javafml"`、`modId="optifine"`），
//!   直接放进模组目录就能被 Forge 加载（隔离开时是 `versions/<id>/mods/`）。
//!   这一步会真的下载并校验。
//! - **原版 / Forge + 1.12 及更早**：需要 patch 原版 jar 并生成版本 JSON，
//!   本工具不自动做；只提供「下载官方安装器 → 用户自己点 Install」。
//! - **Fabric / Quilt / NeoForge**：OptiFine 不能直接加载（Fabric 系要第三方
//!   OptiFabric），本工具不假装支持。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::download;
use crate::error::{Error, IoContext, Result};
use crate::isolate;
use crate::net::HttpClient;
use crate::paths::GamePaths;
use crate::types::LoaderKind;

use super::ExtraStatus;

/// 官方下载页。界面上要把这个地址给用户，方便自己核对。
pub const DOWNLOADS_PAGE: &str = "https://optifine.net/downloads";

/// 解析 optifine.net 页面（下载页 / 跳转页）的单次超时。
const NET_TIMEOUT: Duration = Duration::from_secs(20);

/// 真正下载 jar 的超时。7 MB 的安装器在慢网络上也不该被 20 秒掐断。
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(180);

/// 一个官方安装器至少这么大（最小的是 1.7 时代的几 MB）。比这小说明下到的是错误页。
const MIN_INSTALLER_BYTES: usize = 512 * 1024;

/// OptiFine 的安装方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptifineMode {
    /// 官方 jar 就是 Forge mod：放进 mods/ 即可
    ModsFolder,
    /// 需要用户手动运行官方安装器
    Manual,
    /// 这个加载器组合根本不适用
    Unsupported,
}

/// 从下载页解析出来的一个构建。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptifineBuild {
    pub mc_version: String,
    /// `1.21.1_HD_U_J1` 里的 `J1`（pre 构建会带 `_preN`）
    pub edition: String,
    /// `OptiFine_1.21.1_HD_U_J1.jar`
    pub filename: String,
    /// 带跳转页的下载地址（不是最终 jar 地址）
    pub adloadx_url: String,
    /// 官方下载页，给用户核对用
    pub download_page: String,
    /// 预览版（`_preN`）
    pub pre: bool,
}

/// 「这个组合能不能自动装」的结论。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptifinePlan {
    /// 能不能自动装（只有 `ModsFolder` 为 true）
    pub supported: bool,
    pub mode: OptifineMode,
    /// 给用户看的一句话
    pub reason: String,
    /// optifine.net 上有没有这个 MC 版本的构建
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<OptifineBuild>,
    /// 官方下载页
    pub download_page: String,
    /// 能不能提供官方安装器（有构建就能下载；下载本身要联网）
    pub manual_possible: bool,
}

impl Default for OptifinePlan {
    fn default() -> Self {
        Self {
            supported: false,
            mode: OptifineMode::Unsupported,
            reason: "未查询".to_string(),
            build: None,
            download_page: DOWNLOADS_PAGE.to_string(),
            manual_possible: false,
        }
    }
}

/// 自动安装的结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptifineOutcome {
    pub status: ExtraStatus,
    pub message: String,
    pub mode: OptifineMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<OptifineBuild>,
    /// 官方安装器落盘位置（`<gameDir>/.mcdl/optifine/`）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installer_path: Option<PathBuf>,
    /// 放进 `mods/` 的那份（只有自动安装成功时才有）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mod_path: Option<PathBuf>,
    pub bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    /// 需要用户手动完成时，具体该做什么
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_hint: Option<String>,
}

impl OptifineOutcome {
    fn new(status: ExtraStatus, message: impl Into<String>, mode: OptifineMode) -> Self {
        Self {
            status,
            message: message.into(),
            mode,
            build: None,
            installer_path: None,
            mod_path: None,
            bytes: 0,
            sha1: None,
            manual_hint: None,
        }
    }

    fn with_build(mut self, build: OptifineBuild) -> Self {
        self.build = Some(build);
        self
    }
}

// ---------------------------------------------------------------- 支持矩阵

/// 这个「加载器 + MC 版本」组合能不能自动装 OptiFine，为什么。
pub fn support(loader: Option<LoaderKind>, mc_version: &str) -> (OptifineMode, String) {
    match loader {
        Some(LoaderKind::Forge) => match at_least_1_13(mc_version) {
            Some(true) => (
                OptifineMode::ModsFolder,
                "Forge 已装：OptiFine 的官方 jar 本身就是 Forge mod，放进 mods/ 即可加载"
                    .to_string(),
            ),
            Some(false) => (
                OptifineMode::Manual,
                "1.12 及更早的 OptiFine 需要 patch 原版 jar 并生成版本 JSON；本工具不自动做，\
                 请用官方安装器手动安装"
                    .to_string(),
            ),
            None => (
                OptifineMode::Unsupported,
                "识别不出这个 Minecraft 版本号（快照一般没有 OptiFine 构建）".to_string(),
            ),
        },
        Some(LoaderKind::Fabric) => (
            OptifineMode::Unsupported,
            "OptiFine 不能直接被 Fabric 加载，需要第三方 OptiFabric；本工具不自动装".to_string(),
        ),
        Some(LoaderKind::Quilt) => (
            OptifineMode::Unsupported,
            "OptiFine 不能直接被 Quilt 加载，需要第三方兼容层；本工具不自动装".to_string(),
        ),
        Some(LoaderKind::NeoForge) => (
            OptifineMode::Unsupported,
            "OptiFine 不支持 NeoForge".to_string(),
        ),
        None => (
            OptifineMode::Manual,
            "原版需要 OptiFine 官方安装器 patch 游戏；可以下载后手动点 Install".to_string(),
        ),
    }
}

fn mc_numbers(raw: &str) -> Vec<u32> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_digit() {
            current.push(ch);
        } else if ch == '.' && !current.is_empty() {
            match current.parse() {
                Ok(n) => out.push(n),
                Err(_) => return out,
            }
            current.clear();
        } else {
            break;
        }
    }
    if let Ok(n) = current.parse() {
        out.push(n);
    }
    out
}

/// `None` = 认不出（快照 `24w14a` 那种），不是「小于 1.13」。
fn at_least_1_13(mc_version: &str) -> Option<bool> {
    // 只接受纯 `数字.数字` 的正式版本号：快照 `24w14a` 里也有数字，
    // 按前缀解析会得出「24.x，比 1.13 大」这种错误结论。
    let trimmed = mc_version.trim();
    if trimmed.is_empty() || !trimmed.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let nums = mc_numbers(trimmed);
    let major = *nums.first()?;
    let minor = nums.get(1).copied().unwrap_or(0);
    let patch = nums.get(2).copied().unwrap_or(0);
    Some((major, minor, patch) >= (1, 13, 0))
}

// ---------------------------------------------------------------- 下载页解析

/// 从 `https://optifine.net/downloads` 的 HTML 里抠出某个 MC 版本的所有构建。
///
/// 页面里每个构建会出现好几次（Download / Mirror / Changelog），按文件名去重。
/// 关键点：`OptiFine_1.21.1_…` 是 `OptiFine_1.21.10_…` 的前缀，必须整段匹配
/// `_HD_U_`，不能只做前缀比较。
pub fn parse_builds(html: &str, mc_version: &str) -> Vec<OptifineBuild> {
    let marker = "OptiFine_";
    let mut out: Vec<OptifineBuild> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut rest = html;

    while let Some(pos) = rest.find(marker) {
        rest = &rest[pos..];
        let Some(end) = rest.find(".jar") else { break };
        let token = &rest[marker.len()..end]; // `<mc>_HD_U_<edition>`
        let filename = format!("{marker}{token}.jar");
        rest = &rest[end + 4..];

        if !seen.insert(filename.clone()) {
            continue;
        }
        let Some((build_mc, edition)) = split_build(token) else {
            continue;
        };
        if build_mc != mc_version {
            continue;
        }
        out.push(OptifineBuild {
            mc_version: build_mc.to_string(),
            edition: edition.to_string(),
            filename: filename.clone(),
            adloadx_url: format!("https://optifine.net/adloadx?f={filename}"),
            download_page: DOWNLOADS_PAGE.to_string(),
            pre: edition.contains("_pre"),
        });
    }

    out.sort_by_key(|b| edition_key(&b.edition));
    out
}

/// `1.21.1_HD_U_J1` → (`1.21.1`, `J1`)；`1.21.10_HD_U_J7_pre10` → (`1.21.10`, `J7_pre10`)
fn split_build(token: &str) -> Option<(&str, &str)> {
    let idx = token.find("_HD_U_")?;
    let mc = &token[..idx];
    let edition = &token[idx + "_HD_U_".len()..];
    if mc.is_empty() || edition.is_empty() {
        return None;
    }
    Some((mc, edition))
}

/// 版本排序键：字母、数字、是否正式版、pre 序号。
///
/// 正式版排在同一个字母+数字的 pre 之后（`J7` 比 `J7_pre10` 新），
/// 但不同字母/数字之间只看字母和数字（`J7_pre10` 比 `J1` 新）。
fn edition_key(edition: &str) -> (char, u32, u8, u32) {
    let (base, pre) = match edition.split_once("_pre") {
        Some((b, n)) => (b, n.parse::<u32>().unwrap_or(0)),
        None => (edition, 0),
    };
    let mut chars = base.chars();
    let letter = chars.next().unwrap_or('A');
    let number: u32 = chars.as_str().parse().unwrap_or(0);
    (letter, number, u8::from(pre == 0), pre)
}

/// 挑最新的构建：优先正式版，没有正式版就用最新的 pre。
pub fn pick_build(builds: &[OptifineBuild]) -> Option<&OptifineBuild> {
    builds.iter().max_by_key(|b| edition_key(&b.edition))
}

/// 从 `adloadx` 页面里抠出真正的 `downloadx?f=…&x=…` 地址。
///
/// 页面里这个链接出现两次（上下各一个），取第一个即可。`x` 是页面即时生成的
/// token，所以每次都要重新取页面，不能缓存最终 jar 地址。
pub fn parse_download_link(html: &str) -> Option<String> {
    let marker = "downloadx?f=";
    let start = html.find(marker)?;
    let rest = &html[start..];
    let end = rest
        .char_indices()
        .find(|(_, c)| matches!(c, '\'' | '"' | '<' | '>' | '\n' | '\r' | ' ' | '\t'))
        .map(|(i, _)| i)
        .unwrap_or(rest.len());
    let raw = rest[..end].replace("&amp;", "&");
    if !raw.contains("&x=") {
        return None;
    }
    Some(format!("https://optifine.net/{raw}"))
}

// ---------------------------------------------------------------- 网络

async fn with_timeout<T, F, Fut>(future: Fut, what: F) -> Result<T>
where
    F: FnOnce() -> String,
    Fut: std::future::Future<Output = Result<T>>,
{
    with_timeout_for(NET_TIMEOUT, future, what).await
}

async fn with_timeout_for<T, F, Fut>(limit: Duration, future: Fut, what: F) -> Result<T>
where
    F: FnOnce() -> String,
    Fut: std::future::Future<Output = Result<T>>,
{
    match tokio::time::timeout(limit, future).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(Error::other(format!("{}失败：{e}", what()))),
        Err(_) => Err(Error::other(format!(
            "{}超时（{} 秒）",
            what(),
            limit.as_secs().max(1)
        ))),
    }
}

/// 查 optifine.net 上这个 MC 版本有没有构建。
pub async fn resolve_build(client: &HttpClient, mc_version: &str) -> Result<Option<OptifineBuild>> {
    let html = with_timeout(client.get_text(DOWNLOADS_PAGE), || {
        "读取 optifine.net 下载页".to_string()
    })
    .await?;
    let builds = parse_builds(&html, mc_version);
    Ok(pick_build(&builds).cloned())
}

/// 给界面用的计划：能不能自动装、有没有官方构建可手动装。
pub async fn plan(
    client: &HttpClient,
    mc_version: &str,
    loader: Option<LoaderKind>,
) -> OptifinePlan {
    let (mode, reason) = support(loader, mc_version);
    let mut plan = OptifinePlan {
        supported: mode == OptifineMode::ModsFolder,
        mode,
        reason,
        download_page: DOWNLOADS_PAGE.to_string(),
        ..Default::default()
    };

    // 不适用（Fabric / NeoForge）就不必去打 optifine.net。
    if mode == OptifineMode::Unsupported {
        return plan;
    }

    match resolve_build(client, mc_version).await {
        Ok(Some(build)) => {
            plan.manual_possible = true;
            plan.build = Some(build);
        }
        Ok(None) => {
            plan.supported = false;
            plan.reason = format!(
                "optifine.net 上没有 Minecraft {mc_version} 的 OptiFine 构建（预览版可能还没出）"
            );
        }
        Err(e) => {
            // 查不到不等于不支持：保留支持结论，只把原因说清楚。
            plan.manual_possible = false;
            plan.reason = format!("{}；{}", plan.reason, e);
        }
    }
    plan
}

/// 下载到本地的官方安装器。
#[derive(Debug, Clone)]
pub struct DownloadedInstaller {
    pub path: PathBuf,
    pub bytes: u64,
    pub sha1: String,
    /// 复用了之前下好的文件
    pub reused: bool,
}

/// 下载官方安装器到 `<gameDir>/.mcdl/optifine/`。
///
/// 走两跳：先取 `adloadx` 页面拿即时 token，再下真正的 jar。下载结果只认
/// `PK` 文件头 + 最小体积——optifine.net 出问题时返回的是 HTML，不能当 jar 用。
pub async fn download_installer(
    client: &HttpClient,
    paths: &GamePaths,
    build: &OptifineBuild,
) -> Result<DownloadedInstaller> {
    let cache = paths.root().join(".mcdl").join("optifine");
    std::fs::create_dir_all(&cache).at(&cache)?;
    let dest = cache.join(&build.filename);

    if is_jar(&dest) {
        let bytes = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
        if bytes >= MIN_INSTALLER_BYTES as u64 {
            let sha1 = download::sha1_file(&dest)?;
            tracing::debug!(path = %dest.display(), "复用已下载的 OptiFine 安装器");
            return Ok(DownloadedInstaller {
                path: dest,
                bytes,
                sha1,
                reused: true,
            });
        }
    }

    let page = with_timeout(client.get_text(&build.adloadx_url), || {
        format!("读取 {} 的下载跳转页", build.filename)
    })
    .await?;
    let link = parse_download_link(&page).ok_or_else(|| {
        Error::other(format!(
            "optifine.net 的跳转页里没有找到 downloadx 链接（页面结构可能变了）。\
             可以手动打开 {} 下载",
            DOWNLOADS_PAGE
        ))
    })?;

    let bytes = with_timeout_for(DOWNLOAD_TIMEOUT, client.get_bytes(&link), || {
        format!("下载 {}", build.filename)
    })
    .await?;

    if bytes.len() < MIN_INSTALLER_BYTES {
        return Err(Error::other(format!(
            "{} 只有 {} 字节，不像安装器（可能下到了错误页）",
            build.filename,
            bytes.len()
        )));
    }
    if &bytes[..2] != b"PK" {
        return Err(Error::other(format!(
            "{} 不是 jar（文件头不是 PK），optifine.net 可能返回了错误页",
            build.filename
        )));
    }

    let part = dest.with_extension("jar.part");
    std::fs::write(&part, &bytes).at(&part)?;
    if dest.exists() {
        std::fs::remove_file(&dest).at(&dest)?;
    }
    std::fs::rename(&part, &dest).at(&dest)?;

    let sha1 = download::sha1_file(&dest)?;
    tracing::info!(
        path = %dest.display(),
        bytes = bytes.len(),
        "已从 optifine.net 下载官方安装器"
    );
    Ok(DownloadedInstaller {
        path: dest,
        bytes: bytes.len() as u64,
        sha1,
        reused: false,
    })
}

/// 这个 jar 是不是 Forge mod（`META-INF/mods.toml` 存在）。
///
/// 这是「放进 mods/ 真能加载」的判据：官方 installer 是自解压式的，
/// 1.13+ 的构建里确实带 Forge 的 mod 元数据，但不是所有历史构建都带。
pub fn is_forge_mod(jar: &Path) -> bool {
    let Ok(file) = std::fs::File::open(jar) else {
        return false;
    };
    let Ok(archive) = zip::ZipArchive::new(file) else {
        return false;
    };
    // `index_for_name` 只返回下标（zip 2.x 里是 Option），不借用 archive 里的条目，
    // 能避开临时值的生命周期问题。
    archive.index_for_name("META-INF/mods.toml").is_some()
}

fn is_jar(path: &Path) -> bool {
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 4];
    if f.read_exact(&mut magic).is_err() {
        return false;
    }
    &magic[..2] == b"PK"
}

// ---------------------------------------------------------------- 自动安装

/// 尝试自动装 OptiFine。永远返回一条结论，不抛错。
///
/// 只有 `Forge + 1.13+` 会真的下载并复制进模组目录；其余组合返回
/// `Manual` / `Skipped` 并写明原因（官方安装器由单独的「打开」命令处理）。
///
/// 模组落点由版本隔离决定（[`crate::isolate::mods_dir`]）。OptiFine 是最典型的
/// 「只属于这一个版本」的模组，隔离开着时不该出现在共享的根 `mods/` 里。
pub async fn install(
    client: &HttpClient,
    paths: &GamePaths,
    loader: Option<LoaderKind>,
    mc_version: &str,
    version_id: &str,
    isolated: bool,
) -> OptifineOutcome {
    let (mode, reason) = support(loader, mc_version);
    if mode != OptifineMode::ModsFolder {
        let status = if mode == OptifineMode::Manual {
            ExtraStatus::Manual
        } else {
            ExtraStatus::Skipped
        };
        let mut outcome = OptifineOutcome::new(status, reason, mode);
        if mode == OptifineMode::Manual {
            outcome.manual_hint = Some(
                "点「下载并打开官方安装器」，在 OptiFine 窗口里手动点 Install。\
                 文件来自 optifine.net 官方，本工具不托管。"
                    .to_string(),
            );
        }
        return outcome;
    }

    let build = match resolve_build(client, mc_version).await {
        Ok(Some(build)) => build,
        Ok(None) => {
            return OptifineOutcome::new(
                ExtraStatus::Skipped,
                format!("optifine.net 上没有 Minecraft {mc_version} 的 OptiFine 构建，已跳过"),
                mode,
            )
        }
        Err(e) => {
            return OptifineOutcome::new(
                ExtraStatus::Failed,
                format!("查询 optifine.net 失败：{e}"),
                mode,
            )
        }
    };

    let downloaded = match download_installer(client, paths, &build).await {
        Ok(dl) => dl,
        Err(e) => {
            return OptifineOutcome::new(
                ExtraStatus::Failed,
                format!("下载 OptiFine 官方安装器失败：{e}"),
                mode,
            )
            .with_build(build)
        }
    };

    if !is_forge_mod(&downloaded.path) {
        let mut outcome = OptifineOutcome::new(
            ExtraStatus::Manual,
            format!(
                "{} 里没有 Forge 的 mod 元数据（META-INF/mods.toml），不能直接放进 mods/；\
                 已下载到本地，请用官方安装器手动安装",
                build.filename
            ),
            OptifineMode::Manual,
        )
        .with_build(build);
        outcome.installer_path = Some(downloaded.path);
        outcome.bytes = downloaded.bytes;
        outcome.sha1 = Some(downloaded.sha1);
        outcome.manual_hint = Some("运行这个 jar 会打开官方安装器，点 Install 即可。".to_string());
        return outcome;
    }

    let mods = match isolate::mods_dir(paths.root(), version_id, isolated) {
        Ok(target) => target,
        Err(e) => {
            let mut outcome = OptifineOutcome::new(
                ExtraStatus::Failed,
                format!("算不出模组该放哪（版本隔离）：{e}"),
                mode,
            )
            .with_build(build);
            outcome.installer_path = Some(downloaded.path);
            return outcome;
        }
    };

    if let Err(e) = std::fs::create_dir_all(&mods.dir) {
        let mut outcome = OptifineOutcome::new(
            ExtraStatus::Failed,
            format!("建不了模组目录（{}）：{e}", mods.dir.display()),
            mode,
        )
        .with_build(build);
        outcome.installer_path = Some(downloaded.path);
        return outcome;
    }

    let dest = mods.join(&build.filename);
    if dest.is_file() {
        let mut outcome = OptifineOutcome::new(
            ExtraStatus::Skipped,
            format!("{} 里已经有 {}，跳过", mods.label(), build.filename),
            mode,
        )
        .with_build(build);
        outcome.installer_path = Some(downloaded.path);
        outcome.mod_path = Some(dest);
        outcome.bytes = downloaded.bytes;
        outcome.sha1 = Some(downloaded.sha1);
        return outcome;
    }

    if let Err(e) = std::fs::copy(&downloaded.path, &dest) {
        let mut outcome = OptifineOutcome::new(
            ExtraStatus::Failed,
            format!("把 {} 复制到 {} 失败：{e}", build.filename, mods.label()),
            mode,
        )
        .with_build(build);
        outcome.installer_path = Some(downloaded.path);
        return outcome;
    }

    tracing::info!(
        path = %dest.display(),
        edition = %build.edition,
        isolated = mods.isolated,
        "OptiFine 已落盘"
    );
    let mut outcome = OptifineOutcome::new(
        ExtraStatus::Done,
        format!(
            "已放入 {}{}（{:.1} MB）；它是 Forge mod，启动时选这次装的 Forge 版本即可",
            mods.label(),
            build.filename,
            downloaded.bytes as f64 / 1_048_576.0
        ),
        mode,
    )
    .with_build(build);
    outcome.installer_path = Some(downloaded.path);
    outcome.mod_path = Some(dest);
    outcome.bytes = downloaded.bytes;
    outcome.sha1 = Some(downloaded.sha1);
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 下载页里同一行会重复出现构建名，而且 1.21.1 是 1.21.10 的前缀。
    const PAGE: &str = r#"
        <td class='colFile'><a href="adloadx?f=OptiFine_1.21.1_HD_U_J1.jar">OptiFine_1.21.1_HD_U_J1.jar</a></td>
        <td class='colDownload'><a href="http://adfoc.us/serve/?url=http://optifine.net/adloadx?f=OptiFine_1.21.1_HD_U_J1.jar&x=ca62">Download</a></td>
        <td class='colMirror'><a href="http://optifine.net/adloadx?f=OptiFine_1.21.1_HD_U_J1.jar">(Mirror)</a></td>
        <td class='colChangelog'><a href='changelog?f=OptiFine_1.21.1_HD_U_J1.jar'>Changelog</a></td>
        <a href="adloadx?f=OptiFine_1.21.10_HD_U_J7_pre10.jar">OptiFine_1.21.10_HD_U_J7_pre10.jar</a>
        <a href="adloadx?f=OptiFine_1.21_HD_U_J1_pre9.jar">OptiFine_1.21_HD_U_J1_pre9.jar</a>
        <a href="adloadx?f=OptiFine_1.12.2_HD_U_G5.jar">OptiFine_1.12.2_HD_U_G5.jar</a>
    "#;

    #[test]
    fn parses_exact_mc_version_only() {
        let builds = parse_builds(PAGE, "1.21.1");
        assert_eq!(builds.len(), 1, "1.21.1 不该把 1.21.10 / 1.21 混进来");
        let b = &builds[0];
        assert_eq!(b.filename, "OptiFine_1.21.1_HD_U_J1.jar");
        assert_eq!(b.edition, "J1");
        assert_eq!(b.mc_version, "1.21.1");
        assert!(!b.pre);
        assert_eq!(
            b.adloadx_url,
            "https://optifine.net/adloadx?f=OptiFine_1.21.1_HD_U_J1.jar"
        );

        let builds = parse_builds(PAGE, "1.21.10");
        assert_eq!(builds.len(), 1);
        assert!(builds[0].pre, "J7_pre10 应该标成预览版");
        assert_eq!(builds[0].edition, "J7_pre10");
    }

    #[test]
    fn pick_prefers_stable_then_newest_pre() {
        let builds = parse_builds(PAGE, "1.21.1");
        assert_eq!(pick_build(&builds).unwrap().edition, "J1");

        let builds = parse_builds(
            r#"<a href="x">OptiFine_1.21.10_HD_U_J7_pre9.jar</a>
               <a href="x">OptiFine_1.21.10_HD_U_J7_pre10.jar</a>
               <a href="x">OptiFine_1.21.10_HD_U_J7_pre11.jar</a>"#,
            "1.21.10",
        );
        assert_eq!(pick_build(&builds).unwrap().edition, "J7_pre11");

        // 同字母数字：正式版压过 pre
        let builds = parse_builds(
            r#"<a href="x">OptiFine_1.21.4_HD_U_J4_pre2.jar</a>
               <a href="x">OptiFine_1.21.4_HD_U_J4.jar</a>"#,
            "1.21.4",
        );
        assert_eq!(pick_build(&builds).unwrap().edition, "J4");
    }

    #[test]
    fn no_build_for_unknown_version() {
        assert!(parse_builds(PAGE, "1.20.1").is_empty());
        assert!(pick_build(&[]).is_none());
    }

    #[test]
    fn parses_download_link_and_unescapes_amp() {
        let html = r#"<a href='downloadx?f=OptiFine_1.21.1_HD_U_J1.jar&amp;x=ca621795edf23cd273d69ccb12e8cdfa'>Download</a>"#;
        let link = parse_download_link(html).unwrap();
        assert_eq!(
            link,
            "https://optifine.net/downloadx?f=OptiFine_1.21.1_HD_U_J1.jar&x=ca621795edf23cd273d69ccb12e8cdfa"
        );

        // 没有 token 的链接不能当成可下载地址
        assert!(parse_download_link("downloadx?f=OptiFine.jar").is_none());
        assert!(parse_download_link("没有链接").is_none());
    }

    #[test]
    fn support_matrix_is_honest() {
        // Forge + 1.13 以后：真能自动装
        assert_eq!(
            support(Some(LoaderKind::Forge), "1.21.1").0,
            OptifineMode::ModsFolder
        );
        assert_eq!(
            support(Some(LoaderKind::Forge), "26.3").0,
            OptifineMode::ModsFolder
        );
        // Forge + 1.12 及更早：需要 patch，标注手动
        assert_eq!(
            support(Some(LoaderKind::Forge), "1.12.2").0,
            OptifineMode::Manual
        );
        // 原版：需要官方安装器
        assert_eq!(support(None, "1.21.1").0, OptifineMode::Manual);
        // Fabric / Quilt / NeoForge：不适用
        for kind in [LoaderKind::Fabric, LoaderKind::Quilt, LoaderKind::NeoForge] {
            assert_eq!(
                support(Some(kind), "1.21.1").0,
                OptifineMode::Unsupported,
                "{kind:?} 不该被说成支持"
            );
        }
        // 快照：认不出，不支持
        assert_eq!(
            support(Some(LoaderKind::Forge), "24w14a").0,
            OptifineMode::Unsupported
        );
    }

    #[test]
    fn at_least_1_13_handles_major_bumps() {
        assert_eq!(at_least_1_13("1.12.2"), Some(false));
        assert_eq!(at_least_1_13("1.13"), Some(true));
        assert_eq!(at_least_1_13("1.21.1"), Some(true));
        assert_eq!(at_least_1_13("26.3"), Some(true));
        assert_eq!(at_least_1_13("24w14a"), None);
    }

    #[test]
    fn detects_forge_mod_metadata() {
        use std::io::Write as _;
        let dir = std::env::temp_dir().join(format!(
            "mcdl-optifine-mod-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let with_toml = dir.join("forge.jar");
        {
            let file = std::fs::File::create(&with_toml).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file("META-INF/mods.toml", options).unwrap();
            zip.write_all(b"modLoader=\"javafml\"\nmodId=\"optifine\"")
                .unwrap();
            zip.finish().unwrap();
        }
        assert!(is_forge_mod(&with_toml));

        let plain = dir.join("plain.jar");
        {
            let file = std::fs::File::create(&plain).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file("optifine/Installer.class", options).unwrap();
            zip.write_all(b"not a mod").unwrap();
            zip.finish().unwrap();
        }
        assert!(!is_forge_mod(&plain));
        assert!(!is_forge_mod(&dir.join("missing.jar")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------------ 真实网络
    //
    // 手动跑：`cargo test -p mcdl-core extras::optifine -- --ignored --nocapture`

    /// 真去 optifine.net 解析 1.21.1 的构建。这条会暴露页面结构变化。
    #[tokio::test]
    #[ignore = "需要真实网络（optifine.net）"]
    async fn resolve_optifine_over_network() {
        let client = HttpClient::new().unwrap();
        let build = resolve_build(&client, "1.21.1")
            .await
            .unwrap()
            .expect("1.21.1 应该有 OptiFine 构建");
        println!(
            "OptiFine 1.21.1 → {}（edition {}，pre = {}）",
            build.filename, build.edition, build.pre
        );
        assert!(build.filename.starts_with("OptiFine_1.21.1_HD_U_"));
        assert!(build.filename.ends_with(".jar"));
    }

    /// 走完两跳跳转页真下载一次，并验证它确实是 Forge mod。
    #[tokio::test]
    #[ignore = "需要真实网络，会下载约 7 MB"]
    async fn download_optifine_installer_over_network() {
        let client = HttpClient::new().unwrap();
        let build = resolve_build(&client, "1.21.1")
            .await
            .unwrap()
            .expect("1.21.1 应该有 OptiFine 构建");
        let dir = std::env::temp_dir().join(format!(
            "mcdl-optifine-dl-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let paths = GamePaths::new(&dir);
        let downloaded = download_installer(&client, &paths, &build).await.unwrap();
        println!(
            "{}：{} 字节，sha1 {}，复用 {}",
            downloaded.path.display(),
            downloaded.bytes,
            downloaded.sha1,
            downloaded.reused
        );
        assert!(downloaded.bytes > MIN_INSTALLER_BYTES as u64);
        assert!(
            is_forge_mod(&downloaded.path),
            "1.21.1 的 installer 应该带 mods.toml"
        );

        // 再跑一次必须复用，不重新下载
        let again = download_installer(&client, &paths, &build).await.unwrap();
        assert!(again.reused);
        assert_eq!(again.sha1, downloaded.sha1);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
