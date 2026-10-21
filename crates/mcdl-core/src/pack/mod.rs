//! 整合包：浏览（Modrinth / CurseForge）与安装。
//!
//! 这个模块**不重复实现下载**：整合包自己的文件走 [`crate::download`]，
//! 游戏本体和加载器走 [`crate::install::Installer`]，这一层只负责编排与格式解析。
//!
//! 平台协议全部来自公开文档，不参考任何启动器的实现：
//!
//! - Modrinth API：`https://docs.modrinth.com/api/`
//! - `.mrpack` 格式：`https://docs.modrinth.com/modpacks/format`
//! - CurseForge API：`https://docs.curseforge.com/`
//!
//! ## 进度为什么要单独一套
//!
//! 引擎的 [`Progress`] 只描述「装原版 + 装加载器」那一次安装；整合包在这之后还有
//! 两个阶段（下载模组文件、写覆盖文件），它们不属于那 6 个 [`StageKind`] 里的任何一个。
//! 所以这里定义 [`PackProgress`]：整合包自己的阶段 + 原样转发的引擎事件。
//! 界面据此可以同时画出「整合包进度」和「游戏本体进度」两条线。

pub mod curseforge;
pub mod install;
pub mod modrinth;
pub mod mrpack;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::types::{LoaderKind, LogLevel, Progress};

pub use install::{install_pack, PackInstallReport, PackInstallRequest};
pub use mrpack::{ModpackEnv, ModpackFile, ModpackHashes, ModpackIndex};

// ---------------------------------------------------------------- 平台

/// 整合包来自哪个平台。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PackSource {
    /// Modrinth：公开 API，不需要 key。
    #[default]
    Modrinth,
    /// CurseForge：官方 API 需要 `x-api-key`。
    Curseforge,
}

impl PackSource {
    pub fn label(self) -> &'static str {
        match self {
            PackSource::Modrinth => "Modrinth",
            PackSource::Curseforge => "CurseForge",
        }
    }

    /// 需要用户自己申请 key 的平台。
    pub fn needs_key(self) -> bool {
        matches!(self, PackSource::Curseforge)
    }
}

/// CurseForge 的 key 申请地址。界面上直接给链接，不内置任何 key。
pub const CURSEFORGE_KEY_URL: &str = "https://console.curseforge.com/";

// ---------------------------------------------------------------- 搜索

/// 排序方式。两个平台的字段不一样，这里做统一映射。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PackSort {
    /// 相关度（Modrinth 默认）
    #[default]
    Relevance,
    /// 下载量
    Downloads,
    /// 最近更新
    Updated,
    /// 最新发布
    Newest,
}

/// 一次搜索的参数。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackSearchQuery {
    pub query: String,
    #[serde(default)]
    pub source: PackSource,
    /// 每页条数，默认 20，上限 50
    #[serde(default)]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
    /// 只要支持这个 MC 版本的整合包
    #[serde(default)]
    pub mc_version: Option<String>,
    /// 只要这个加载器的整合包
    #[serde(default)]
    pub loader: Option<LoaderKind>,
    #[serde(default)]
    pub sort: PackSort,
    /// 仅 CurseForge 需要
    #[serde(default)]
    pub api_key: Option<String>,
}

impl PackSearchQuery {
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            limit: 20,
            ..Default::default()
        }
    }

    /// 实际生效的条数：0 会退化成空列表，所以给默认值并夹住上限。
    pub fn page_size(&self) -> usize {
        match self.limit {
            0 => 20,
            n => n.min(50),
        }
    }
}

/// 搜索结果里的一条。
///
/// 两个平台字段差异很大，这里只保留界面真正要显示的那些，
/// 并且**只放已经拿到的值**：拿不到就留 `None`，界面写「—」，不猜。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackHit {
    pub source: PackSource,
    /// Modrinth 是 project id，CurseForge 是数字 id 的字符串形式
    pub id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    /// 作者名；CurseForge 的项目详情里才有，搜索命中可能为空
    pub author: Option<String>,
    pub icon_url: Option<String>,
    pub downloads: u64,
    pub followers: u64,
    /// 分类里混着加载器名（fabric / forge …）和玩法分类
    pub categories: Vec<String>,
    /// 支持的 MC 版本
    pub game_versions: Vec<String>,
    /// 支持的加载器（Modrinth 有独立字段；CurseForge 从分类里认）
    pub loaders: Vec<String>,
    pub date_modified: String,
    pub latest_version: Option<String>,
    pub license: Option<String>,
}

