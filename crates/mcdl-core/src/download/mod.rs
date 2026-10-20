//! 并发下载、SHA1 校验、断点续传、natives 解压。
//!
//! 设计要点：
//! - 已存在且校验通过的文件直接跳过（这就是"断点续传"：按文件粒度）
//! - 并发用固定大小的 worker 池，进度按累计字节抛给 [`ProgressSink`]
//! - 校验失败的文件删掉重下一次，再失败才报错

use std::path::Path;

use crate::error::Result;
use crate::net::HttpClient;
use crate::types::{DownloadJob, DownloadOutcome, ProgressSink, StageKind};

/// 批量下载。`stage` 只用于进度事件里的归类。
pub async fn run_batch(
    _client: &HttpClient,
    _jobs: Vec<DownloadJob>,
    _concurrency: usize,
    _sink: &dyn ProgressSink,
    _stage: StageKind,
) -> Result<DownloadOutcome> {
    todo!("core-net: 实现并发批量下载")
}

/// 下载单个文件，必要时校验。目标已存在且校验通过时直接返回。
pub async fn download_one(_client: &HttpClient, _job: &DownloadJob) -> Result<()> {
    todo!("core-net: 实现单文件下载")
}

/// 算文件的 SHA1（小写十六进制）。
pub fn sha1_file(_path: &Path) -> Result<String> {
    todo!("core-net: 实现 SHA1 计算")
}

/// 校验文件；没有期望值时返回 `true`。
pub fn verify(_path: &Path, _expected: Option<&str>) -> Result<bool> {
    todo!("core-net: 实现文件校验")
}

/// 把 natives jar 解压到版本目录下的 `natives/`，跳过 `extract.exclude` 里的路径。
pub fn extract_natives(
    _jar: &Path,
    _dest: &Path,
    _exclude: &[String],
) -> Result<usize> {
    todo!("core-net: 实现 natives 解压")
}
