//! HTTP 客户端与下载源地址。
//!
//! 官方源与 BMCLAPI 的差别只体现在「资源文件」这一类上：
//! 版本清单、版本 JSON 永远走官方，避免镜像篡改。

use std::time::Duration;

use serde::de::DeserializeOwned;

use crate::error::{Error, Result};
use crate::types::Source;

/// 单个请求的整体超时。连接建立另有 15s 的上限（见 [`HttpClient::new`]）。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// 带超时与 UA 的 HTTP 客户端，全流程复用一个。
#[derive(Debug, Clone)]
pub struct HttpClient {
    inner: reqwest::Client,
}

impl HttpClient {
    pub fn new() -> Result<Self> {
        let inner = reqwest::Client::builder()
            .user_agent(crate::user_agent())
            .connect_timeout(std::time::Duration::from_secs(15))
            .timeout(std::time::Duration::from_secs(120))
            // 卡住的连接必须能被发现：单次读超过 30 秒没有字节就判失败重试。
            // 下载大文件时总超时被放宽到 30 分钟，没有这一条的话
            // 一条僵死的 TCP 连接会安静地占住一个 worker，进度永远停在 N/N-1。
            .read_timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| crate::Error::Http(e.to_string()))?;
        Ok(Self { inner })
    }

    pub fn raw(&self) -> &reqwest::Client {
        &self.inner
    }

    /// 拉 JSON。失败时把 URL 带进错误里，方便定位是哪个接口挂了。
    pub async fn get_json<T: DeserializeOwned>(&self, url: &str) -> Result<T> {
        let body = self.get_bytes(url).await?;
        let value = serde_json::from_slice(&body).map_err(|e| {
            // 镜像返回 HTML 错误页是这里最常见的事故，所以把 URL 和响应长度一起记下来。
            tracing::error!(url, bytes = body.len(), error = %e, "响应不是合法 JSON");
            Error::other(format!(
                "{url} 返回的不是合法 JSON（{} 字节）：{e}",
                body.len()
            ))
        })?;
        Ok(value)
    }

    pub async fn get_text(&self, url: &str) -> Result<String> {
        let resp = self.send(url).await?;
        resp.text()
            .await
            .map_err(|e| Error::Http(format!("读取 {url} 的响应文本失败：{e}")))
    }

    pub async fn get_bytes(&self, url: &str) -> Result<Vec<u8>> {
        let resp = self.send(url).await?;
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| Error::Http(format!("读取 {url} 的响应体失败：{e}")))?;
        Ok(bytes.to_vec())
    }

    /// 发一个 GET，并把网络错误、非 2xx 都翻译成带 URL 的错误。
    async fn send(&self, url: &str) -> Result<reqwest::Response> {
        tracing::trace!(url, "GET");
        let resp = self
            .inner
            .get(url)
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(|e| Error::Http(format!("GET {url} 失败：{e}")))?;

        let status = resp.status();
        if !status.is_success() {
            // 不读 body：元数据接口出错时返回的通常是 HTML，读了也用不上。
            return Err(Error::Http(format!("GET {url} 返回 HTTP {status}")));
        }
        Ok(resp)
    }
}

/// 版本清单地址（永远是官方）。
pub const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

/// 依赖库文件的地址前缀。
pub fn library_base(source: Source) -> &'static str {
    match source {
        Source::Official => "https://libraries.minecraft.net",
        Source::Bmclapi => "https://bmclapi2.bangbang93.com/maven",
    }
}

/// 资源文件（assets）的地址前缀。
pub fn asset_base(source: Source) -> &'static str {
    match source {
        Source::Official => "https://resources.download.minecraft.net",
        Source::Bmclapi => "https://bmclapi2.bangbang93.com/assets",
    }
}

/// 拼出某个库文件的完整地址。
pub fn library_url(source: Source, path: &str) -> String {
    format!("{}/{}", library_base(source), path.trim_start_matches('/'))
}

/// 拼出某个资源文件的完整地址。对象按 hash 前两位分目录。
pub fn asset_url(source: Source, hash: &str) -> String {
    let head = &hash[..2.min(hash.len())];
    format!("{}/{head}/{hash}", asset_base(source))
}

/// 版本 JSON 的地址就是清单里给的那条 url，这里只做透传。
pub fn version_url(url: &str) -> &str {
    url
}

