//! Modrinth 整合包接口。
//!
//! 只用公开文档里的三个端点：
//!
//! - `GET /v2/search?query=…&facets=…&index=…` —— 搜索
//! - `GET /v2/project/{slug|id}` —— 详情
//! - `GET /v2/project/{slug|id}/version` —— 版本与文件
//!
//! 不需要 API key。Modrinth 要求请求带能识别调用方的 User-Agent，
//! 这一点由 [`crate::net::HttpClient`] 统一保证。

use crate::error::{Error, Result};
use crate::net::HttpClient;
use crate::types::LoaderKind;

use super::{
    PackDep, PackFile, PackHit, PackProject, PackSearchQuery, PackSearchResult, PackSort,
    PackSource, PackVersion,
};

const API: &str = "https://api.modrinth.com/v2";

/// 拼一个带查询参数的 URL。参数值由 `url` 负责百分号编码。
fn api_url(path: &str, params: &[(&str, String)]) -> String {
    let mut url = reqwest::Url::parse(&format!("{API}{path}")).expect("Modrinth 基址是常量");
    {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in params {
            pairs.append_pair(key, value);
        }
    }
    url.to_string()
}

/// 搜索用的 `facets`：永远是「整合包」，再按需叠加版本与加载器。
///
/// 格式是 Modrinth 文档里的「数组的数组」：外层 AND，内层 OR。
pub fn facets(q: &PackSearchQuery) -> String {
    let mut groups: Vec<String> = vec![r#"["project_type:modpack"]"#.to_string()];
    if let Some(version) = q.mc_version.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        groups.push(format!(r#"["versions:{}"]"#, escape_facet(version)));
    }
    if let Some(loader) = q.loader {
        groups.push(format!(
            r#"["categories:{}"]"#,
            loader_facet(loader)
        ));
    }
    format!("[{}]", groups.join(","))
}

/// facet 里的值不能带引号和方括号，否则会破坏整段 JSON。
fn escape_facet(raw: &str) -> String {
    raw.chars()
        .filter(|c| !matches!(c, '"' | '[' | ']' | '\\'))
        .collect()
}

fn loader_facet(kind: LoaderKind) -> &'static str {
    match kind {
        LoaderKind::Fabric => "fabric",
        LoaderKind::Quilt => "quilt",
        LoaderKind::Forge => "forge",
        LoaderKind::NeoForge => "neoforge",
    }
}

fn sort_index(sort: PackSort) -> &'static str {
    match sort {
        PackSort::Relevance => "relevance",
        PackSort::Downloads => "downloads",
        PackSort::Updated => "updated",
        PackSort::Newest => "newest",
    }
}

/// 搜索请求的完整 URL（单测直接断言它）。
pub fn search_url(q: &PackSearchQuery) -> String {
    let limit = q.page_size().to_string();
    let offset = q.offset.to_string();
    api_url(
        "/search",
        &[
            ("query", q.query.trim().to_string()),
            ("facets", facets(q)),
            ("index", sort_index(q.sort).to_string()),
            ("limit", limit),
            ("offset", offset),
        ],
    )
}

pub async fn search(client: &HttpClient, q: &PackSearchQuery) -> Result<PackSearchResult> {
    let url = search_url(q);
    let raw: SearchResponse = client.get_json(&url).await?;
    let hits = raw
        .hits
        .into_iter()
        .map(|hit| {
            // 搜索接口不给独立的 loaders 字段，加载器名混在 categories 里。
            let loaders = super::PackHit {
                categories: hit.categories.clone(),
                ..Default::default()
            }
            .loaders_from_categories();
            PackHit {
                source: PackSource::Modrinth,
                id: hit.project_id,
                slug: hit.slug,
                title: hit.title,
                description: hit.description,
                author: non_empty(hit.author),
                icon_url: hit.icon_url.filter(|s| !s.is_empty()),
                downloads: hit.downloads,
                followers: hit.follows,
                categories: hit.categories,
                game_versions: hit.versions,
                loaders,
                date_modified: hit.date_modified,
                latest_version: hit.latest_version,
                license: hit.license,
            }
        })
        .collect();
    Ok(PackSearchResult {
        hits,
        total: raw.total_hits,
        offset: raw.offset,
        limit: raw.limit,
    })
}