impl PackHit {
    /// 从分类里认出加载器名，去重后返回。
    pub fn loaders_from_categories(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for c in &self.categories {
            let lower = c.to_ascii_lowercase();
            if matches!(lower.as_str(), "fabric" | "forge" | "neoforge" | "quilt")
                && !out.contains(&lower)
            {
                out.push(lower);
            }
        }
        out
    }
}

/// 一页搜索结果。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackSearchResult {
    pub hits: Vec<PackHit>,
    /// 平台报告的命中总数（用于「共 N 个」和翻页）
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
}

/// 整合包详情。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackProject {
    pub source: PackSource,
    pub id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    /// Markdown 正文；界面只做纯文本截断显示，不渲染 Markdown
    pub body: String,
    pub author: Option<String>,
    pub icon_url: Option<String>,
    pub downloads: u64,
    pub followers: u64,
    pub categories: Vec<String>,
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
    pub license: Option<String>,
    pub source_url: Option<String>,
    pub issues_url: Option<String>,
    pub wiki_url: Option<String>,
    pub published: String,
    pub updated: String,
}

/// 版本里的一个可下载文件。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackFile {
    pub filename: String,
    /// 空字符串表示平台没有给出下载地址（CurseForge 的 distribution 限制）
    pub url: String,
    pub size: u64,
    pub sha1: Option<String>,
    pub sha512: Option<String>,
    pub primary: bool,
}

impl PackFile {
    /// 能不能直接下载。CurseForge 有一部分作者关掉了第三方分发。
    pub fn downloadable(&self) -> bool {
        !self.url.trim().is_empty()
    }
}

/// 版本里声明的一条依赖（模组之间的依赖关系，安装整合包本身用不到，展示用）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackDep {
    pub project_id: Option<String>,
    pub version_id: Option<String>,
    pub file_name: Option<String>,
    /// required / optional / incompatible / embedded
    pub dependency_type: String,
}

/// 整合包的一个版本。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackVersion {
    pub id: String,
    pub name: String,
    pub version_number: String,
    /// release / beta / alpha
    pub version_type: String,
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
    pub date_published: String,
    pub downloads: u64,
    pub files: Vec<PackFile>,
    pub dependencies: Vec<PackDep>,
}

impl PackVersion {
    /// 安装要用的那个文件：优先 `primary` 的 `.mrpack` / `.zip`。
    ///
    /// Modrinth 的版本里常带一个 `cosign-bundle.zip` 签名包，不能当成整合包本体。
    pub fn archive_file(&self) -> Option<&PackFile> {
        let is_archive = |f: &PackFile| {
            let name = f.filename.to_ascii_lowercase();
            f.downloadable() && (name.ends_with(".mrpack") || name.ends_with(".zip"))
        };
        self.files
            .iter()
            .find(|f| f.primary && is_archive(f))
            .or_else(|| self.files.iter().find(|f| is_archive(f)))
    }
}

// ---------------------------------------------------------------- 统一入口

/// 搜索整合包。按 `q.source` 分派到对应平台。
pub async fn search(
    client: &crate::net::HttpClient,
    q: &PackSearchQuery,
) -> crate::Result<PackSearchResult> {
    match q.source {
        PackSource::Modrinth => modrinth::search(client, q).await,
        PackSource::Curseforge => curseforge::search(client, q).await,
    }
}

/// 整合包详情。
pub async fn project(
    client: &crate::net::HttpClient,
    source: PackSource,
    id: &str,
    api_key: Option<&str>,
) -> crate::Result<PackProject> {
    match source {
        PackSource::Modrinth => modrinth::project(client, id).await,
        PackSource::Curseforge => curseforge::project(client, api_key, id).await,
    }
}

