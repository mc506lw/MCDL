//! 对外契约类型：CLI、Tauri 命令层、UI 都只认这里的结构。
//!
//! 改动这里的字段等于改协议，需要同步 `docs/ARCHITECTURE.md`。

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------- 下载源

/// 文件来源。默认官方，镜像只能由用户显式选择。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// Mojang 官方 CDN。
    #[default]
    Official,
    /// BMCLAPI 社区镜像（第三方转发，界面上必须标明）。
    Bmclapi,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Official => "官方源",
            Source::Bmclapi => "BMCLAPI 镜像",
        }
    }
}

// ---------------------------------------------------------------- 版本

/// `version_manifest_v2.json` 里的一条。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionSummary {
    pub id: String,
    /// release / snapshot / old_beta / old_alpha
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(rename = "releaseTime", default)]
    pub release_time: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionManifest {
    pub latest: LatestVersions,
    pub versions: Vec<VersionSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatestVersions {
    pub release: String,
    pub snapshot: String,
}

impl VersionManifest {
    pub fn find(&self, id: &str) -> Option<&VersionSummary> {
        self.versions.iter().find(|v| v.id == id)
    }
}

/// 解析完 `inheritsFrom` 之后的完整版本描述。
#[derive(Debug, Clone, Default)]
pub struct VersionDetail {
    pub id: String,
    pub kind: String,
    pub main_class: String,
    /// 资源索引名（assets 目录下的 index 文件名去掉 .json）
    pub assets: String,
    pub asset_index: Option<AssetIndexRef>,
    pub client: Option<FileRef>,
    pub libraries: Vec<Library>,
    pub jvm_args: Vec<Arg>,
    pub game_args: Vec<Arg>,
    /// 1.12 及更早的整串启动参数
    pub legacy_args: Option<String>,
    pub logging: Option<LoggingFile>,
    pub java_major: Option<u32>,
}

/// 带 SHA1 的远程文件。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileRef {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// 资源索引里的逻辑路径，官方 JSON 里对应的是别的字段名，不直接写出。
    #[serde(default, skip_serializing)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssetIndexRef {
    pub id: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_size: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssetIndex {
    pub objects: HashMap<String, AssetObject>,
    /// 1.6 时代把资源铺进 `<gameDir>/resources/` 的布局
    #[serde(default)]
    pub map_to_resources: bool,
    /// 1.7.x 把资源铺进 `assets/virtual/<索引名>/` 的布局。
    ///
    /// 官方索引里的键名是 `virtual`，不是 `virtual_`——少了这个 rename 就永远是
    /// false，1.6–1.7.2 的游戏会没声音没语言。
    #[serde(rename = "virtual", default)]
    pub virtual_: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssetObject {
    pub hash: String,
    pub size: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoggingFile {
    pub argument: String,
    pub file: FileRef,
}

// ---------------------------------------------------------------- 库与规则

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Library {
    pub name: String,
    pub downloads: LibraryDownloads,
    /// 老版本 natives 的 classifier 选择表：os -> classifier 后缀
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub natives: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<Rule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract: Option<Extract>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LibraryDownloads {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<LibraryArtifact>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub classifiers: HashMap<String, LibraryArtifact>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LibraryArtifact {
    /// 我们自己算出来的落盘路径。官方 JSON 里没有这个字段，所以永不写出。
    #[serde(default, skip_serializing)]
    pub path: Option<String>,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Extract {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub action: RuleAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<OsRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<HashMap<String, bool>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleAction {
    Allow,
    Disallow,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OsRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// 求值 `rules` 用的环境。
#[derive(Debug, Clone)]
pub struct Env {
    /// windows / osx / linux
    pub os: String,
    /// x86_64 / x86 / arm64
    pub arch: String,
    pub features: HashMap<String, bool>,
}

impl Default for Env {
    fn default() -> Self {
        let os = match std::env::consts::OS {
            "macos" => "osx",
            other => other,
        };
        Self {
            os: os.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            features: HashMap::new(),
        }
    }
}

// ---------------------------------------------------------------- 启动参数

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Arg {
    Plain(String),
    Conditional { rules: Vec<Rule>, value: ArgValue },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ArgValue {
    One(String),
    Many(Vec<String>),
}

// ---------------------------------------------------------------- 下载
/// 一个待下载文件。
#[derive(Debug, Clone)]
pub struct DownloadJob {
    pub url: String,
    pub dest: PathBuf,
    pub sha1: Option<String>,
    pub size: Option<u64>,
    /// 展示用标签，例如 `assets/objects/ab/abcd…`
    pub label: String,
}

#[derive(Debug, Clone, Default)]
pub struct DownloadOutcome {
    pub downloaded: usize,
    pub skipped: usize,
    pub bytes: u64,
}

// ---------------------------------------------------------------- 加载器

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LoaderKind {
    Fabric,
    Quilt,
    Forge,
    NeoForge,
}

impl LoaderKind {
    pub fn label(self) -> &'static str {
        match self {
            LoaderKind::Fabric => "Fabric",
            LoaderKind::Quilt => "Quilt",
            LoaderKind::Forge => "Forge",
            LoaderKind::NeoForge => "NeoForge",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "fabric" => Some(Self::Fabric),
            "quilt" => Some(Self::Quilt),
            "forge" => Some(Self::Forge),
            "neoforge" | "neo" => Some(Self::NeoForge),
            _ => None,
        }
    }
}

/// `fabric`、`fabric@0.16.0`、`forge@47.2.0` 这类写法。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoaderSpec {
    pub kind: LoaderKind,
    pub mc_version: String,
    pub loader_version: Option<String>,
}

impl LoaderSpec {
    pub fn new(kind: LoaderKind, mc_version: impl Into<String>) -> Self {
        Self {
            kind,
            mc_version: mc_version.into(),
            loader_version: None,
        }
    }

    pub fn with_version(mut self, v: impl Into<String>) -> Self {
        self.loader_version = Some(v.into());
        self
    }

    /// `fabric@0.16.0` / `forge`
    pub fn parse(raw: &str, mc_version: &str) -> crate::Result<Self> {
        let (kind_part, ver) = match raw.split_once('@') {
            Some((k, v)) => (k, Some(v.to_string())),
            None => (raw, None),
        };
        let kind = LoaderKind::parse(kind_part)
            .ok_or_else(|| crate::Error::other(format!("不认识的加载器：{kind_part}")))?;
        Ok(Self {
            kind,
            mc_version: mc_version.to_string(),
            loader_version: ver,
        })
    }

    pub fn display(&self) -> String {
        match &self.loader_version {
            Some(v) => format!("{} {v}", self.kind.label()),
            None => self.kind.label().to_string(),
        }
    }
}

// ---------------------------------------------------------------- Java

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JavaRuntime {
    pub path: PathBuf,
    pub major: u32,
    pub version: String,
    pub vendor: String,
    pub arch: String,
}

// ---------------------------------------------------------------- 安装流程

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageKind {
    Java,
    Meta,
    Libraries,
    Assets,
    Client,
    Loader,
}

impl StageKind {
    pub fn label(self) -> &'static str {
        match self {
            StageKind::Java => "Java 运行时",
            StageKind::Meta => "版本元数据",
            StageKind::Libraries => "依赖库",
            StageKind::Assets => "资源文件",
            StageKind::Client => "游戏本体",
            StageKind::Loader => "模组加载器",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stage {
    pub kind: StageKind,
    pub files: usize,
    pub bytes: u64,
}

/// 一次安装请求。这是 CLI 和 GUI 唯一的入口参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallRequest {
    /// `.minecraft` 所在目录
    pub game_dir: PathBuf,
    pub mc_version: String,
    #[serde(default)]
    pub loader: Option<LoaderSpec>,
    #[serde(default)]
    pub source: Source,
    /// 并发下载数，默认 16
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    /// 指定 Java；不填则自动探测
    #[serde(default)]
    pub java: Option<PathBuf>,
    /// 跳过资源文件（只装能启动的最小集合）
    #[serde(default)]
    pub skip_assets: bool,
}

fn default_concurrency() -> usize {
    16
}

impl InstallRequest {
    pub fn new(game_dir: impl Into<PathBuf>, mc_version: impl Into<String>) -> Self {
        Self {
            game_dir: game_dir.into(),
            mc_version: mc_version.into(),
            loader: None,
            source: Source::Official,
            concurrency: 16,
            java: None,
            skip_assets: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstallReport {
    pub version_id: String,
    pub game_dir: PathBuf,
    pub files: usize,
    pub bytes: u64,
    pub duration_ms: u64,
    pub java: Option<PathBuf>,
    /// 装完后 `versions/` 下可以直接用的版本目录名
    pub installed_version: String,
}

/// 安装前的预估。只读元数据，不下载游戏文件。
///
/// 界面拿它回答「这一次要点多少、装到哪」，让用户在开始之前就知道代价。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstallPreview {
    pub version_id: String,
    pub kind: String,
    /// 需要的主版本号
    pub java_major: u32,
    /// 本机能不能直接用：`Some(路径)` 表示有，`None` 表示要下载
    pub java_path: Option<PathBuf>,
    pub libraries: usize,
    pub assets: usize,
    pub files: usize,
    pub bytes: u64,
    /// 资源索引本身的大小（下载它才能数出 assets 数量）
    pub index_bytes: u64,
    pub loader: Option<String>,
    pub stages: Vec<StageKind>,
}

// ---------------------------------------------------------------- 进度

/// 引擎向外抛的进度事件。
///
/// 事件里只放已经发生的事实，不放百分比：由接收方自己算，
/// 因为不同阶段的总量是逐步确定的。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Progress {
    Stage {
        kind: StageKind,
        index: usize,
        total: usize,
        label: String,
    },
    Files {
        kind: StageKind,
        done: usize,
        total: usize,
        bytes_done: u64,
        bytes_total: u64,
    },
    Log {
        level: LogLevel,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

/// 进度接收端。GUI 用 channel，CLI 用 stdout。
pub trait ProgressSink: Send + Sync {
    fn emit(&self, progress: Progress);
}

/// 什么都不做的接收端，给测试和库调用方用。
pub struct NullSink;

impl ProgressSink for NullSink {
    fn emit(&self, _progress: Progress) {}
}

impl<F> ProgressSink for F
where
    F: Fn(Progress) + Send + Sync,
{
    fn emit(&self, progress: Progress) {
        self(progress)
    }
}