pub async fn project(client: &HttpClient, id: &str) -> Result<PackProject> {
    let id = id.trim();
    if id.is_empty() {
        return Err(Error::other("没有指定要查的整合包"));
    }
    let url = api_url(&format!("/project/{id}"), &[]);
    let raw: ProjectResponse = client.get_json(&url).await?;
    Ok(PackProject {
        source: PackSource::Modrinth,
        id: raw.id,
        slug: raw.slug,
        title: raw.title,
        description: raw.description,
        body: raw.body,
        author: None, // 项目详情里只有 team id，作者名要从搜索结果拿
        icon_url: raw.icon_url.filter(|s| !s.is_empty()),
        downloads: raw.downloads,
        followers: raw.followers,
        categories: raw.categories,
        game_versions: raw.game_versions,
        loaders: raw.loaders,
        license: raw.license.as_ref().and_then(license_name),
        source_url: raw.source_url.filter(|s| !s.is_empty()),
        issues_url: raw.issues_url.filter(|s| !s.is_empty()),
        wiki_url: raw.wiki_url.filter(|s| !s.is_empty()),
        published: raw.published,
        updated: raw.updated,
    })
}

pub async fn versions(client: &HttpClient, id: &str) -> Result<Vec<PackVersion>> {
    let id = id.trim();
    if id.is_empty() {
        return Err(Error::other("没有指定要查的整合包"));
    }
    let url = api_url(&format!("/project/{id}/version"), &[]);
    let raw: Vec<VersionResponse> = client.get_json(&url).await?;
    Ok(raw.into_iter().map(version_from).collect())
}

// ---------------------------------------------------------------- 响应结构

#[derive(Debug, serde::Deserialize)]
struct SearchResponse {
    #[serde(default)]
    hits: Vec<Hit>,
    #[serde(default)]
    offset: usize,
    #[serde(default)]
    limit: usize,
    #[serde(default)]
    total_hits: usize,
}