/// 某个整合包的所有版本，从新到旧。
pub async fn versions(
    client: &crate::net::HttpClient,
    source: PackSource,
    id: &str,
    api_key: Option<&str>,
) -> crate::Result<Vec<PackVersion>> {
    match source {
        PackSource::Modrinth => modrinth::versions(client, id).await,
        PackSource::Curseforge => curseforge::versions(client, api_key, id).await,
    }
}

/// 挑一个可以安装的版本：给了 `version_id` 就精确匹配，否则取最新的。
pub async fn resolve_version(
    client: &crate::net::HttpClient,
    source: PackSource,
    id: &str,
    version_id: Option<&str>,
    api_key: Option<&str>,
) -> crate::Result<PackVersion> {
    let list = versions(client, source, id, api_key).await?;
    let picked =
        match version_id {
            Some(want) => list.iter().find(|v| v.id == want).cloned().ok_or_else(|| {
                crate::Error::other(format!("{} 里没有版本 {want}", source.label()))
            })?,
            None => list.first().cloned().ok_or_else(|| {
                crate::Error::other(format!("{} 上这个整合包没有发布过版本", source.label()))
            })?,
        };
    if picked.archive_file().is_none() {
        return Err(crate::Error::other(format!(
            "{} 版本「{}」没有可下载的整合包文件（可能只发布了服务端包）",
            source.label(),
            picked.version_number
        )));
    }
    Ok(picked)
}

// ---------------------------------------------------------------- 进度

/// 整合包安装的进度事件。
///
/// 分两类：
/// - 整合包自己的阶段（`Stage` / `Files` / `Log`），`index` 1 起、`total` 是整合包的阶段数；
/// - `Engine`：装游戏本体和加载器时，引擎的 [`Progress`] 原样转发。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PackProgress {
    Stage {
        index: usize,
        total: usize,
        label: String,
    },
    Files {
        done: usize,
        total: usize,
        bytes_done: u64,
        bytes_total: u64,
    },
    Log {
        level: LogLevel,
        message: String,
    },
    /// 引擎（原版 + 加载器）的进度，字段原样，不改。
    Engine {
        progress: Progress,
    },
}

/// 整合包安装的进度接收端。
///
/// 和 [`crate::types::ProgressSink`] 一样是同步 trait：接收方自己决定要不要缓冲，
/// 不要在 `emit` 里做耗时的事（它跑在下载 worker 里）。
pub trait PackProgressSink: Send + Sync {
    fn emit(&self, progress: PackProgress);
}

/// 什么都不做的接收端。
pub struct NullPackSink;

impl PackProgressSink for NullPackSink {
    fn emit(&self, _progress: PackProgress) {}
}

impl<F> PackProgressSink for F
where
    F: Fn(PackProgress) + Send + Sync,
{
    fn emit(&self, progress: PackProgress) {
        self(progress)
    }
}

/// 把 [`PackProgressSink`] 适配成引擎要的 [`crate::types::ProgressSink`]：原样转发。
pub(crate) struct EngineSink<'a> {
    pub sink: &'a dyn PackProgressSink,
}

impl crate::types::ProgressSink for EngineSink<'_> {
    fn emit(&self, progress: Progress) {
        self.sink.emit(PackProgress::Engine { progress });
    }
}

/// 把引擎的 `Files` / `Log` 折进整合包自己的进度（下载整合包文件时用）。
///
/// 引擎的 `Stage` 在这里没有意义（整合包的阶段是自己发的），直接丢掉。
pub(crate) struct FilesSink<'a> {
    pub sink: &'a dyn PackProgressSink,
}

impl crate::types::ProgressSink for FilesSink<'_> {
    fn emit(&self, progress: Progress) {
        match progress {
            Progress::Files {
                done,
                total,
                bytes_done,
                bytes_total,
                ..
            } => self.sink.emit(PackProgress::Files {
                done,
                total,
                bytes_done,
                bytes_total,
            }),
            Progress::Log { level, message } => {
                self.sink.emit(PackProgress::Log { level, message })
            }
            Progress::Stage { .. } => {}
        }
    }
}

// ---------------------------------------------------------------- 小工具

/// 平台返回的加载器名 → 引擎的枚举。认不出返回 `None`。
pub fn loader_kind(raw: &str) -> Option<LoaderKind> {
    LoaderKind::parse(raw)
}

