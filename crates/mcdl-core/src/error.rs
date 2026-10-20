use std::path::PathBuf;

/// 引擎统一错误类型。
///
/// 对用户展示的文案直接写在 `#[error]` 里：CLI 打日志、GUI 弹提示都用这一份，
/// 不要在调用方再拼一遍。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("网络请求失败：{0}")]
    Http(String),

    #[error("解析元数据失败：{0}")]
    Json(#[from] serde_json::Error),

    #[error("读写文件失败（{path}）：{source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("版本清单里没有 {0}")]
    VersionNotFound(String),

    #[error("下载校验失败，文件内容与官方不一致：{path}")]
    HashMismatch { path: PathBuf },

    #[error("这个版本需要 Java {0}，但本机没有找到，自动获取也失败了")]
    JavaMissing(u32),

    #[error("安装加载器失败：{0}")]
    Loader(String),

    #[error("用户取消了操作")]
    Cancelled,

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }

    pub fn other(msg: impl Into<String>) -> Self {
        Error::Other(msg.into())
    }
}

/// 给 `?` 用：把 IO 错误带上路径。
pub trait IoContext<T> {
    fn at(self, path: impl Into<PathBuf>) -> Result<T>;
}

impl<T> IoContext<T> for std::result::Result<T, std::io::Error> {
    fn at(self, path: impl Into<PathBuf>) -> Result<T> {
        self.map_err(|e| Error::io(path, e))
    }
}
