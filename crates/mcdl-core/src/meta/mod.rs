//! Mojang 元数据：版本清单、版本 JSON、`inheritsFrom` 合并、`rules` 求值。
//!
//! 协议文档：<https://minecraft.wiki/w/Client.json>
//! 这里只实现协议，不参考任何启动器的实现细节。

use std::collections::HashMap;

use serde_json::Value;

use crate::error::{Error, Result};
use crate::net::{self, HttpClient};
use crate::types::{
    Arg, AssetIndexRef, Env, Extract, FileRef, Library, LibraryArtifact, LibraryDownloads,
    LoggingFile, OsRule, Rule, RuleAction, Source, VersionDetail, VersionManifest, VersionSummary,
};

/// `inheritsFrom` 链的最大深度。超过这个数一定是 JSON 写错了（或者成了环）。
const MAX_INHERIT_DEPTH: usize = 16;

/// 拉取 `version_manifest_v2.json`。
///
/// 无论用户选哪个下载源，清单都走官方：镜像的清单经常滞后或改写过。
pub async fn fetch_manifest(client: &HttpClient, source: Source) -> Result<VersionManifest> {
    tracing::debug!(url = net::MANIFEST_URL, ?source, "拉取版本清单");
    let manifest: VersionManifest = client.get_json(net::MANIFEST_URL).await?;
    tracing::debug!(
        versions = manifest.versions.len(),
        latest_release = %manifest.latest.release,
        latest_snapshot = %manifest.latest.snapshot,
        "版本清单已就绪"
    );
    Ok(manifest)
}

/// 拉取某个版本的 JSON，并沿 `inheritsFrom` 链合并成完整描述。
pub async fn fetch_detail(
    client: &HttpClient,
    source: Source,
    summary: &VersionSummary,
) -> Result<VersionDetail> {
    // 从子往父收集，最后从根开始合并，这样「子覆盖父」天然成立。
    let mut chain: Vec<(String, Value)> = Vec::new();
    let mut next = Some(summary.clone());
    // 只有真的遇到 inheritsFrom 才去拉清单，普通原版版本不会多花一次请求。
    let mut manifest: Option<VersionManifest> = None;

    while let Some(cur) = next {
        if chain.len() >= MAX_INHERIT_DEPTH {
            return Err(Error::other(format!(
                "版本 {} 的 inheritsFrom 链超过 {MAX_INHERIT_DEPTH} 层，疑似循环继承",
                summary.id
            )));
        }

        let url = net::version_url(&cur.url);
        tracing::debug!(version = %cur.id, url, "拉取版本 JSON");
        let raw: Value = client.get_json(url).await?;

        let parent_id = raw
            .get("inheritsFrom")
            .and_then(Value::as_str)
            .map(str::to_owned);
        chain.push((cur.id.clone(), raw));

        next = match parent_id {
            None => None,
            Some(parent_id) => {
                if manifest.is_none() {
                    manifest = Some(fetch_manifest(client, source).await?);
                }
                let parent = manifest
                    .as_ref()
                    .and_then(|m| m.find(&parent_id))
                    .cloned()
                    .ok_or_else(|| {
                        Error::other(format!(
                            "版本 {} 继承了 {parent_id}，但版本清单里没有它",
                            cur.id
                        ))
                    })?;
                tracing::debug!(child = %cur.id, parent = %parent_id, "沿 inheritsFrom 继续向上");
                Some(parent)
            }
        };
    }

    let mut detail = VersionDetail::default();
    for (id, raw) in chain.iter().rev() {
        detail = merge_detail(detail, parse_detail(raw, id)?);
    }

    tracing::info!(
        version = %detail.id,
        libraries = detail.libraries.len(),
        inherits = chain.len().saturating_sub(1),
        "版本 JSON 解析完成"
    );
    Ok(detail)
}

/// 把子版本合并到父版本上。
///
/// - 标量（mainClass / assets / assetIndex / client / logging / javaVersion …）：子优先，子没有就留父的；
/// - `libraries`：父的在前、子的在后追加；只有「同名 + 同一个落盘路径 + 同样的 natives 形态」
///   才当成同一条用子版本替换（见 [`same_library`]）；
/// - `arguments`：父的在前、子的在后拼接（Forge 之类只在子版本里追加参数，拼掉父的会启动不起来）。
fn merge_detail(mut base: VersionDetail, child: VersionDetail) -> VersionDetail {
    if !child.id.is_empty() {
        base.id = child.id;
    }
    if !child.kind.is_empty() {
        base.kind = child.kind;
    }
    if !child.main_class.is_empty() {
        base.main_class = child.main_class;
    }
    if !child.assets.is_empty() {
        base.assets = child.assets;
    }
    if child.asset_index.is_some() {
        base.asset_index = child.asset_index;
    }
    if child.client.is_some() {
        base.client = child.client;
    }
    if child.logging.is_some() {
        base.logging = child.logging;
    }
    if child.java_major.is_some() {
        base.java_major = child.java_major;
    }
    if child.legacy_args.is_some() {
        base.legacy_args = child.legacy_args;
    }

    base.jvm_args.extend(child.jvm_args);
    base.game_args.extend(child.game_args);

    for lib in child.libraries {
        match base
            .libraries
            .iter_mut()
            .find(|existing| same_library(existing, &lib))
        {
            Some(slot) => *slot = lib,
            None => base.libraries.push(lib),
        }
    }
    base
}