#[derive(Debug, Default, serde::Deserialize)]
struct Hit {
    #[serde(default)]
    project_id: String,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    categories: Vec<String>,
    /// 支持的 MC 版本
    #[serde(default)]
    versions: Vec<String>,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    follows: u64,
    #[serde(default)]
    icon_url: Option<String>,
    #[serde(default)]
    date_modified: String,
    #[serde(default)]
    latest_version: Option<String>,
    #[serde(default)]
    license: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct ProjectResponse {
    #[serde(default)]
    id: String,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    categories: Vec<String>,
    #[serde(default)]
    game_versions: Vec<String>,
    #[serde(default)]
    loaders: Vec<String>,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    followers: u64,
    #[serde(default)]
    icon_url: Option<String>,
    #[serde(default)]
    source_url: Option<String>,
    #[serde(default)]
    issues_url: Option<String>,
    #[serde(default)]
    wiki_url: Option<String>,
    #[serde(default)]
    published: String,
    #[serde(default)]
    updated: String,
    /// 项目详情里是对象 `{ id, name, url }`，搜索结果里是字符串，两种都接。
    #[serde(default)]
    license: Option<serde_json::Value>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct VersionResponse {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    version_number: String,
    #[serde(default)]
    version_type: String,
    #[serde(default)]
    game_versions: Vec<String>,
    #[serde(default)]
    loaders: Vec<String>,
    #[serde(default)]
    date_published: String,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    files: Vec<FileEntry>,
    #[serde(default)]
    dependencies: Vec<DepEntry>,
}

#[derive(Debug, Default, serde::Deserialize)]
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

#[derive(Debug, Default, serde::Deserialize)]
struct FileHashes {
    #[serde(default)]
    sha1: Option<String>,
    #[serde(default)]
    sha512: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct DepEntry {
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    version_id: Option<String>,
    #[serde(default)]
    file_name: Option<String>,
    #[serde(default)]
    dependency_type: String,
}

fn version_from(raw: VersionResponse) -> PackVersion {
    PackVersion {
        id: raw.id,
        name: raw.name,
        version_number: raw.version_number,
        version_type: raw.version_type,
        game_versions: raw.game_versions,
        loaders: raw.loaders,
        date_published: raw.date_published,
        downloads: raw.downloads,
        files: raw
            .files
            .into_iter()
            .map(|f| PackFile {
                filename: f.filename,
                url: f.url,
                size: f.size,
                sha1: f.hashes.sha1,
                sha512: f.hashes.sha512,
                primary: f.primary,
            })
            .collect(),
        dependencies: raw
            .dependencies
            .into_iter()
            .map(|d| PackDep {
                project_id: d.project_id,
                version_id: d.version_id,
                file_name: d.file_name,
                dependency_type: d.dependency_type,
            })
            .collect(),
    }
}

fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// 许可证字段两种形态：项目详情里是对象，搜索结果里是字符串。
fn license_name(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => non_empty(s.clone()),
        serde_json::Value::Object(map) => map
            .get("name")
            .or_else(|| map.get("id"))
            .and_then(|v| v.as_str())
            .map(str::to_string),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query() -> PackSearchQuery {
        PackSearchQuery::new("sodium")
    }

    #[test]
    fn facets_always_restrict_to_modpacks() {
        assert_eq!(facets(&query()), r#"[["project_type:modpack"]]"#);
    }

    #[test]
    fn facets_add_version_and_loader_groups() {
        let mut q = query();
        q.mc_version = Some("1.21.1".into());
        q.loader = Some(LoaderKind::Fabric);
        assert_eq!(
            facets(&q),
            r#"[["project_type:modpack"],["versions:1.21.1"],["categories:fabric"]]"#
        );
    }

    #[test]
    fn facets_use_platform_names_for_every_loader() {
        let mut q = query();
        for (kind, name) in [
            (LoaderKind::Fabric, "fabric"),
            (LoaderKind::Quilt, "quilt"),
            (LoaderKind::Forge, "forge"),
            (LoaderKind::NeoForge, "neoforge"),
        ] {
            q.loader = Some(kind);
            assert!(
                facets(&q).contains(&format!("categories:{name}")),
                "{kind:?} 的 facet 不对：{}",
                facets(&q)
            );
        }
    }

    /// 用户填的版本号里带引号也不能把 facets 的 JSON 结构弄坏。
    #[test]
    fn facet_values_are_sanitized() {
        let mut q = query();
        q.mc_version = Some(r#"1.21"]],"x:["#.into());
        let text = facets(&q);
        // 外层一组 + 两个内层组 = 3 对括号，注入的方括号必须已经被丢掉
        assert_eq!(text.matches('[').count(), 3, "结构不能被注入：{text}");
        assert_eq!(text.matches(']').count(), 3);
        assert!(!text.contains('"'), "值里的引号要被丢掉：{text}");
        assert_eq!(
            text,
            r#"[["project_type:modpack"],["versions:1.21,x:"]]"#,
            "注入的部分只能变成无害的字符：{text}"
        );
    }

    #[test]
    fn search_url_carries_every_parameter() {
        let mut q = PackSearchQuery::new("better mc");
        q.mc_version = Some("1.21.1".into());
        q.loader = Some(LoaderKind::NeoForge);
        q.sort = PackSort::Downloads;
        q.limit = 12;
        q.offset = 24;

        let url = search_url(&q);
        assert!(url.starts_with("https://api.modrinth.com/v2/search?"), "{url}");
        assert!(url.contains("query=better+mc") || url.contains("query=better%20mc"), "{url}");
        assert!(url.contains("index=downloads"), "{url}");
        assert!(url.contains("limit=12"), "{url}");
        assert!(url.contains("offset=24"), "{url}");
        // facets 里的方括号/引号必须被编码，不能原样出现
        assert!(!url.contains("project_type:modpack\"]"), "{url}");
        assert!(url.contains("project_type%3Amodpack"), "{url}");
        assert!(url.contains("neoforge"), "{url}");
    }

    #[test]
    fn sort_index_maps_to_documented_values() {
        assert_eq!(sort_index(PackSort::Relevance), "relevance");
        assert_eq!(sort_index(PackSort::Downloads), "downloads");
        assert_eq!(sort_index(PackSort::Updated), "updated");
        assert_eq!(sort_index(PackSort::Newest), "newest");
    }

    #[test]
    fn license_field_accepts_object_and_string() {
        assert_eq!(
            license_name(&serde_json::json!({"id":"MIT","name":"MIT License"})).as_deref(),
            Some("MIT License")
        );
        assert_eq!(
            license_name(&serde_json::json!("GPL-3.0")).as_deref(),
            Some("GPL-3.0")
        );
        assert_eq!(license_name(&serde_json::json!(null)), None);
    }

    #[test]
    fn version_response_maps_files_and_dependencies() {
        let raw: VersionResponse = serde_json::from_str(
            r#"{
                "id": "Wo44jriL",
                "name": "1.0 for 1.21.1",
                "version_number": "1.0",
                "version_type": "release",
                "game_versions": ["1.21.1"],
                "loaders": ["fabric"],
                "date_published": "2026-01-01T00:00:00Z",
                "downloads": 12,
                "files": [
                    { "filename": "pack.mrpack", "url": "https://cdn.example/pack.mrpack",
                      "size": 100, "primary": true,
                      "hashes": { "sha1": "aa", "sha512": "bb" } }
                ],
                "dependencies": [
                    { "project_id": "P1", "version_id": "V1", "dependency_type": "required" }
                ]
            }"#,
        )
        .unwrap();
        let version = version_from(raw);
        assert_eq!(version.id, "Wo44jriL");
        assert_eq!(version.archive_file().unwrap().sha1.as_deref(), Some("aa"));
        assert_eq!(version.dependencies[0].dependency_type, "required");
    }

    // ------------------------------------------------------------ 真实网络
    //
    // 手动跑：`cargo test -p mcdl-core pack::modrinth -- --ignored --nocapture`

    /// 真实请求一次搜索，证明端点、facets、解析这条链路是通的。
    #[tokio::test]
    #[ignore = "需要真实网络（Modrinth API）"]
    async fn search_modpacks_over_network() {
        let client = HttpClient::new().unwrap();
        let mut q = PackSearchQuery::new("optimized");
        q.limit = 3;
        let result = search(&client, &q).await.unwrap();

        println!(
            "total_hits = {}，本页 {} 条",
            result.total,
            result.hits.len()
        );
        for hit in &result.hits {
            println!(
                "- {}（{}）下载 {} · 作者 {} · 版本 {:?} · 分类 {:?}",
                hit.title,
                hit.slug,
                hit.downloads,
                hit.author.as_deref().unwrap_or("-"),
                hit.game_versions,
                hit.categories
            );
        }
        assert!(!result.hits.is_empty(), "搜索应该有结果");
        assert!(result.hits.iter().all(|h| !h.id.is_empty()));
        assert!(result.total > 0);

        // 详情 + 版本：安装要用到的另外两个端点
        let slug = &result.hits[0].slug;
        let project = project(&client, slug).await.unwrap();
        println!("项目：{} / {} / 许可 {:?}", project.title, project.slug, project.license);
        assert_eq!(project.slug, *slug);

        let versions = versions(&client, slug).await.unwrap();
        println!("版本数 = {}", versions.len());
        let first = versions.first().expect("至少有一个版本");
        let archive = first.archive_file().expect("最新版应该有可下载的整合包文件");
        println!(
            "最新版：{}（{}）· {} 字节 · sha1 {:?}",
            first.version_number, archive.filename, archive.size, archive.sha1
        );
        assert!(archive.filename.to_ascii_lowercase().ends_with(".mrpack"));
    }
}
