//! CurseForge 整合包接口。
//!
//! 官方 API 要 `x-api-key`，**这个仓库里不存任何 key**：
//! 用户在界面上填一次，存在本机 `localStorage` 里，调用时随参数传进来。
//! 没填时这里给一句能照着做的提示，而不是一个 401。
//!
//! 端点（公开文档 `docs.curseforge.com`）：
//!
//! - `GET /v1/mods/search?gameId=432&classId=4471&…`
//! - `GET /v1/mods/{id}`
//! - `GET /v1/mods/{id}/files?pageSize=50`
//! - `GET /v1/mods/{modId}/files/{fileId}`（安装时按 `manifest.json` 逐条解析）
//!
//! 一部分作者关掉了「允许第三方分发」，这时 `downloadUrl` 是 `null`。
//! 这种情况不猜 CDN 地址，直接告诉用户哪几个文件要手动装。

use serde::de::DeserializeOwned;

use crate::error::{Error, Result};
use crate::net::HttpClient;
use crate::types::LoaderKind;

use super::{
    PackFile, PackHit, PackProject, PackSearchQuery, PackSearchResult, PackSort, PackSource,
    PackVersion, CURSEFORGE_KEY_URL,
};

const API: &str = "https://api.curseforge.com/v1";
/// Minecraft 在 CurseForge 上的 gameId。
const GAME_ID: u32 = 432;
/// 「整合包」这个分类的 classId。
const CLASS_MODPACK: u32 = 4471;

/// 取出可用的 key，没有就给一句能照着做的提示。
pub fn require_key(key: Option<&str>) -> Result<&str> {
    match key.map(str::trim).filter(|k| !k.is_empty()) {
        Some(k) => Ok(k),
        None => Err(Error::other(format!(
            "CurseForge 需要 API key。在「整合包」页的 CurseForge 面板里填入你申请的 key\
             （申请地址 {CURSEFORGE_KEY_URL}），或者直接搜 Modrinth —— 那边不需要 key。"
        ))),
    }
}

fn api_url(path: &str, params: &[(&str, String)]) -> String {
    let mut url = reqwest::Url::parse(&format!("{API}{path}")).expect("CurseForge 基址是常量");
    {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in params {
            pairs.append_pair(key, value);
        }
    }
    url.to_string()
}

fn sort_field(sort: PackSort) -> &'static str {
    match sort {
        // CurseForge 没有「相关度」这个排序，用 Featured 兜底
        PackSort::Relevance => "1",
        PackSort::Downloads => "6",
        PackSort::Updated => "3",
        PackSort::Newest => "3",
    }
}

fn mod_loader_type(kind: LoaderKind) -> &'static str {
    match kind {
        LoaderKind::Forge => "1",
        LoaderKind::Fabric => "4",
        LoaderKind::Quilt => "5",
        LoaderKind::NeoForge => "6",
    }
}

/// 搜索请求的完整 URL（单测直接断言它）。
pub fn search_url(q: &PackSearchQuery) -> String {
    let mut params: Vec<(&str, String)> = vec![
        ("gameId", GAME_ID.to_string()),
        ("classId", CLASS_MODPACK.to_string()),
        ("searchFilter", q.query.trim().to_string()),
        ("sortField", sort_field(q.sort).to_string()),
        ("sortOrder", "desc".to_string()),
        ("pageSize", q.page_size().to_string()),
        ("index", q.offset.to_string()),
    ];
    if let Some(version) = q.mc_version.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        params.push(("gameVersion", version.to_string()));
    }
    if let Some(kind) = q.loader {
        params.push(("modLoaderType", mod_loader_type(kind).to_string()));
    }
    api_url("/mods/search", &params)
}

