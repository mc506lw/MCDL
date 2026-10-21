//! Fabric API / Quilted Fabric API 的自动补齐。
//!
//! 没有 Fabric API，绝大多数 Fabric 模组会直接崩在启动阶段，所以它默认勾选。
//! 数据只来自 Modrinth 的公开接口（不需要 API key）：
//!
//! ```text
//! GET https://api.modrinth.com/v2/project/fabric-api/version
//!     ?game_versions=["1.21.1"]&loaders=["fabric"]
//! ```
//!
//! 服务端已经按这两个条件过滤过，但我们**不信任**它：版本 JSON 里再核对一遍
//! `game_versions` / `loaders`，并优先取 `release`。这样接口行为变了也不会
//! 把 beta 版、或者给别的加载器准备的构建塞进 mods/。

use serde::{Deserialize, Serialize};

use crate::download;
use crate::error::Result;
use crate::isolate;
use crate::net::HttpClient;
use crate::paths::GamePaths;
use crate::types::{DownloadJob, LoaderKind};

use super::ExtraStatus;

const API: &str = "https://api.modrinth.com/v2";

/// Fabric 的官方 API 工程。
pub const FABRIC_API_PROJECT: &str = "fabric-api";
/// Quilt 这边要的是 Quilted Fabric API（Modrinth 上 `qsl` 这个 slug 同时覆盖 QFAPI 与 QSL）。
///
/// 注意：`quilted-fabric-api` 这个 slug 在 Modrinth 上是 404，别照着名字猜。
pub const QUILT_API_PROJECT: &str = "qsl";

/// 某个加载器对应哪个 Modrinth 工程；Forge 系不需要。
pub fn project_for(kind: LoaderKind) -> Option<&'static str> {
    match kind {
        LoaderKind::Fabric => Some(FABRIC_API_PROJECT),
        LoaderKind::Quilt => Some(QUILT_API_PROJECT),
        LoaderKind::Forge | LoaderKind::NeoForge => None,
    }
}

/// Modrinth 的加载器名。
fn loader_name(kind: LoaderKind) -> &'static str {
    match kind {
        LoaderKind::Fabric => "fabric",
        LoaderKind::Quilt => "quilt",
        LoaderKind::Forge => "forge",
        LoaderKind::NeoForge => "neoforge",
    }
}

/// 版本列表接口的完整 URL。单测直接断言它（编码错一个字符就是 404）。
pub fn version_url(project: &str, mc_version: &str, loader: &str) -> String {
    let mut url = reqwest::Url::parse(&format!("{API}/project/{project}/version"))
        .expect("Modrinth 基址是常量");
    {
        let mut pairs = url.query_pairs_mut();
        // 接口要的是 JSON 数组字符串，方括号和引号必须原样编码进去。
        pairs.append_pair("game_versions", &format!(r#"["{mc_version}"]"#));
        pairs.append_pair("loaders", &format!(r#"["{loader}"]"#));
    }
    url.to_string()
}

/// 一个可下载的 Fabric API 文件。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FabricApiFile {
    /// Modrinth 工程 slug
    pub project: String,
    /// 版本号，例如 `0.116.17+1.21.1`
    pub version: String,
    /// 落盘用的文件名，直接取接口给的 `filename`
    pub filename: String,
    pub url: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    #[serde(default)]
    pub game_versions: Vec<String>,
    #[serde(default)]
    pub loaders: Vec<String>,
}

/// 补齐 Fabric API 的结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FabricApiOutcome {
    pub status: ExtraStatus,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<FabricApiFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<std::path::PathBuf>,
    /// 这次真正落盘的字节数（跳过已存在文件时是 0）
    pub bytes: u64,
    /// 文件本来就在，什么都没下
    pub already_present: bool,
}

impl FabricApiOutcome {
    fn new(status: ExtraStatus, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            file: None,
            path: None,
            bytes: 0,
            already_present: false,
        }
    }

    fn with_file(mut self, file: FabricApiFile) -> Self {
        self.file = Some(file);
        self
    }
}

// ---------------------------------------------------------------- 响应结构

#[derive(Debug, Default, Deserialize)]
struct VersionEntry {
    #[serde(default)]
    version_number: String,
    #[serde(default)]
    version_type: String,
    #[serde(default)]
    date_published: String,
    #[serde(default)]
    game_versions: Vec<String>,
    #[serde(default)]
    loaders: Vec<String>,
    #[serde(default)]
    files: Vec<FileEntry>,
}

#[derive(Debug, Default, Deserialize)]
struct FileEntry {
    #[serde(default)]
    filename: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    primary: bool,
    #[serde(default)]
    hashes: FileHashes,
}

#[derive(Debug, Default, Deserialize)]
struct FileHashes {
    #[serde(default)]
    sha1: Option<String>,
}

/// 从版本列表里挑一个：先核对它真的支持这个 MC 版本和这个加载器，再优先 release，
/// 同一档里取发布时间最新的。
fn pick_version<'a>(
    entries: &'a [VersionEntry],
    mc_version: &str,
    loader: &str,
) -> Option<&'a VersionEntry> {
    let mut matched: Vec<&VersionEntry> = entries
        .iter()
        .filter(|e| e.game_versions.iter().any(|v| v == mc_version))
        .filter(|e| e.loaders.iter().any(|l| l == loader))
        .collect();

    // 接口按发布时间从新到旧返回，但不能只依赖这个顺序。
    matched.sort_by(|a, b| b.date_published.cmp(&a.date_published));

    matched
        .iter()
        .find(|e| e.version_type == "release")
        .or_else(|| matched.first())
        .copied()
}