/// 默认并发数，与 [`crate::types::InstallRequest`] 保持一致。
pub fn default_concurrency() -> usize {
    16
}

/// 整合包压缩包在磁盘上的落点：`<gameDir>/.mcdl/packs/<文件名>`。
///
/// 放在游戏目录里（而不是系统临时目录），这样重装同一个包时能直接命中缓存。
pub fn archive_cache_dir(game_dir: &std::path::Path) -> PathBuf {
    game_dir.join(".mcdl").join("packs")
}

/// 把文件名里的非法字符换掉，避免把 URL 里的怪字符直接写进路径。
pub fn safe_file_name(raw: &str) -> String {
    let name = raw
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("pack.zip")
        .trim()
        .to_string();
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    if cleaned.is_empty() {
        "pack.zip".to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_file_name_strips_dirs_and_bad_chars() {
        assert_eq!(safe_file_name("a/b/c.mrpack"), "c.mrpack");
        assert_eq!(safe_file_name(r"..\..\evil.zip"), "evil.zip");
        assert_eq!(safe_file_name("a:b?.zip"), "a_b_.zip");
        assert_eq!(safe_file_name("   "), "pack.zip");
        assert_eq!(safe_file_name(""), "pack.zip");
    }

    #[test]
    fn archive_file_prefers_primary_mrpack() {
        let version = PackVersion {
            files: vec![
                PackFile {
                    filename: "cosign-bundle.zip".into(),
                    url: "https://example.invalid/cosign.zip".into(),
                    ..Default::default()
                },
                PackFile {
                    filename: "pack.mrpack".into(),
                    url: "https://example.invalid/pack.mrpack".into(),
                    primary: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        assert_eq!(
            version.archive_file().map(|f| f.filename.as_str()),
            Some("pack.mrpack")
        );
    }

    #[test]
    fn archive_file_skips_versions_without_download_url() {
        // CurseForge 的作者可以关掉第三方分发，这时 downloadUrl 是 null。
        let version = PackVersion {
            files: vec![PackFile {
                filename: "pack.zip".into(),
                url: String::new(),
                primary: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(version.archive_file().is_none());
    }

    #[test]
    fn hit_loaders_come_from_categories() {
        let hit = PackHit {
            categories: vec![
                "fabric".into(),
                "optimization".into(),
                "fabric".into(),
                "neoforge".into(),
            ],
            ..Default::default()
        };
        assert_eq!(hit.loaders_from_categories(), ["fabric", "neoforge"]);
    }

    #[test]
    fn page_size_is_clamped() {
        let mut q = PackSearchQuery::new("x");
        q.limit = 0;
        assert_eq!(q.page_size(), 20);
        q.limit = 500;
        assert_eq!(q.page_size(), 50);
        q.limit = 7;
        assert_eq!(q.page_size(), 7);
    }

    #[test]
    fn loader_kind_parses_platform_names() {
        assert_eq!(loader_kind("fabric"), Some(LoaderKind::Fabric));
        assert_eq!(loader_kind("NeoForge"), Some(LoaderKind::NeoForge));
        assert_eq!(loader_kind("quilt"), Some(LoaderKind::Quilt));
        assert_eq!(loader_kind("liteloader"), None);
    }

    #[test]
    fn pack_progress_serializes_with_a_type_tag() {
        let value = serde_json::to_value(PackProgress::Files {
            done: 3,
            total: 10,
            bytes_done: 30,
            bytes_total: 100,
        })
        .unwrap();
        assert_eq!(value["type"], "files");
        assert_eq!(value["done"], 3);

        let value = serde_json::to_value(PackProgress::Engine {
            progress: Progress::Log {
                level: LogLevel::Info,
                message: "hi".into(),
            },
        })
        .unwrap();
        assert_eq!(value["type"], "engine");
        assert_eq!(value["progress"]["type"], "log");
    }

    #[test]
    fn archive_cache_dir_lives_under_game_dir() {
        let dir = archive_cache_dir(std::path::Path::new("C:/mc"));
        assert!(dir.ends_with(".mcdl/packs") || dir.ends_with(".mcdl\\packs"));
    }
}