/// 带 `x-api-key` 的 GET + JSON 解析。
///
/// 不走 [`HttpClient::get_json`]：那个没有自定义请求头的能力，而 CurseForge 只认这个头。
async fn get_json<T: DeserializeOwned>(client: &HttpClient, url: &str, key: &str) -> Result<T> {
    let resp = client
        .raw()
        .get(url)
        .header("x-api-key", key)
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|e| Error::Http(format!("GET {url} 失败：{e}")))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(Error::Http(match status.as_u16() {
            401 | 403 => format!(
                "CurseForge 拒绝了这个 API key（HTTP {status}）。\
                 去 {CURSEFORGE_KEY_URL} 确认 key 有效，或改用 Modrinth。"
            ),
            404 => format!("CurseForge 上没有这个整合包（GET {url} 返回 404）"),
            429 => format!("CurseForge 限流了（HTTP 429），等一会儿再试（GET {url}）"),
            _ => format!("GET {url} 返回 HTTP {status}"),
        }));
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| Error::Http(format!("读取 {url} 的响应体失败：{e}")))?;
    serde_json::from_slice(&bytes).map_err(|e| {
        Error::other(format!(
            "{url} 返回的不是合法 JSON（{} 字节）：{e}",
            bytes.len()
        ))
    })
}

pub async fn search(client: &HttpClient, q: &PackSearchQuery) -> Result<PackSearchResult> {
    let key = require_key(q.api_key.as_deref())?;
    let url = search_url(q);
    let raw: ListResponse<ModResponse> = get_json(client, &url, key).await?;
    let hits = raw
        .data
        .into_iter()
        .map(|mod_| {
            // 搜索响应里没有独立的版本字段，但带 `latestFilesIndexes`，
            // 从那里能认出「最新文件支持哪些 MC 版本 / 哪些加载器」。
            let (game_versions, loaders) = split_versions_loaders(&mod_.latest_files_indexes);
            PackHit {
                source: PackSource::Curseforge,
                id: mod_.id.to_string(),
                slug: mod_.slug,
                title: mod_.name,
                description: mod_.summary,
                author: mod_.authors.first().map(|a| a.name.clone()),
                icon_url: mod_
                    .logo
                    .as_ref()
                    .and_then(|l| l.thumbnail_url.clone().or_else(|| l.url.clone()))
                    .filter(|s| !s.is_empty()),
                downloads: mod_.download_count,
                followers: 0,
                categories: mod_.categories.into_iter().map(|c| c.name).collect(),
                game_versions,
                loaders,
                date_modified: mod_.date_modified,
                latest_version: mod_.latest_files.first().map(|f| f.id.to_string()),
                license: None,
            }
        })
        .collect();
    Ok(PackSearchResult {
        hits,
        total: raw.pagination.total_count as usize,
        offset: raw.pagination.index as usize,
        limit: raw.pagination.page_size as usize,
    })
}

pub async fn project(client: &HttpClient, key: Option<&str>, id: &str) -> Result<PackProject> {
    let key = require_key(key)?;
    let id = numeric_id(id)?;
    let url = api_url(&format!("/mods/{id}"), &[]);
    let raw: DataResponse<ModResponse> = get_json(client, &url, key).await?;
    let mod_ = raw.data;
    let (game_versions, loaders) = split_versions_loaders(&mod_.latest_files_indexes);
    Ok(PackProject {
        source: PackSource::Curseforge,
        id: mod_.id.to_string(),
        slug: mod_.slug,
        title: mod_.name,
        description: mod_.summary,
        body: mod_.description,
        author: mod_.authors.first().map(|a| a.name.clone()),
        icon_url: mod_
            .logo
            .as_ref()
            .and_then(|l| l.url.clone().or_else(|| l.thumbnail_url.clone()))
            .filter(|s| !s.is_empty()),
        downloads: mod_.download_count,
        followers: 0,
        categories: mod_.categories.into_iter().map(|c| c.name).collect(),
        game_versions,
        loaders,
        license: None,
        source_url: mod_
            .links
            .source_url
            .clone()
            .or_else(|| mod_.links.website_url.clone()),
        issues_url: mod_.links.issues_url.clone(),
        wiki_url: mod_.links.wiki_url.clone(),
        published: mod_.date_created,
        updated: mod_.date_modified,
    })
}

pub async fn versions(
    client: &HttpClient,
    key: Option<&str>,
    id: &str,
) -> Result<Vec<PackVersion>> {
    let key = require_key(key)?;
    let id = numeric_id(id)?;
    // 一页 50 条足够界面翻；整合包的版本本来就不多。
    let url = api_url(
        &format!("/mods/{id}/files"),
        &[("pageSize", "50".to_string()), ("index", "0".to_string())],
    );
    let raw: ListResponse<FileResponse> = get_json(client, &url, key).await?;
    Ok(raw.data.into_iter().map(version_from_file).collect())
}