/// BMCLAPI 的根地址。镜像只做转发，界面上必须标明是第三方源。
pub const BMCLAPI_BASE: &str = "https://bmclapi2.bangbang93.com";

/// 游戏本体 jar 的地址。BMCLAPI 的版本目录形态与官方 CDN 不同。
///
/// - 官方：用清单/版本 JSON 里给的那条 url；
/// - BMCLAPI：`/version/<id>/client`，不带 `.jar` 后缀。
pub fn client_url(source: Source, version_id: &str, official: &str) -> String {
    match source {
        Source::Official => official.to_string(),
        Source::Bmclapi => format!("{BMCLAPI_BASE}/version/{version_id}/client"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_url_switches_source() {
        let path = "com/mojang/brigadier/1.3.10/brigadier-1.3.10.jar";
        assert_eq!(
            library_url(Source::Official, path),
            "https://libraries.minecraft.net/com/mojang/brigadier/1.3.10/brigadier-1.3.10.jar"
        );
        assert_eq!(
            library_url(Source::Bmclapi, path),
            "https://bmclapi2.bangbang93.com/maven/com/mojang/brigadier/1.3.10/brigadier-1.3.10.jar"
        );
        // 前导斜杠不该拼出双斜杠
        assert_eq!(
            library_url(Source::Official, "/a/b.jar"),
            "https://libraries.minecraft.net/a/b.jar"
        );
    }

    #[test]
    fn asset_url_uses_hash_prefix() {
        let hash = "abcdef0123456789abcdef0123456789abcdef01";
        assert_eq!(
            asset_url(Source::Official, hash),
            format!("https://resources.download.minecraft.net/ab/{hash}")
        );
        assert_eq!(
            asset_url(Source::Bmclapi, hash),
            format!("https://bmclapi2.bangbang93.com/assets/ab/{hash}")
        );
    }

    #[test]
    fn asset_url_tolerates_short_hash() {
        // 理论上不会出现，但不能在这里 panic。
        assert_eq!(
            asset_url(Source::Official, "ab"),
            "https://resources.download.minecraft.net/ab/ab"
        );
    }

    #[test]
    fn client_url_follows_source() {
        let official = "https://piston-data.mojang.com/v1/objects/30c73b1c/client.jar";
        assert_eq!(
            client_url(Source::Official, "1.21.1", official),
            official,
            "官方源必须原样透传，不能改写"
        );
        assert_eq!(
            client_url(Source::Bmclapi, "1.21.1", official),
            "https://bmclapi2.bangbang93.com/version/1.21.1/client"
        );
        // 加载器版本 id 里带点/横线也要原样拼进去
        assert_eq!(
            client_url(Source::Bmclapi, "1.21.1-forge-51.0.33", official),
            "https://bmclapi2.bangbang93.com/version/1.21.1-forge-51.0.33/client"
        );
    }

    /// 错误信息里必须带 URL：这是用户唯一能自助排查的线索。
    #[tokio::test]
    async fn http_error_carries_url() {
        let client = HttpClient::new().unwrap();
        let url = "mcdl-nope://127.0.0.1/manifest.json";
        let err = client.get_json::<serde_json::Value>(url).await.unwrap_err();
        let text = err.to_string();
        assert!(text.contains(url), "错误里应该带 URL，实际是：{text}");
    }

    /// 真实网络：验证 gzip 解压 + 官方清单能解析。默认忽略，手动跑：
    /// `cargo test -p mcdl-core -- --ignored`
    #[tokio::test]
    #[ignore = "需要真实网络"]
    async fn fetch_official_manifest_over_network() {
        let client = HttpClient::new().unwrap();
        let raw: serde_json::Value = client.get_json(MANIFEST_URL).await.unwrap();
        let versions = raw["versions"].as_array().expect("清单里应该有 versions");
        assert!(versions.len() > 100);
        // get_text 走同一条链路，顺便验证 gzip 解出来的还是完整 JSON
        let text = client.get_text(MANIFEST_URL).await.unwrap();
        assert!(text.starts_with('{'), "文本响应应当被 gzip 解压成 JSON");
        assert!(text.contains("\"latest\""));
        assert!(text.len() > 100_000);
    }
}
