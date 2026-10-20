//! Java 运行时：探测本机、按版本需求挑选、缺失时从 Adoptium 取一份。
//!
//! Adoptium（Eclipse Temurin）的二进制可以自由再分发，是这里唯一的内置来源。

use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::net::HttpClient;
use crate::types::{JavaRuntime, ProgressSink};

/// 扫本机：`JAVA_HOME`、`PATH`、各平台常见安装目录。
pub fn detect() -> Vec<JavaRuntime> {
    todo!("core-loader: 实现 Java 探测")
}

/// 在已探测到的运行时里挑一个能用的：优先精确匹配主版本。
pub fn pick<'a>(runtimes: &'a [JavaRuntime], major: u32) -> Option<&'a JavaRuntime> {
    runtimes
        .iter()
        .filter(|j| j.major >= major)
        .min_by_key(|j| j.major)
        .or_else(|| runtimes.iter().max_by_key(|j| j.major))
}

/// 版本 JSON 里没写 `javaVersion` 时，按 MC 版本推断需要的主版本号。
///
/// | MC | Java |
/// | --- | --- |
/// | ≤ 1.16.5 | 8 |
/// | 1.17 – 1.20.4 | 17 |
/// | 1.20.5 – 1.21.x | 21 |
/// | 26.x+ | 21 起，按 JSON 为准 |
pub fn required_major(_mc_version: &str) -> u32 {
    todo!("core-loader: 实现 Java 版本推断")
}

/// 从 Adoptium 下载并解压一个 JRE 到 `dest`，返回 `java` 可执行文件路径。
pub async fn ensure(
    _client: &HttpClient,
    _major: u32,
    _dest: &Path,
    _sink: &dyn ProgressSink,
) -> Result<PathBuf> {
    todo!("core-loader: 实现 Adoptium JRE 获取")
}