fn primary_file(entry: &VersionEntry) -> Option<&FileEntry> {
    entry
        .files
        .iter()
        .find(|f| f.primary)
        .or_else(|| entry.files.first())
}

/// 查最新版本；这个 MC 版本没有对应构建时返回 `Ok(None)`（不是错误）。
pub async fn resolve(
    client: &HttpClient,
    kind: LoaderKind,
    mc_version: &str,
) -> Result<Option<FabricApiFile>> {
    let Some(project) = project_for(kind) else {
        return Ok(None);
    };
    let url = version_url(project, mc_version, loader_name(kind));
    let entries: Vec<VersionEntry> = client.get_json(&url).await?;

    let Some(entry) = pick_version(&entries, mc_version, loader_name(kind)) else {
        tracing::info!(
            project,
            mc = mc_version,
            "Modrinth 上没有这个版本的构建，跳过"
        );
        return Ok(None);
    };
    let Some(file) = primary_file(entry) else {
        tracing::warn!(project, version = %entry.version_number, "版本里没有文件条目");
        return Ok(None);
    };

    Ok(Some(FabricApiFile {
        project: project.to_string(),
        version: entry.version_number.clone(),
        filename: file.filename.clone(),
        url: file.url.clone(),
        size: file.size,
        sha1: file.hashes.sha1.clone(),
        game_versions: entry.game_versions.clone(),
        loaders: entry.loaders.clone(),
    }))
}

