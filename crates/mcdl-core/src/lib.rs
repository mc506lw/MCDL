//! Minecraft 下载与安装引擎。
//!
//! 这个 crate 不依赖任何 UI 框架：CLI 和 Tauri 应用都只是它的调用方。
//! 分四层：
//!
//! - [`meta`]：Mojang 元数据（version_manifest / version json / rules 求值）
//! - [`net`]：HTTP 客户端与下载源（官方 / BMCLAPI）
//! - [`download`]：并发下载、断点、SHA1 校验
//! - [`loader`]：Fabric / Quilt / Forge / NeoForge 安装（调用官方 installer）
//! - [`java`]：Java 运行时探测与按需获取
//! - [`install`]：把上面几层编排成一次安装

pub mod download;
pub mod error;
pub mod install;
pub mod installed;
pub mod java;
pub mod loader;
pub mod meta;
pub mod net;
pub mod paths;
pub mod types;

pub use error::{Error, Result};
pub use install::Installer;
pub use installed::InstalledVersion;
pub use types::*;

/// 本库的版本号，写进 User-Agent 与日志。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 默认 User-Agent。Mojang 的接口要求能识别调用方。
pub fn user_agent() -> String {
    format!("mcdl/{VERSION} (+https://github.com/example/mcdl)")
}
