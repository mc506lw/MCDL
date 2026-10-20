//! HTTP 客户端与下载源地址。
//!
//! 官方源与 BMCLAPI 的差别只体现在「资源文件」这一类上：
//! 版本清单、版本 JSON 永远走官方，避免镜像篡改。

use crate::error::Result;
use crate::types::Source;

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
            .build()
            .map_err(|e| crate::Error::Http(e.to_string()))?;
        Ok(Self { inner })
    }

    pub fn raw(&self) -> &reqwest::Client {
        &self.inner
    }

    /// 拉 JSON。失败时把 URL 带进错误里，方便定位是哪个接口挂了。
    pub async fn get_json<T: serde::de::DeserializeOwned>(&self, _url: &str) -> Result<T> {
        todo!("core-net: 实现 JSON 拉取")
    }

    pub async fn get_text(&self, _url: &str) -> Result<String> {
        todo!("core-net: 实现文本拉取")
    }

    pub async fn get_bytes(&self, _url: &str) -> Result<Vec<u8>> {
        todo!("core-net: 实现字节拉取")
    }
}

/// 版本清单地址（永远是官方）。
pub const MANIFEST_URL: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

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