/// 把 Fabric API 下载到模组目录。永远返回一条结论，不抛错：
/// 附加项失败不该让一次已经成功的安装显示成失败。
///
/// 落点由版本隔离决定（[`crate::isolate::mods_dir`]）：
/// 开着隔离是 `<gameDir>/versions/<version_id>/mods/`，关掉是 `<gameDir>/mods/`。
/// `version_id` 必须是改名之后的最终版本目录名。
pub async fn install(
    client: &HttpClient,
    paths: &GamePaths,
    loader: Option<LoaderKind>,
    mc_version: &str,
    version_id: &str,
    isolated: bool,
) -> FabricApiOutcome {
    let Some(kind) = loader else {
        return FabricApiOutcome::new(ExtraStatus::Skipped, "原版不需要 Fabric API");
    };
    let Some(project) = project_for(kind) else {
        return FabricApiOutcome::new(
            ExtraStatus::Skipped,
            format!("{} 不需要 Fabric API", kind.label()),
        );
    };

    let file = match resolve(client, kind, mc_version).await {
        Ok(Some(file)) => file,
        Ok(None) => {
            return FabricApiOutcome::new(
                ExtraStatus::Skipped,
                format!("Modrinth 上没有 {project} 支持 Minecraft {mc_version} 的版本，已跳过（不影响启动）"),
            )
        }
        Err(e) => {
            return FabricApiOutcome::new(
                ExtraStatus::Failed,
                format!("查询 {project} 失败：{e}"),
            )
        }
    };

    let mods = match isolate::mods_dir(paths.root(), version_id, isolated) {
        Ok(target) => target,
        Err(e) => {
            return FabricApiOutcome::new(
                ExtraStatus::Failed,
                format!("算不出模组该放哪（版本隔离）：{e}"),
            )
            .with_file(file)
        }
    };

    if let Err(e) = std::fs::create_dir_all(&mods.dir) {
        return FabricApiOutcome::new(
            ExtraStatus::Failed,
            format!("建不了模组目录（{}）：{e}", mods.dir.display()),
        )
        .with_file(file);
    }

    let dest = mods.join(&file.filename);
    if dest.is_file() {
        let size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
        tracing::info!(path = %dest.display(), "模组目录里已经有同名文件，跳过");
        let mut outcome = FabricApiOutcome::new(
            ExtraStatus::Skipped,
            format!(
                "{} 里已经有 {}（{:.1} MB），跳过",
                mods.label(),
                file.filename,
                size as f64 / 1_048_576.0
            ),
        )
        .with_file(file);
        outcome.path = Some(dest);
        outcome.already_present = true;
        return outcome;
    }

    let job = DownloadJob {
        url: file.url.clone(),
        dest: dest.clone(),
        sha1: file.sha1.clone(),
        size: Some(file.size),
        label: file.filename.clone(),
    };
    if let Err(e) = download::download_one(client, &job).await {
        return FabricApiOutcome::new(
            ExtraStatus::Failed,
            format!("下载 {} 失败：{e}", file.filename),
        )
        .with_file(file);
    }

    let bytes = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
    tracing::info!(
        path = %dest.display(),
        bytes,
        version = %file.version,
        isolated = mods.isolated,
        "Fabric API 已落盘"
    );
    let mut outcome = FabricApiOutcome::new(
        ExtraStatus::Done,
        format!(
            "已放入 {}{}（{:.1} MB）",
            mods.label(),
            file.filename,
            bytes as f64 / 1_048_576.0
        ),
    )
    .with_file(file);
    outcome.path = Some(dest);
    outcome.bytes = bytes;
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(
        version: &str,
        kind: &str,
        published: &str,
        game: &[&str],
        loaders: &[&str],
        filename: &str,
    ) -> VersionEntry {
        VersionEntry {
            version_number: version.into(),
            version_type: kind.into(),
            date_published: published.into(),
            game_versions: game.iter().map(|s| s.to_string()).collect(),
            loaders: loaders.iter().map(|s| s.to_string()).collect(),
            files: vec![FileEntry {
                filename: filename.into(),
                url: format!("https://cdn.modrinth.com/data/AABBCC/{filename}"),
                size: 2_400_000,
                primary: true,
                hashes: FileHashes {
                    sha1: Some("aa".into()),
                },
            }],
        }
    }

    #[test]
    fn url_encodes_json_arrays() {
        let url = version_url("fabric-api", "1.21.1", "fabric");
        assert!(
            url.starts_with("https://api.modrinth.com/v2/project/fabric-api/version?"),
            "{url}"
        );
        // 方括号与引号必须被编码，否则 Modrinth 会当成非法查询
        assert!(url.contains("game_versions=%5B%221.21.1%22%5D"), "{url}");
        assert!(url.contains("loaders=%5B%22fabric%22%5D"), "{url}");
    }

    #[test]
    fn picks_release_over_newer_beta() {
        let entries = vec![
            entry(
                "0.117.0-beta.1",
                "beta",
                "2026-10-01",
                &["1.21.1"],
                &["fabric"],
                "beta.jar",
            ),
            entry(
                "0.116.17+1.21.1",
                "release",
                "2026-09-01",
                &["1.21.1"],
                &["fabric"],
                "release.jar",
            ),
        ];
        let picked = pick_version(&entries, "1.21.1", "fabric").expect("应该挑到一个");
        assert_eq!(picked.version_number, "0.116.17+1.21.1");
        assert_eq!(primary_file(picked).unwrap().filename, "release.jar");
    }

    #[test]
    fn picks_newest_when_all_are_prerelease() {
        let entries = vec![
            entry(
                "0.116.16+1.21.1",
                "beta",
                "2026-08-01",
                &["1.21.1"],
                &["fabric"],
                "old.jar",
            ),
            entry(
                "0.116.17+1.21.1",
                "beta",
                "2026-09-01",
                &["1.21.1"],
                &["fabric"],
                "new.jar",
            ),
        ];
        let picked = pick_version(&entries, "1.21.1", "fabric").unwrap();
        assert_eq!(primary_file(picked).unwrap().filename, "new.jar");
    }

    /// 服务端过滤不可信：给别的 MC 版本、别的加载器的条目必须被剔掉。
    #[test]
    fn ignores_entries_for_other_versions_or_loaders() {
        let entries = vec![
            entry(
                "0.1",
                "release",
                "2026-09-01",
                &["1.21"],
                &["fabric"],
                "wrong-mc.jar",
            ),
            entry(
                "0.2",
                "release",
                "2026-09-01",
                &["1.21.1"],
                &["quilt"],
                "wrong-loader.jar",
            ),
            entry(
                "0.3",
                "release",
                "2026-09-01",
                &["1.21.1"],
                &["fabric"],
                "right.jar",
            ),
        ];
        let picked = pick_version(&entries, "1.21.1", "fabric").unwrap();
        assert_eq!(primary_file(picked).unwrap().filename, "right.jar");
    }

    #[test]
    fn no_match_returns_none() {
        let entries = vec![entry(
            "0.1",
            "release",
            "2026-09-01",
            &["1.20.1"],
            &["fabric"],
            "x.jar",
        )];
        assert!(pick_version(&entries, "1.21.1", "fabric").is_none());
        assert!(pick_version(&[], "1.21.1", "fabric").is_none());
    }

    #[test]
    fn project_mapping_only_covers_fabric_family() {
        assert_eq!(project_for(LoaderKind::Fabric), Some("fabric-api"));
        assert_eq!(project_for(LoaderKind::Quilt), Some("qsl"));
        assert_eq!(project_for(LoaderKind::Forge), None);
        assert_eq!(project_for(LoaderKind::NeoForge), None);
    }

    #[test]
    fn parses_modrinth_payload() {
        // 只保留接口里我们真正用到的字段；多出来的字段要被忽略。
        let raw = r#"[
            {
                "id": "Wo44jriL",
                "project_id": "P7dR8mSH",
                "version_number": "0.116.17+1.21.1",
                "version_type": "release",
                "date_published": "2026-09-01T16:40:04.373428Z",
                "game_versions": ["1.21.1"],
                "loaders": ["fabric"],
                "files": [
                    {
                        "filename": "fabric-api-0.116.17+1.21.1.jar",
                        "url": "https://cdn.modrinth.com/data/P7dR8mSH/versions/abc/fabric-api-0.116.17%2B1.21.1.jar",
                        "size": 2458192,
                        "primary": true,
                        "hashes": { "sha1": "deadbeef", "sha512": "cafe" }
                    },
                    { "filename": "sources.jar", "url": "https://x", "size": 1, "primary": false }
                ]
            }
        ]"#;
        let entries: Vec<VersionEntry> = serde_json::from_str(raw).unwrap();
        let picked = pick_version(&entries, "1.21.1", "fabric").unwrap();
        let file = primary_file(picked).unwrap();
        assert_eq!(file.filename, "fabric-api-0.116.17+1.21.1.jar");
        assert_eq!(file.size, 2_458_192);
        assert_eq!(file.hashes.sha1.as_deref(), Some("deadbeef"));
    }

    // ------------------------------------------------------------ 真实网络
    //
    // 手动跑：`cargo test -p mcdl-core extras::fabric_api -- --ignored --nocapture`

    #[tokio::test]
    #[ignore = "需要真实网络（Modrinth API）"]
    async fn resolve_fabric_api_over_network() {
        let client = HttpClient::new().unwrap();
        let file = resolve(&client, LoaderKind::Fabric, "1.21.1")
            .await
            .unwrap()
            .expect("1.21.1 应该有 Fabric API");
        println!(
            "Fabric API {}：{}（{} 字节，sha1 {:?}）",
            file.version, file.filename, file.size, file.sha1
        );
        assert!(file.filename.ends_with(".jar"));
        assert!(file.size > 100_000);
        assert!(file.sha1.is_some(), "Modrinth 必须给 sha1，下载要校验");
        assert!(file.game_versions.iter().any(|v| v == "1.21.1"));
    }

    /// Quilt 这边 Modrinth 上的 `qsl`（QFAPI / QSL）更新很慢：实测 1.21.1 没有构建，
    /// 1.20.1 有。查不到时必须安静地返回 `None`（跳过），不能报错。
    #[tokio::test]
    #[ignore = "需要真实网络（Modrinth API）"]
    async fn resolve_quilt_api_over_network() {
        let client = HttpClient::new().unwrap();
        let missing = resolve(&client, LoaderKind::Quilt, "1.21.1").await.unwrap();
        println!("QFAPI 1.21.1 → {missing:?}（查不到是正常的，跳过即可）");

        let file = resolve(&client, LoaderKind::Quilt, "1.20.1")
            .await
            .unwrap()
            .expect("1.20.1 应该有 QFAPI");
        println!("QFAPI 1.20.1：{}（{} 字节）", file.filename, file.size);
        assert!(file.loaders.iter().any(|l| l == "quilt"));
    }
}