/// 按 `manifest.json` 里的一条 `(projectID, fileID)` 取真实下载地址。
pub async fn resolve_file(
    client: &HttpClient,
    key: Option<&str>,
    mod_id: u64,
    file_id: u64,
) -> Result<PackFile> {
    let key = require_key(key)?;
    let url = api_url(&format!("/mods/{mod_id}/files/{file_id}"), &[]);
    let raw: DataResponse<FileResponse> = get_json(client, &url, key).await?;
    Ok(file_from(raw.data))
}

fn numeric_id(raw: &str) -> Result<u64> {
    raw.trim()
        .parse::<u64>()
        .map_err(|_| Error::other(format!("CurseForge 的整合包 id 是数字，收到的是「{raw}」")))
}

fn file_from(raw: FileResponse) -> PackFile {
    // hashes 里 algo 1 是 sha1、2 是 md5。只有 sha1 能拿来校验。
    let sha1 = raw
        .hashes
        .iter()
        .find(|h| h.algo == 1)
        .map(|h| h.value.clone())
        .filter(|v| !v.is_empty());
    PackFile {
        filename: raw.file_name,
        url: raw.download_url.unwrap_or_default(),
        size: raw.file_length,
        sha1,
        sha512: None,
        primary: raw.release_type == 1,
    }
}

fn version_from_file(raw: FileResponse) -> PackVersion {
    let mut game_versions: Vec<String> = Vec::new();
    let mut loaders: Vec<String> = Vec::new();
    for value in &raw.game_versions {
        let lower = value.to_ascii_lowercase();
        if LoaderKind::parse(&lower).is_some() {
            if !loaders.contains(&lower) {
                loaders.push(lower);
            }
        } else if looks_like_mc_version(value) && !game_versions.contains(value) {
            game_versions.push(value.clone());
        }
    }
    PackVersion {
        id: raw.id.to_string(),
        name: raw.display_name.clone(),
        version_number: raw.file_name.clone(),
        version_type: match raw.release_type {
            1 => "release".to_string(),
            2 => "beta".to_string(),
            _ => "alpha".to_string(),
        },
        game_versions,
        loaders,
        date_published: raw.file_date.clone(),
        downloads: raw.download_count,
        files: vec![file_from(raw)],
        dependencies: Vec::new(),
    }
}

/// 版本号 → 可比较的元组（`1.21.10 > 1.21.9`，字符串比较做不到）。
fn version_key(raw: &str) -> Vec<u32> {    raw.split('.')
        .map(|part| {
            let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().unwrap_or(0)
        })
        .collect()
}

/// `1.21.1`、`26.3` 这种；`Fabric`、`Client` 不算版本号。
fn looks_like_mc_version(raw: &str) -> bool {
    let trimmed = raw.trim();
    !trimmed.is_empty()
        && trimmed.starts_with(|c: char| c.is_ascii_digit())
        && trimmed.chars().any(|c| c == '.')
}

// ---------------------------------------------------------------- 响应结构

#[derive(Debug, Default, serde::Deserialize)]
struct DataResponse<T> {
    data: T,
}

#[derive(Debug, Default, serde::Deserialize)]
struct ListResponse<T> {
    #[serde(default)]
    data: Vec<T>,
    #[serde(default)]
    pagination: Pagination,
}

#[derive(Debug, Default, serde::Deserialize)]
struct Pagination {
    #[serde(default)]
    index: u32,
    #[serde(default, rename = "pageSize")]
    page_size: u32,
    #[serde(default, rename = "totalCount")]
    total_count: u64,
}