/// 两条库记录是不是「同一条」。
///
/// 只看 maven name 不够：1.12.2–1.18.2 的 `ca.weblite:java-objc-bridge:1.0.0`
/// 在官方 JSON 里出现两次，name 和 artifact 路径完全相同，只有一条带 `natives`
/// （macOS 的 dylib）。按 name 去重会把带 natives 的那条覆盖掉，macOS 上就少解压
/// 一个动态库。所以 key 必须是「name + artifact 落盘路径 + natives 表」。
fn same_library(a: &Library, b: &Library) -> bool {
    a.name == b.name
        && artifact_path(a) == artifact_path(b)
        && a.natives == b.natives
}

fn artifact_path(lib: &Library) -> Option<&str> {
    lib.downloads
        .artifact
        .as_ref()
        .and_then(|artifact| artifact.path.as_deref())
}

/// 求值 `rules`：返回这个环境里该条目是否生效。
///
/// 规则按顺序匹配，最后一条命中的决定结果。`rules` 缺失（空列表）时默认允许；
/// 但**只要写了规则，没命中就按不允许算** —— 否则 `[{allow, os: osx}]` 这种
/// 「只在 macOS 上要」的库（1.20.4 的 java-objc-bridge、1.6.4 的 lwjgl nightly）
/// 会在 Windows 上也被算进来。
pub fn rules_allow(rules: &[Rule], env: &Env) -> bool {
    let mut decided: Option<bool> = None;
    for rule in rules {
        if rule_matches(rule, env) {
            decided = Some(matches!(rule.action, RuleAction::Allow));
        }
    }
    decided.unwrap_or(rules.is_empty())
}

fn rule_matches(rule: &Rule, env: &Env) -> bool {
    if let Some(os) = &rule.os {
        if !os_matches(os, env) {
            return false;
        }
    }
    if let Some(features) = &rule.features {
        for (name, wanted) in features {
            // 环境里没提这个 feature 就等于 false（官方启动器的默认值）。
            let actual = env.features.get(name).copied().unwrap_or(false);
            if actual != *wanted {
                return false;
            }
        }
    }
    true
}

fn os_matches(os: &OsRule, env: &Env) -> bool {
    if let Some(name) = &os.name {
        if !name.eq_ignore_ascii_case(&env.os) {
            return false;
        }
    }
    if let Some(arch) = &os.arch {
        if canon_arch(arch) != canon_arch(&env.arch) {
            return false;
        }
    }
    if let Some(version) = &os.version {
        // `os.version` 是正则（例如 `^10\.5\.\d$`），本 crate 不引入正则依赖，
        // 也没法可靠拿到 System.getProperty("os.version")。判不了就按「不命中」处理：
        // 这类规则只出现在 1.6 时代给 osx 10.5 准备的条目上，退回默认结果是对的。
        tracing::trace!(version, os = %env.os, "os.version 规则无法判定，按不命中处理");
        return false;
    }
    true
}

/// 归一化 CPU 架构名：`std::env::consts::ARCH` 与 Mojang 规则里的写法并不一致
/// （`aarch64` vs `arm64`、`x86_64` vs `amd64`）。
fn canon_arch(arch: &str) -> String {
    let lower = arch.trim().to_ascii_lowercase();
    match lower.as_str() {
        "aarch64" | "arm64" | "arm64e" => "arm64".to_string(),
        "x86_64" | "amd64" | "x64" => "x86_64".to_string(),
        "i386" | "i486" | "i586" | "i686" | "x86" | "ia32" => "x86".to_string(),
        "arm" | "arm32" | "armv7" | "armv7l" => "arm32".to_string(),
        other => other.to_string(),
    }
}

/// 把原始 JSON 解析成 [`VersionDetail`]（不含继承合并）。
pub fn parse_detail(raw: &Value, id: &str) -> Result<VersionDetail> {
    let mut detail = VersionDetail {
        id: str_at(raw, "id").unwrap_or_else(|| id.to_string()),
        kind: str_at(raw, "type").unwrap_or_default(),
        main_class: str_at(raw, "mainClass").unwrap_or_default(),
        assets: str_at(raw, "assets").unwrap_or_default(),
        ..Default::default()
    };

    if let Some(ai) = raw.get("assetIndex").filter(|v| v.is_object()) {
        detail.asset_index = Some(AssetIndexRef {
            id: str_at(ai, "id").unwrap_or_default(),
            url: str_at(ai, "url").unwrap_or_default(),
            sha1: str_at(ai, "sha1"),
            size: u64_at(ai, "size"),
            total_size: u64_at(ai, "totalSize"),
        });
    }

    if let Some(client) = raw.pointer("/downloads/client").filter(|v| v.is_object()) {
        match str_at(client, "url").filter(|u| !u.is_empty()) {
            Some(url) => {
                detail.client = Some(FileRef {
                    url,
                    sha1: str_at(client, "sha1"),
                    size: u64_at(client, "size"),
                    path: None,
                })
            }
            // 没有 url 就没法下载；留空比让下游拼出一个空 URL 去请求更好定位。
            None => tracing::warn!(version = %detail.id, "downloads.client 缺少 url，跳过游戏本体"),
        }
    }

    if let Some(libs) = raw.get("libraries").and_then(Value::as_array) {
        detail.libraries = libs.iter().filter_map(parse_library).collect();
    }

    if let Some(args) = raw.get("arguments") {
        detail.jvm_args = parse_args(args.get("jvm"), &detail.id, "jvm");
        detail.game_args = parse_args(args.get("game"), &detail.id, "game");
    }
    // 1.12 及更早：整串参数，启动时按空格切开
    if let Some(legacy) = raw.get("minecraftArguments").and_then(Value::as_str) {
        detail.legacy_args = Some(legacy.to_string());
    }

    if let Some(logging) = raw.pointer("/logging/client").filter(|v| v.is_object()) {
        let argument = str_at(logging, "argument").unwrap_or_default();
        if let Some(file) = logging.get("file").filter(|v| v.is_object()) {
            if let Some(url) = str_at(file, "url").filter(|u| !u.is_empty()) {
                detail.logging = Some(LoggingFile {
                    argument,
                    file: FileRef {
                        url,
                        sha1: str_at(file, "sha1"),
                        size: u64_at(file, "size"),
                        // 日志配置的文件名（例如 client-1.12.xml）放在 path 里
                        path: str_at(file, "id"),
                    },
                });
            }
        }
    }

    detail.java_major = raw
        .pointer("/javaVersion/majorVersion")
        .and_then(Value::as_u64)
        .map(|v| v as u32);

    Ok(detail)
}

