//! Tauri 命令层。
//!
//! 这一层只做两件事：把前端参数原样交给 `mcdl_core`，把引擎抛出的
//! [`Progress`] 通过 IPC channel 推回前端。任何下载、校验、解压逻辑都不写在这里，
//! 否则 CLI 和 GUI 就会各有一份实现。

use std::path::PathBuf;
use std::sync::Mutex;

use mcdl_core::{
    java, loader, meta, Error, InstallReport, InstallRequest, JavaRuntime, LoaderKind, Progress,
    ProgressSink, Source, VersionManifest,
};
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::Manager;
use tauri_plugin_opener::OpenerExt;

/// 一次安装向外抛的事件。
///
/// 引擎的 [`Progress`] 原样嵌在 `progress` 里，不改字段：契约在 `types.rs`，
/// 这一层只加「结束了没有」这几种收尾状态。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InstallEvent {
    Progress { progress: Progress },
    Done { report: InstallReport },
    Failed { message: String },
    Cancelled,
}

/// 把引擎的进度塞进 channel 的接收端。
struct ChannelSink {
    channel: Channel<InstallEvent>,
}

impl ProgressSink for ChannelSink {
    fn emit(&self, progress: Progress) {
        // 前端关掉窗口时 send 会失败，这里不当作安装失败处理。
        let _ = self.channel.send(InstallEvent::Progress { progress });
    }
}

#[derive(Default)]
struct AppState {
    /// 正在跑的安装任务。取消 = abort 这个句柄，future 被 drop 掉。
    ///
    /// 引擎没有取消令牌，但所有下载都在 await 点上，drop 掉 future 就是最干净的取消：
    /// 已经落盘的文件留着，下次按 SHA1 直接复用。
    running: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

/// 拉版本清单。清单永远走官方源，和用户选的下载源无关。
#[tauri::command]
async fn list_versions() -> Result<VersionManifest, String> {
    let client = mcdl_core::net::HttpClient::new().map_err(|e| e.to_string())?;
    meta::fetch_manifest(&client, Source::Official)
        .await
        .map_err(|e| e.to_string())
}

/// 开始一次安装。立刻返回，进度和结果都走 `on_event`。
#[tauri::command]
fn install(
    app: tauri::AppHandle,
    req: InstallRequest,
    on_event: Channel<InstallEvent>,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut slot = state.running.lock().map_err(|e| e.to_string())?;
    if slot.as_ref().is_some_and(|h| !h.inner().is_finished()) {
        return Err("已经有一个安装任务在跑".to_string());
    }

    let sink = ChannelSink {
        channel: on_event.clone(),
    };
    let worker = app.clone();
    let handle = tauri::async_runtime::spawn(async move {
        let result = match mcdl_core::Installer::new() {
            Ok(installer) => installer.run(&req, &sink).await,
            Err(e) => Err(e),
        };
        let _ = match result {
            Ok(report) => on_event.send(InstallEvent::Done { report }),
            // 引擎自己判定被取消（还没走到这一步，先接住）
            Err(Error::Cancelled) => on_event.send(InstallEvent::Cancelled),
            Err(e) => on_event.send(InstallEvent::Failed {
                message: e.to_string(),
            }),
        };
        // 任务结束后把槽位空出来，否则这个进程里再也装不了第二次。
        if let Ok(mut slot) = worker.state::<AppState>().running.lock() {
            *slot = None;
        }
    });

    *slot = Some(handle);
    Ok(())
}

/// 取消正在跑的安装。返回是否真的取消了。
///
/// abort 之后那条任务不会再发任何事件，前端把 `true` 当作「已取消」即可。
#[tauri::command]
fn cancel(app: tauri::AppHandle) -> Result<bool, String> {
    let state = app.state::<AppState>();
    let mut slot = state.running.lock().map_err(|e| e.to_string())?;
    match slot.take() {
        Some(handle) => {
            handle.abort();
            Ok(true)
        }
        None => Ok(false),
    }
}

/// 扫本机的 Java 运行时。同步命令，扫目录很快。
#[tauri::command]
fn detect_java() -> Vec<JavaRuntime> {
    java::detect()
}

/// 某个加载器对某个 MC 版本可用的版本列表。
///
/// 引擎侧最多返回 20 条、从新到旧，并且有 5 分钟缓存；
/// 单次查询超过 10 秒会失败，前端要能降级成「用最新版」。
#[tauri::command]
async fn list_loaders(kind: LoaderKind, mc_version: String) -> Result<Vec<String>, String> {
    let client = mcdl_core::net::HttpClient::new().map_err(|e| e.to_string())?;
    loader::available_loader_versions(&client, kind, &mc_version)
        .await
        .map_err(|e| e.to_string())
}

/// 游戏目录下已经落盘的 Java 运行时：`runtime/jre-<major>/bin/java.exe`。
///
/// 这里只列目录，不做任何下载或校验——那是引擎的事。
#[tauri::command]
fn list_runtimes(game_dir: String) -> Vec<String> {
    let root = PathBuf::from(game_dir).join("runtime");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let exe = if cfg!(windows) { "java.exe" } else { "java" };
    let mut found: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("jre-"))
        .map(|e| e.path().join("bin").join(exe))
        .filter(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    found.sort();
    found
}

/// 系统默认的 `.minecraft` 目录。前端拿它填表单的初值。
#[tauri::command]
fn default_game_dir() -> String {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("HOME").map(PathBuf::from)
    };
    base.unwrap_or_default()
        .join(".minecraft")
        .to_string_lossy()
        .into_owned()
}

/// 在系统文件管理器里打开游戏目录。
#[tauri::command]
fn open_game_dir(app: tauri::AppHandle, path: String) -> Result<(), String> {
    app.opener()
        .open_path(path, None::<&str>)
        .map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            list_versions,
            list_loaders,
            install,
            cancel,
            detect_java,
            list_runtimes,
            default_game_dir,
            open_game_dir,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