#[derive(Debug, Default, serde::Deserialize)]
struct ModResponse {
    #[serde(default)]
    id: u64,
    #[serde(default)]
    name: String,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    description: String,
    #[serde(default, rename = "downloadCount")]
    download_count: u64,
    #[serde(default, rename = "dateCreated")]
    date_created: String,
    #[serde(default, rename = "dateModified")]
    date_modified: String,
    #[serde(default)]
    categories: Vec<CategoryResponse>,
    #[serde(default)]
    authors: Vec<AuthorResponse>,
    #[serde(default)]
    logo: Option<LogoResponse>,
    #[serde(default)]
    links: LinksResponse,
    #[serde(default, rename = "latestFiles")]
    latest_files: Vec<FileResponse>,
    #[serde(default, rename = "latestFilesIndexes")]
    latest_files_indexes: Vec<FileIndexResponse>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct CategoryResponse {
    #[serde(default)]
    name: String,
}

#[derive(Debug, Default, serde::Deserialize)]
struct AuthorResponse {
    #[serde(default)]
    name: String,
}

#[derive(Debug, Default, serde::Deserialize)]
struct LogoResponse {
    #[serde(default, rename = "thumbnailUrl")]
    thumbnail_url: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct LinksResponse {
    #[serde(default, rename = "websiteUrl")]
    website_url: Option<String>,
    #[serde(default, rename = "wikiUrl")]
    wiki_url: Option<String>,
    #[serde(default, rename = "issuesUrl")]
    issues_url: Option<String>,
    #[serde(default, rename = "sourceUrl")]
    source_url: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct FileIndexResponse {
    #[serde(default, rename = "gameVersion")]
    game_version: Option<String>,
    #[serde(default, rename = "modLoader")]
    mod_loader: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct FileResponse {
    #[serde(default)]
    id: u64,
    #[serde(default, rename = "displayName")]
    display_name: String,
    #[serde(default, rename = "fileName")]
    file_name: String,
    #[serde(default, rename = "releaseType")]
    release_type: u32,
    #[serde(default)]
    hashes: Vec<HashResponse>,
    #[serde(default, rename = "fileDate")]
    file_date: String,
    #[serde(default, rename = "fileLength")]
    file_length: u64,
    #[serde(default, rename = "downloadCount")]
    download_count: u64,
    #[serde(default, rename = "downloadUrl")]
    download_url: Option<String>,
    #[serde(default, rename = "gameVersions")]
    game_versions: Vec<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct HashResponse {
    #[serde(default)]
    value: String,
    #[serde(default)]
    algo: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query() -> PackSearchQuery {
        PackSearchQuery::new("better mc")
    }

    #[test]
    fn missing_key_gets_an_actionable_message() {
        let err = require_key(None).unwrap_err().to_string();
        assert!(err.contains("API key"), "{err}");
        assert!(err.contains(CURSEFORGE_KEY_URL), "要给申请地址：{err}");
        assert!(err.contains("Modrinth"), "要给替代方案：{err}");

        // 全是空白也算没填
        assert!(require_key(Some("   ")).is_err());
        assert_eq!(require_key(Some(" abc ")).unwrap(), "abc");
    }

    #[test]
    fn search_without_key_fails_before_touching_the_network() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let client = HttpClient::new().unwrap();
        let err = rt
            .block_on(search(&client, &query()))
            .unwrap_err()
            .to_string();
        assert!(err.contains("API key"), "实际是：{err}");
    }

    #[test]
    fn search_url_uses_modpack_filters() {
        let url = search_url(&query());
        assert!(url.starts_with("https://api.curseforge.com/v1/mods/search?"), "{url}");
        assert!(url.contains("gameId=432"), "{url}");
        assert!(url.contains("classId=4471"), "整合包的 classId：{url}");
        assert!(url.contains("searchFilter=better+mc") || url.contains("searchFilter=better%20mc"), "{url}");
        assert!(url.contains("sortOrder=desc"), "{url}");
    }

    #[test]
    fn search_url_adds_version_and_loader_filters() {
        let mut q = query();
        q.mc_version = Some("1.21.1".into());
        q.loader = Some(LoaderKind::NeoForge);
        let url = search_url(&q);
        assert!(url.contains("gameVersion=1.21.1"), "{url}");
        assert!(url.contains("modLoaderType=6"), "NeoForge 是 6：{url}");
    }

    #[test]
    fn loader_types_follow_the_documented_numbers() {
        assert_eq!(mod_loader_type(LoaderKind::Forge), "1");
        assert_eq!(mod_loader_type(LoaderKind::Fabric), "4");
        assert_eq!(mod_loader_type(LoaderKind::Quilt), "5");
        assert_eq!(mod_loader_type(LoaderKind::NeoForge), "6");
    }

    #[test]
    fn file_without_download_url_is_not_downloadable() {
        let raw: FileResponse = serde_json::from_str(
            r#"{"id": 7, "fileName": "x.jar", "fileLength": 0, "downloadUrl": null,
                "hashes": [{"value":"deadbeef","algo":1},{"value":"cafe","algo":2}]}"#,
        )
        .unwrap();
        let file = file_from(raw);
        assert!(!file.downloadable(), "downloadUrl 为空时不能假装能下载");
        assert_eq!(file.sha1.as_deref(), Some("deadbeef"), "只认 algo 1（sha1）");
    }

    #[test]
    fn file_version_splits_mc_versions_from_loaders() {
        let raw: FileResponse = serde_json::from_str(
            r#"{"id": 9, "displayName": "1.0", "fileName": "pack.zip", "releaseType": 2,
                "fileLength": 100, "downloadUrl": "https://edge.example/pack.zip",
                "gameVersions": ["1.21.1", "1.20.1", "Fabric", "Client", "Server"],
                "hashes": []}"#,
        )
        .unwrap();
        let version = version_from_file(raw);
        assert_eq!(version.game_versions, ["1.21.1", "1.20.1"]);
        assert_eq!(version.loaders, ["fabric"]);
        assert_eq!(version.version_type, "beta");
        assert_eq!(version.id, "9");
        assert!(version.archive_file().is_some());
    }

    #[test]
    fn non_numeric_project_id_is_rejected() {
        let err = numeric_id("fabulously-optimized").unwrap_err().to_string();
        assert!(err.contains("数字"), "实际是：{err}");
        assert_eq!(numeric_id(" 42 ").unwrap(), 42);
    }

    #[test]
    fn looks_like_mc_version_filters_labels() {
        assert!(looks_like_mc_version("1.21.1"));
        assert!(looks_like_mc_version("26.3"));
        assert!(!looks_like_mc_version("Fabric"));
        assert!(!looks_like_mc_version("Client"));
        assert!(!looks_like_mc_version("1"));
        assert!(!looks_like_mc_version(""));
    }

    #[test]
    fn search_response_parses_with_pagination() {
        let raw: ListResponse<ModResponse> = serde_json::from_str(
            r#"{
                "data": [{
                    "id": 123, "name": "Better MC", "slug": "better-mc",
                    "summary": "A pack", "downloadCount": 999,
                    "dateModified": "2026-01-01T00:00:00Z",
                    "categories": [{ "name": "Adventure" }],
                    "authors": [{ "name": "Someone" }],
                    "logo": { "thumbnailUrl": "https://e/x.png", "url": null },
                    "latestFiles": [{ "id": 5, "fileName": "p.zip", "downloadUrl": "https://e/p.zip",
                                      "fileLength": 10, "hashes": [] }]
                }],
                "pagination": { "index": 0, "pageSize": 20, "totalCount": 41 }
            }"#,
        )
        .unwrap();
        assert_eq!(raw.pagination.total_count, 41);
        assert_eq!(raw.data.len(), 1);
        assert_eq!(raw.data[0].authors[0].name, "Someone");
    }

    /// 真实网络（需要 key）。手动跑：
    /// `MCDL_CF_KEY=xxx cargo test -p mcdl-core pack::curseforge -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "需要真实网络 + 自己申请的 CurseForge API key"]
    async fn search_over_network_with_real_key() {
        let Ok(key) = std::env::var("MCDL_CF_KEY") else {
            println!("没有设置 MCDL_CF_KEY，跳过");
            return;
        };
        let client = HttpClient::new().unwrap();
        let mut q = PackSearchQuery::new("better mc");
        q.api_key = Some(key);
        q.limit = 3;
        let result = search(&client, &q).await.unwrap();
        println!("total = {}，本页 {} 条", result.total, result.hits.len());
        for hit in &result.hits {
            println!("- {}（id {}）下载 {}", hit.title, hit.id, hit.downloads);
        }
        assert!(!result.hits.is_empty());
    }
}