fn parse_library(raw: &Value) -> Option<Library> {
    let name = raw.get("name").and_then(Value::as_str)?;
    let mut lib = Library {
        name: name.to_string(),
        ..Default::default()
    };

    if let Some(downloads) = raw.get("downloads") {
        let mut parsed = LibraryDownloads::default();
        if let Some(artifact) = downloads.get("artifact").filter(|v| v.is_object()) {
            parsed.artifact = Some(parse_artifact(artifact));
        }
        if let Some(classifiers) = downloads.get("classifiers").and_then(Value::as_object) {
            parsed.classifiers = classifiers
                .iter()
                .map(|(key, value)| (key.clone(), parse_artifact(value)))
                .collect();
        }
        lib.downloads = parsed;
    }

    if let Some(natives) = raw.get("natives").and_then(Value::as_object) {
        let table: HashMap<String, String> = natives
            .iter()
            .filter_map(|(os, classifier)| {
                classifier
                    .as_str()
                    .map(|value| (os.clone(), value.to_string()))
            })
            .collect();
        if !table.is_empty() {
            lib.natives = Some(table);
        }
    }

    if let Some(rules) = raw.get("rules").and_then(Value::as_array) {
        lib.rules = rules
            .iter()
            .filter_map(|rule| match serde_json::from_value::<Rule>(rule.clone()) {
                Ok(rule) => Some(rule),
                Err(e) => {
                    // 单条规则看不懂不该让整个版本装不上，记一条日志跳过。
                    tracing::warn!(library = %lib.name, error = %e, "规则解析失败，已忽略");
                    None
                }
            })
            .collect();
    }

    if let Some(extract) = raw.get("extract") {
        let exclude = extract
            .get("exclude")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        lib.extract = Some(Extract { exclude });
    }

    Some(lib)
}

fn parse_artifact(raw: &Value) -> LibraryArtifact {
    LibraryArtifact {
        path: str_at(raw, "path"),
        url: str_at(raw, "url").unwrap_or_default(),
        sha1: str_at(raw, "sha1"),
        size: u64_at(raw, "size"),
    }
}

fn parse_args(raw: Option<&Value>, version: &str, kind: &str) -> Vec<Arg> {
    let Some(Value::Array(items)) = raw else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| match serde_json::from_value::<Arg>(item.clone()) {
            Ok(arg) => Some(arg),
            Err(e) => {
                tracing::warn!(version, kind, error = %e, "启动参数条目解析失败，已忽略");
                None
            }
        })
        .collect()
}

