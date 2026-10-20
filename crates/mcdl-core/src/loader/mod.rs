//! 模组加载器安装。
//!
//! 一律调用官方 installer 的 headless 模式，不自己实现 Forge 的 processors：
//!
//! - Fabric / Quilt：`java -jar fabric-installer.jar client -dir <gameDir> -mcversion <v> -loader <v>`
//! - Forge / NeoForge：`java -jar forge-installer.jar --installClient`
//!
//! 好处是许可证干净（调用二进制不传染 LGPL）、行为与官方一致；
//! 代价是进度粒度只能靠解析 installer 的 stdout。

use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::net::HttpClient;
use crate::paths::GamePaths;
use crate::types::{LoaderKind, LoaderSpec, ProgressSink, Source};

/// 安装加载器需要的外部环境。
pub struct LoaderCtx<'a> {
    pub client: &'a HttpClient,
    pub paths: &'a GamePaths,
    pub java: &'a Path,
    pub source: Source,
}

#[derive(Debug, Clone, Default)]
pub struct LoaderResult {
    /// 装完后 `versions/` 下新增的版本目录名，例如 `1.21.1-forge-51.0.33`
    pub version_id: String,
    /// installer 写出的 JSON 里的主类（可空，启动器可自行读 JSON）
    pub main_class: Option<String>,
    /// installer 新增/改动的文件
    pub files: Vec<PathBuf>,
    /// installer 的完整输出，出错时原样带回给用户
    pub log: String,
}

/// 安装一个加载器。
pub async fn install(
    _ctx: &LoaderCtx<'_>,
    _spec: &LoaderSpec,
    _sink: &dyn ProgressSink,
) -> Result<LoaderResult> {
    todo!("core-loader: 实现加载器安装编排")
}

/// 某个加载器支持的 MC 版本列表，给 UI 做下拉框。
pub async fn supported_mc_versions(
    _client: &HttpClient,
    _kind: LoaderKind,
) -> Result<Vec<String>> {
    todo!("core-loader: 实现加载器版本查询")
}

/// 某个 MC 版本可用的加载器版本列表。
pub async fn available_loader_versions(
    _client: &HttpClient,
    _kind: LoaderKind,
    _mc_version: &str,
) -> Result<Vec<String>> {
    todo!("core-loader: 实现加载器版本列表查询")
}

/// 下载 installer jar 到临时目录，返回路径。
pub async fn fetch_installer(
    _client: &HttpClient,
    _kind: LoaderKind,
    _spec: &LoaderSpec,
    _cache_dir: &Path,
) -> Result<PathBuf> {
    todo!("core-loader: 实现 installer 下载")
}
