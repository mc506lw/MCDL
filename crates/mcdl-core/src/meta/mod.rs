//! Mojang 元数据：版本清单、版本 JSON、`inheritsFrom` 合并、`rules` 求值。
//!
//! 协议文档：<https://minecraft.wiki/w/Client.json>
//! 这里只实现协议，不参考任何启动器的实现细节。

use crate::error::Result;
use crate::net::HttpClient;
use crate::types::{Env, Rule, Source, VersionDetail, VersionManifest, VersionSummary};

/// 拉取 `version_manifest_v2.json`。
///
/// 无论用户选哪个下载源，清单都走官方：镜像的清单经常滞后或改写过。
pub async fn fetch_manifest(_client: &HttpClient, _source: Source) -> Result<VersionManifest> {
    todo!("core-net: 实现版本清单拉取")
}

/// 拉取某个版本的 JSON，并沿 `inheritsFrom` 链合并成完整描述。
pub async fn fetch_detail(
    _client: &HttpClient,
    _source: Source,
    _summary: &VersionSummary,
) -> Result<VersionDetail> {
    todo!("core-net: 实现版本 JSON 拉取与 inheritsFrom 合并")
}

/// 求值 `rules`：返回这个环境里该条目是否生效。
///
/// 规则按顺序匹配，最后一条命中的决定结果；一条都没命中时默认允许。
pub fn rules_allow(_rules: &[Rule], _env: &Env) -> bool {
    todo!("core-net: 实现 rules 求值")
}

/// 把原始 JSON 解析成 [`VersionDetail`]（不含继承合并）。
pub fn parse_detail(_raw: &serde_json::Value, _id: &str) -> Result<VersionDetail> {
    todo!("core-net: 实现版本 JSON 解析")
}