fn str_at(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn u64_at(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------ rules 求值

    struct RuleCase {
        name: &'static str,
        rules: &'static str,
        os: &'static str,
        arch: &'static str,
        features: &'static [(&'static str, bool)],
        want: bool,
    }

    fn parse_rules(json: &str) -> Vec<Rule> {
        serde_json::from_str(json).expect("测试用的规则 JSON 必须合法")
    }

    fn env_with(os: &str, arch: &str, features: &[(&str, bool)]) -> Env {
        Env {
            os: os.to_string(),
            arch: arch.to_string(),
            features: features
                .iter()
                .map(|(k, v)| (k.to_string(), *v))
                .collect(),
        }
    }

    /// 表驱动：windows / osx / linux × x86_64 / x86 / arm64 × features 各种组合。
    const CASES: &[RuleCase] = &[
        // 1. 没有规则 → 允许
        RuleCase {
            name: "无规则默认允许",
            rules: "[]",
            os: "windows",
            arch: "x86_64",
            features: &[],
            want: true,
        },
        // 2. 裸 allow：任何环境都允许（下面另有 9 环境遍历测试）
        RuleCase {
            name: "裸 allow",
            rules: r#"[{"action":"allow"}]"#,
            os: "linux",
            arch: "arm64",
            features: &[],
            want: true,
        },
        // 3. 1.12.2 的常见写法：先 allow，再单独 disallow osx
        RuleCase {
            name: "allow + disallow osx（windows）",
            rules: r#"[{"action":"allow"},{"action":"disallow","os":{"name":"osx"}}]"#,
            os: "windows",
            arch: "x86_64",
            features: &[],
            want: true,
        },
        RuleCase {
            name: "allow + disallow osx（windows/x86）",
            rules: r#"[{"action":"allow"},{"action":"disallow","os":{"name":"osx"}}]"#,
            os: "windows",
            arch: "x86",
            features: &[],
            want: true,
        },
        RuleCase {
            name: "allow + disallow osx（linux/arm64）",
            rules: r#"[{"action":"allow"},{"action":"disallow","os":{"name":"osx"}}]"#,
            os: "linux",
            arch: "arm64",
            features: &[],
            want: true,
        },
        RuleCase {
            name: "allow + disallow osx（osx）",
            rules: r#"[{"action":"allow"},{"action":"disallow","os":{"name":"osx"}}]"#,
            os: "osx",
            arch: "x86_64",
            features: &[],
            want: false,
        },
        RuleCase {
            name: "allow + disallow osx（osx/arm64）",
            rules: r#"[{"action":"allow"},{"action":"disallow","os":{"name":"osx"}}]"#,
            os: "osx",
            arch: "arm64",
            features: &[],
            want: false,
        },
        // 4. 只在某个 OS 上要的库（1.20.4 的 java-objc-bridge）
        RuleCase {
            name: "仅 osx 的库（osx）",
            rules: r#"[{"action":"allow","os":{"name":"osx"}}]"#,
            os: "osx",
            arch: "x86_64",
            features: &[],
            want: true,
        },
        RuleCase {
            name: "仅 osx 的库（windows）",
            rules: r#"[{"action":"allow","os":{"name":"osx"}}]"#,
            os: "windows",
            arch: "x86_64",
            features: &[],
            want: false,
        },
        RuleCase {
            name: "仅 osx 的库（linux）",
            rules: r#"[{"action":"allow","os":{"name":"osx"}}]"#,
            os: "linux",
            arch: "x86_64",
            features: &[],
            want: false,
        },
        // 5. 32 位专用的 JVM 参数（1.21.1 的 -Xss1M）
        RuleCase {
            name: "仅 x86 架构（windows/x86）",
            rules: r#"[{"action":"allow","os":{"arch":"x86"}}]"#,
            os: "windows",
            arch: "x86",
            features: &[],
            want: true,
        },
        RuleCase {
            name: "仅 x86 架构（windows/x86_64）",
            rules: r#"[{"action":"allow","os":{"arch":"x86"}}]"#,
            os: "windows",
            arch: "x86_64",
            features: &[],
            want: false,
        },
        RuleCase {
            name: "仅 x86 架构（windows/arm64）",
            rules: r#"[{"action":"allow","os":{"arch":"x86"}}]"#,
            os: "windows",
            arch: "arm64",
            features: &[],
            want: false,
        },
        RuleCase {
            name: "仅 x86 架构（linux/x86）",
            rules: r#"[{"action":"allow","os":{"arch":"x86"}}]"#,
            os: "linux",
            arch: "x86",
            features: &[],
            want: true,
        },
        RuleCase {
            name: "仅 x86 架构（osx/x86_64）",
            rules: r#"[{"action":"allow","os":{"arch":"x86"}}]"#,
            os: "osx",
            arch: "x86_64",
            features: &[],
            want: false,
        },
        // 6. os.name + os.arch 同时约束
        RuleCase {
            name: "windows 32 位（匹配）",
            rules: r#"[{"action":"allow","os":{"name":"windows","arch":"x86"}}]"#,
            os: "windows",
            arch: "x86",
            features: &[],
            want: true,
        },
        RuleCase {
            name: "windows 32 位（64 位环境不匹配）",
            rules: r#"[{"action":"allow","os":{"name":"windows","arch":"x86"}}]"#,
            os: "windows",
            arch: "x86_64",
            features: &[],
            want: false,
        },
        RuleCase {
            name: "windows 32 位（linux 不匹配）",
            rules: r#"[{"action":"allow","os":{"name":"windows","arch":"x86"}}]"#,
            os: "linux",
            arch: "x86",
            features: &[],
            want: false,
        },
        // 7. arm64：规则里写 arm64，环境里是 aarch64 也要匹配
        RuleCase {
            name: "arm64 规则（windows/arm64）",
            rules: r#"[{"action":"allow","os":{"arch":"arm64"}}]"#,
            os: "windows",
            arch: "arm64",
            features: &[],
            want: true,
        },
        RuleCase {
            name: "arm64 规则（windows/aarch64 别名）",
            rules: r#"[{"action":"allow","os":{"arch":"arm64"}}]"#,
            os: "windows",
            arch: "aarch64",
            features: &[],
            want: true,
        },
        RuleCase {
            name: "arm64 规则（linux/arm64）",
            rules: r#"[{"action":"allow","os":{"arch":"arm64"}}]"#,
            os: "linux",
            arch: "arm64",
            features: &[],
            want: true,
        },
        RuleCase {
            name: "arm64 规则（x86_64 不匹配）",
            rules: r#"[{"action":"allow","os":{"arch":"arm64"}}]"#,
            os: "windows",
            arch: "x86_64",
            features: &[],
            want: false,
        },
        // 8. features：环境里没写等于 false
        RuleCase {
            name: "demo 用户（命中）",
            rules: r#"[{"action":"allow","features":{"is_demo_user":true}}]"#,
            os: "windows",
            arch: "x86_64",
            features: &[("is_demo_user", true)],
            want: true,
        },
        RuleCase {
            name: "demo 用户（没有这个 feature）",
            rules: r#"[{"action":"allow","features":{"is_demo_user":true}}]"#,
            os: "windows",
            arch: "x86_64",
            features: &[],
            want: false,
        },
        RuleCase {
            name: "demo 用户（feature 显式 false）",
            rules: r#"[{"action":"allow","features":{"is_demo_user":true}}]"#,
            os: "windows",
            arch: "x86_64",
            features: &[("is_demo_user", false)],
            want: false,
        },
        RuleCase {
            name: "要求 feature 为 false（缺失即 false，命中）",
            rules: r#"[{"action":"allow","features":{"has_custom_resolution":false}}]"#,
            os: "linux",
            arch: "x86_64",
            features: &[],
            want: true,
        },
        RuleCase {
            name: "要求 feature 为 false（实际为 true，不命中）",
            rules: r#"[{"action":"allow","features":{"has_custom_resolution":false}}]"#,
            os: "linux",
            arch: "x86_64",
            features: &[("has_custom_resolution", true)],
            want: false,
        },
        RuleCase {
            name: "os + features 同时约束（都命中）",
            rules: r#"[{"action":"allow","os":{"name":"windows"},"features":{"is_quick_play_singleplayer":true}}]"#,
            os: "windows",
            arch: "x86_64",
            features: &[("is_quick_play_singleplayer", true)],
            want: true,
        },
        RuleCase {
            name: "os + features 同时约束（feature 缺）",
            rules: r#"[{"action":"allow","os":{"name":"windows"},"features":{"is_quick_play_singleplayer":true}}]"#,
            os: "windows",
            arch: "x86_64",
            features: &[],
            want: false,
        },
        RuleCase {
            name: "os + features 同时约束（os 不匹配）",
            rules: r#"[{"action":"allow","os":{"name":"windows"},"features":{"is_quick_play_singleplayer":true}}]"#,
            os: "linux",
            arch: "x86_64",
            features: &[("is_quick_play_singleplayer", true)],
            want: false,
        },
        // 9. 顺序：最后一条命中的决定结果
        RuleCase {
            name: "disallow 在前 allow 在后（windows 命中后者）",
            rules: r#"[{"action":"disallow"},{"action":"allow","os":{"name":"windows"}}]"#,
            os: "windows",
            arch: "x86_64",
            features: &[],
            want: true,
        },
        RuleCase {
            name: "disallow 在前 allow 在后（linux 停在 disallow）",
            rules: r#"[{"action":"disallow"},{"action":"allow","os":{"name":"windows"}}]"#,
            os: "linux",
            arch: "x86_64",
            features: &[],
            want: false,
        },
        RuleCase {
            name: "allow 在前 disallow 在后（后者通吃）",
            rules: r#"[{"action":"allow"},{"action":"disallow"}]"#,
            os: "linux",
            arch: "x86_64",
            features: &[],
            want: false,
        },
        // 10. os.version 是正则，判不了就按不命中（1.6.4 给 osx 10.5 的条目）
        RuleCase {
            name: "os.version 无法判定时 disallow 不生效",
            rules: r#"[{"action":"allow"},{"action":"disallow","os":{"name":"osx","version":"^10\\.5\\.\\d$"}}]"#,
            os: "osx",
            arch: "x86_64",
            features: &[],
            want: true,
        },
        RuleCase {
            name: "只有 os.version 的 allow 判不了 → 默认拒绝",
            rules: r#"[{"action":"allow","os":{"name":"osx","version":"^10\\.5\\.\\d$"}}]"#,
            os: "osx",
            arch: "x86_64",
            features: &[],
            want: false,
        },
    ];

    #[test]
    fn rules_allow_table() {
        for case in CASES {
            let env = env_with(case.os, case.arch, case.features);
            let got = rules_allow(&parse_rules(case.rules), &env);
            assert_eq!(
                got, case.want,
                "用例「{}」：os={} arch={} features={:?} rules={}",
                case.name, case.os, case.arch, case.features, case.rules
            );
        }
    }

    /// 9 个平台组合 × 几种典型规则，确保没有哪个 os/arch 漏判。
    #[test]
    fn rules_allow_covers_every_platform() {
        let envs = [
            ("windows", "x86_64"),
            ("windows", "x86"),
            ("windows", "arm64"),
            ("osx", "x86_64"),
            ("osx", "arm64"),
            ("linux", "x86_64"),
            ("linux", "x86"),
            ("linux", "arm64"),
            ("linux", "aarch64"),
        ];

        let allow_all = parse_rules(r#"[{"action":"allow"}]"#);
        let windows_only = parse_rules(r#"[{"action":"allow","os":{"name":"windows"}}]"#);
        let no_rules: Vec<Rule> = Vec::new();

        for (os, arch) in envs {
            let env = env_with(os, arch, &[]);
            assert!(rules_allow(&no_rules, &env), "无规则应当允许：{os}/{arch}");
            assert!(rules_allow(&allow_all, &env), "裸 allow 应当允许：{os}/{arch}");
            assert_eq!(
                rules_allow(&windows_only, &env),
                os == "windows",
                "仅 windows 的规则判错了：{os}/{arch}"
            );
        }
    }

    #[test]
    fn canon_arch_aliases() {
        assert_eq!(canon_arch("aarch64"), canon_arch("arm64"));
        assert_eq!(canon_arch("X86_64"), canon_arch("amd64"));
        assert_eq!(canon_arch("i686"), canon_arch("x86"));
        assert_ne!(canon_arch("x86"), canon_arch("x86_64"));
        assert_ne!(canon_arch("arm64"), canon_arch("x86_64"));
    }

    // ------------------------------------------------------------ 版本 JSON

    const MODERN: &str = r#"{
      "id": "1.21.1",
      "type": "release",
      "mainClass": "net.minecraft.client.main.Main",
      "assets": "17",
      "assetIndex": {
        "id": "17",
        "sha1": "15200809663f45954c2c9a33ce88849e61aaaecc",
        "size": 449557,
        "totalSize": 826050937,
        "url": "https://piston-meta.mojang.com/v1/packages/15200809663f45954c2c9a33ce88849e61aaaecc/17.json"
      },
      "downloads": {
        "client": {
          "sha1": "30c73b1c5da787909b2f73340419fdf13b9def88",
          "size": 26836906,
          "url": "https://piston-data.mojang.com/v1/objects/30c73b1c5da787909b2f73340419fdf13b9def88/client.jar"
        }
      },
      "javaVersion": { "component": "java-runtime-delta", "majorVersion": 21 },
      "logging": {
        "client": {
          "argument": "-Dlog4j.configurationFile=${path}",
          "file": {
            "id": "client-1.12.xml",
            "sha1": "bd65e7d2e3c237be76cfbef4c2405033d7f91521",
            "size": 888,
            "url": "https://piston-data.mojang.com/v1/objects/bd65e7d2e3c237be76cfbef4c2405033d7f91521/client-1.12.xml"
          },
          "type": "log4j2-xml"
        }
      },
      "arguments": {
        "game": [
          "--username", "${auth_player_name}",
          {
            "rules": [{ "action": "allow", "features": { "has_custom_resolution": true } }],
            "value": ["--width", "${resolution_width}", "--height", "${resolution_height}"]
          }
        ],
        "jvm": [
          {
            "rules": [{ "action": "allow", "os": { "name": "osx" } }],
            "value": ["-XstartOnFirstThread"]
          },
          {
            "rules": [{ "action": "allow", "os": { "arch": "x86" } }],
            "value": "-Xss1M"
          }
        ]
      },
      "libraries": [
        {
          "name": "com.mojang:brigadier:1.3.10",
          "downloads": {
            "artifact": {
              "path": "com/mojang/brigadier/1.3.10/brigadier-1.3.10.jar",
              "sha1": "7f7e0c2b5a1f0c9b4d2a9e8e5f3b1c0d9e8f7a6b",
              "size": 77116,
              "url": "https://libraries.minecraft.net/com/mojang/brigadier/1.3.10/brigadier-1.3.10.jar"
            }
          }
        },
        {
          "name": "ca.weblite:java-objc-bridge:1.1",
          "rules": [{ "action": "allow", "os": { "name": "osx" } }],
          "downloads": {
            "artifact": {
              "path": "ca/weblite/java-objc-bridge/1.1/java-objc-bridge-1.1.jar",
              "sha1": "1227f9e0666314f9de41477e3ec277e542ed7f7b",
              "size": 1330045,
              "url": "https://libraries.minecraft.net/ca/weblite/java-objc-bridge/1.1/java-objc-bridge-1.1.jar"
            }
          }
        }
      ]
    }"#;

    const LEGACY: &str = r#"{
      "id": "1.12.2",
      "type": "release",
      "mainClass": "net.minecraft.client.main.Main",
      "assets": "1.12",
      "downloads": {
        "client": {
          "sha1": "0f275bc1547d01fa5f56ba34bdc87d981ee12daf",
          "size": 10180113,
          "url": "https://piston-data.mojang.com/v1/objects/0f275bc1547d01fa5f56ba34bdc87d981ee12daf/client.jar"
        }
      },
      "minecraftArguments": "--username ${auth_player_name} --version ${version_name} --gameDir ${game_directory}",
      "libraries": [
        {
          "name": "org.lwjgl.lwjgl:lwjgl-platform:2.9.4-nightly-20150209",
          "downloads": {
            "artifact": {
              "path": "org/lwjgl/lwjgl/lwjgl-platform/2.9.4-nightly-20150209/lwjgl-platform-2.9.4-nightly-20150209.jar",
              "sha1": "b04f3ee8f5e43fa3b162981b50bb72fe1acabb33",
              "size": 22,
              "url": "https://libraries.minecraft.net/org/lwjgl/lwjgl/lwjgl-platform/2.9.4-nightly-20150209/lwjgl-platform-2.9.4-nightly-20150209.jar"
            },
            "classifiers": {
              "natives-windows": {
                "path": "org/lwjgl/lwjgl/lwjgl-platform/2.9.4-nightly-20150209/lwjgl-platform-2.9.4-nightly-20150209-natives-windows.jar",
                "sha1": "c1d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6a7b8c9d0",
                "size": 123456,
                "url": "https://libraries.minecraft.net/org/lwjgl/lwjgl/lwjgl-platform/2.9.4-nightly-20150209/lwjgl-platform-2.9.4-nightly-20150209-natives-windows.jar"
              }
            }
          },
          "natives": { "windows": "natives-windows", "linux": "natives-linux", "osx": "natives-osx" },
          "extract": { "exclude": ["META-INF/"] },
          "rules": [{ "action": "allow" }]
        }
      ]
    }"#;

    #[test]
    fn parse_modern_detail() {
        let raw: Value = serde_json::from_str(MODERN).unwrap();
        let detail = parse_detail(&raw, "1.21.1").unwrap();

        assert_eq!(detail.id, "1.21.1");
        assert_eq!(detail.kind, "release");
        assert_eq!(detail.main_class, "net.minecraft.client.main.Main");
        assert_eq!(detail.assets, "17");
        assert_eq!(detail.java_major, Some(21));

        let index = detail.asset_index.expect("1.13+ 必须有 assetIndex");
        assert_eq!(index.id, "17");
        assert_eq!(index.size, Some(449557));
        assert_eq!(index.total_size, Some(826050937));

        let client = detail.client.expect("必须有 client.jar");
        assert_eq!(client.size, Some(26836906));
        assert!(client.sha1.is_some());

        let logging = detail.logging.expect("必须有 log4j 配置");
        assert_eq!(logging.argument, "-Dlog4j.configurationFile=${path}");
        assert_eq!(logging.file.path.as_deref(), Some("client-1.12.xml"));
        assert_eq!(logging.file.size, Some(888));

        // 1.13+ 走 arguments：普通字符串和带 rules 的对象都要能解析
        assert_eq!(detail.game_args.len(), 3);
        assert_eq!(detail.jvm_args.len(), 2);
        assert!(matches!(&detail.game_args[0], Arg::Plain(s) if s == "--username"));
        match &detail.game_args[2] {
            Arg::Conditional { rules, value } => {
                assert_eq!(rules.len(), 1);
                match value {
                    crate::types::ArgValue::Many(items) => assert_eq!(items.len(), 4),
                    _ => panic!("has_custom_resolution 的 value 是数组"),
                }
            }
            _ => panic!("第 3 个 game 参数应该是条件参数"),
        }
        assert!(detail.legacy_args.is_none());

        assert_eq!(detail.libraries.len(), 2);
        let brigadier = &detail.libraries[0];
        assert_eq!(brigadier.name, "com.mojang:brigadier:1.3.10");
        let artifact = brigadier.downloads.artifact.as_ref().unwrap();
        assert_eq!(artifact.size, Some(77116));
        assert_eq!(
            artifact.path.as_deref(),
            Some("com/mojang/brigadier/1.3.10/brigadier-1.3.10.jar")
        );
        assert!(brigadier.rules.is_empty());

        // 解析出来的 rules 要能直接喂给 rules_allow
        let objc = &detail.libraries[1];
        assert_eq!(objc.rules.len(), 1);
        assert!(rules_allow(&objc.rules, &env_with("osx", "x86_64", &[])));
        assert!(!rules_allow(&objc.rules, &env_with("windows", "x86_64", &[])));
    }

    #[test]
    fn parse_legacy_detail() {
        let raw: Value = serde_json::from_str(LEGACY).unwrap();
        let detail = parse_detail(&raw, "1.12.2").unwrap();

        assert_eq!(detail.id, "1.12.2");
        assert!(detail.game_args.is_empty(), "老版本没有 arguments");
        assert_eq!(
            detail.legacy_args.as_deref(),
            Some(
                "--username ${auth_player_name} --version ${version_name} --gameDir ${game_directory}"
            )
        );
        assert_eq!(detail.java_major, None);

        let lib = &detail.libraries[0];
        assert_eq!(lib.natives.as_ref().unwrap().len(), 3);
        assert_eq!(
            lib.natives.as_ref().unwrap().get("windows").map(String::as_str),
            Some("natives-windows")
        );
        assert_eq!(lib.downloads.classifiers.len(), 1);
        assert!(lib.downloads.classifiers.contains_key("natives-windows"));
        assert_eq!(
            lib.extract.as_ref().unwrap().exclude,
            vec!["META-INF/".to_string()]
        );
        assert!(rules_allow(&lib.rules, &env_with("windows", "x86_64", &[])));
    }

    #[test]
    fn parse_detail_survives_missing_fields() {
        // 残缺 JSON 不该 panic，缺什么就留空。
        let raw: Value = serde_json::from_str(r#"{"id":"weird"}"#).unwrap();
        let detail = parse_detail(&raw, "fallback").unwrap();
        assert_eq!(detail.id, "weird");
        assert!(detail.main_class.is_empty());
        assert!(detail.client.is_none());
        assert!(detail.libraries.is_empty());
        assert!(detail.java_major.is_none());

        // 没有 id 字段时用调用方给的 id
        let raw: Value = serde_json::from_str(r#"{"mainClass":"x"}"#).unwrap();
        assert_eq!(parse_detail(&raw, "1.0").unwrap().id, "1.0");
    }

    // ------------------------------------------------------------ inheritsFrom 合并

    fn detail(json: &str, id: &str) -> VersionDetail {
        let raw: Value = serde_json::from_str(json).unwrap();
        parse_detail(&raw, id).unwrap()
    }

    #[test]
    fn merge_child_overrides_scalars_and_appends_libraries() {
        let parent = detail(MODERN, "1.21.1");
        let child = detail(
            r#"{
              "id": "1.21.1-forge-51.0.33",
              "type": "release",
              "mainClass": "cpw.mods.bootstraplauncher.BootstrapLauncher",
              "inheritsFrom": "1.21.1",
              "libraries": [
                {
                  "name": "net.minecraftforge:forge:1.21.1-51.0.33",
                  "downloads": {
                    "artifact": {
                      "path": "net/minecraftforge/forge/1.21.1-51.0.33/forge-1.21.1-51.0.33.jar",
                      "url": "https://maven.minecraftforge.net/net/minecraftforge/forge/1.21.1-51.0.33/forge-1.21.1-51.0.33.jar"
                    }
                  }
                }
              ],
              "arguments": {
                "game": ["--launchTarget", "forgeclient"]
              }
            }"#,
            "1.21.1-forge-51.0.33",
        );

        let merged = merge_detail(parent, child);

        assert_eq!(merged.id, "1.21.1-forge-51.0.33");
        assert_eq!(merged.main_class, "cpw.mods.bootstraplauncher.BootstrapLauncher");
        // 子版本没写的字段保留父版本
        assert_eq!(merged.assets, "17");
        assert_eq!(merged.java_major, Some(21));
        assert!(merged.client.is_some());
        assert!(merged.asset_index.is_some());
        // 库是追加：父的 2 个 + 子的 1 个
        assert_eq!(merged.libraries.len(), 3);
        assert_eq!(merged.libraries[0].name, "com.mojang:brigadier:1.3.10");
        assert_eq!(merged.libraries[2].name, "net.minecraftforge:forge:1.21.1-51.0.33");
        // 参数是拼接：父的 3 个 game + 子的 2 个
        assert_eq!(merged.game_args.len(), 5);
        assert_eq!(merged.jvm_args.len(), 2);
    }

    #[test]
    fn merge_replaces_same_named_library() {
        let parent = detail(MODERN, "1.21.1");
        let child = detail(
            r#"{
              "id": "1.21.1-patched",
              "inheritsFrom": "1.21.1",
              "libraries": [
                {
                  "name": "com.mojang:brigadier:1.3.10",
                  "downloads": {
                    "artifact": {
                      "path": "com/mojang/brigadier/1.3.10/brigadier-1.3.10.jar",
                      "url": "https://mirror.example/brigadier.jar"
                    }
                  }
                }
              ]
            }"#,
            "1.21.1-patched",
        );

        let merged = merge_detail(parent, child);
        assert_eq!(merged.libraries.len(), 2, "同名库不应该变成两份");
        let brigadier = merged
            .libraries
            .iter()
            .find(|l| l.name == "com.mojang:brigadier:1.3.10")
            .unwrap();
        assert_eq!(
            brigadier.downloads.artifact.as_ref().unwrap().url,
            "https://mirror.example/brigadier.jar",
            "同名库应该用子版本的内容"
        );
    }

    /// 1.12.2–1.18.2 的真实形态：同名、同一个 artifact 路径，但一条带 natives 一条不带。
    /// 按 name 去重会把带 natives 的那条吃掉（macOS 就少了 dylib），所以两条都必须保留。
    #[test]
    fn merge_keeps_same_name_with_different_natives() {
        let raw: Value = serde_json::from_str(
            r#"{
              "id": "1.12.2",
              "libraries": [
                {
                  "name": "ca.weblite:java-objc-bridge:1.0.0",
                  "downloads": {
                    "artifact": {
                      "path": "ca/weblite/java-objc-bridge/1.0.0/java-objc-bridge-1.0.0.jar",
                      "sha1": "6ef160c3133a78de015830860197602ca1c855d3",
                      "size": 40502,
                      "url": "https://libraries.minecraft.net/ca/weblite/java-objc-bridge/1.0.0/java-objc-bridge-1.0.0.jar"
                    },
                    "classifiers": {
                      "natives-osx": {
                        "path": "ca/weblite/java-objc-bridge/1.0.0/java-objc-bridge-1.0.0-natives-osx.jar",
                        "sha1": "c1d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6a7b8c9d0",
                        "size": 1234,
                        "url": "https://libraries.minecraft.net/ca/weblite/java-objc-bridge/1.0.0/java-objc-bridge-1.0.0-natives-osx.jar"
                      }
                    }
                  },
                  "natives": { "osx": "natives-osx" },
                  "rules": [{ "action": "allow", "os": { "name": "osx" } }]
                },
                {
                  "name": "ca.weblite:java-objc-bridge:1.0.0",
                  "downloads": {
                    "artifact": {
                      "path": "ca/weblite/java-objc-bridge/1.0.0/java-objc-bridge-1.0.0.jar",
                      "sha1": "6ef160c3133a78de015830860197602ca1c855d3",
                      "size": 40502,
                      "url": "https://libraries.minecraft.net/ca/weblite/java-objc-bridge/1.0.0/java-objc-bridge-1.0.0.jar"
                    }
                  },
                  "rules": [{ "action": "allow", "os": { "name": "osx" } }]
                }
              ]
            }"#,
        )
        .unwrap();

        let parsed = parse_detail(&raw, "1.12.2").unwrap();
        assert_eq!(parsed.libraries.len(), 2, "解析阶段不能丢条目");

        let merged = merge_detail(VersionDetail::default(), parsed);
        assert_eq!(merged.libraries.len(), 2, "同名但 natives 形态不同的两条都要留");
        assert_eq!(
            merged
                .libraries
                .iter()
                .filter(|l| l.natives.is_some())
                .count(),
            1,
            "带 natives 的那条必须还在，否则 macOS 解压不到 dylib"
        );
    }

    /// 真实网络：1.21.1 的清单 + 版本 JSON，验证解析结果与官方数据一致。
    /// 默认忽略，手动跑：`cargo test -p mcdl-core -- --ignored`
    #[tokio::test]
    #[ignore = "需要真实网络"]
    async fn fetch_detail_from_official_network() {
        let client = HttpClient::new().unwrap();
        let manifest = fetch_manifest(&client, Source::Official).await.unwrap();
        let summary = manifest.find("1.21.1").expect("清单里应该有 1.21.1");
        let detail = fetch_detail(&client, Source::Official, summary).await.unwrap();

        assert_eq!(detail.id, "1.21.1");
        assert_eq!(detail.kind, "release");
        assert_eq!(detail.main_class, "net.minecraft.client.main.Main");
        assert_eq!(detail.assets, "17");
        assert_eq!(detail.java_major, Some(21));
        assert_eq!(detail.client.as_ref().unwrap().size, Some(26836906));
        assert!(detail.libraries.len() > 90);
        assert!(!detail.game_args.is_empty());
        assert!(!detail.jvm_args.is_empty());
        assert!(detail.legacy_args.is_none());
    }
}
