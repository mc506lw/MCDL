//! 「扫描并导入其它启动器实例」的 Tauri 命令层。
//!
//! 和 `lib.rs` 里的 `install` 一样，这一层只做三件事：
//! 把前端参数交给 `mcdl_core::import`、把引擎抛的进度塞进 IPC channel、管住
//! 「同时只能跑一个导入任务」和取消。
//!
//! **注册方式（由 `lib.rs` 的负责人做）：**
//!
//! ```ignore
//! invoke_handler(tauri::generate_handler![
//!     // …原有命令…
//!     cmd_import::import_scan,
//!     cmd_import::import_measure,
//!     cmd_import::import_start,
//!     cmd_import::import_cancel,
//! ])
//! ```
//!
//! 不需要 `.manage(...)`：任务槽位是这个模块自己的静态量，注册命令就够用。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use mcdl_core::import::{
    self, CancelToken, DirSize, ImportItem, ImportReport, ImportRequest, ScanResult,
};
use serde::Serialize;
use tauri::ipc::Channel;

/// 一次导入向外抛的事件。
///
/// 引擎的 [`import::ImportProgress`] 原样嵌在 `progress` 里，不改字段；
/// 这一层只加「结束了没有」这几种收尾状态。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImportEvent {
    Progress { progress: import::ImportProgress },
    Done { report: ImportReport },
    Failed { message: String },
    Cancelled,
}

/// 把引擎的进度塞进 channel 的接收端。
struct ChannelSink {
    channel: Channel<ImportEvent>,
}

impl import::ImportSink for ChannelSink {
    fn emit(&self, progress: import::ImportProgress) {
        // 前端关掉窗口时 send 会失败，不当作导入失败。
        let _ = self.channel.send(ImportEvent::Progress { progress });
    }
}

/// 正在跑的导入任务。
struct Running {
    /// 引擎在每个文件之间检查这个标志
    cancel: CancelToken,
    /// 任务结束后用来清空槽位，避免把「刚结束」误判成「还在跑」
    finished: Arc<AtomicBool>,
}

/// 任务槽位。放静态量里，`lib.rs` 就不用为这个功能改 `AppState`。
fn slot() -> &'static Mutex<Option<Running>> {
    static SLOT: OnceLock<Mutex<Option<Running>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// 扫本机其它启动器。同步命令：只读配置和目录名，几毫秒就回来。
///
/// 永远不失败：找不到就是空列表，界面上写「没找到」而不是弹错误。
#[tauri::command]
pub fn import_scan() -> ScanResult {
    import::scan_default()
}

/// 懒加载体积：这一项「导入会搬多少」。
///
/// 只数各版本目录 + mods/config/saves，不数 libraries/assets——那些是共享的，
/// 导入也不搬。扫描阶段故意不算体积（大目录会拖慢首屏），界面拿到列表之后再补。
///
/// 用 `async` + `spawn_blocking`：递归 stat 一个几万文件的 mods 目录要几百毫秒，
/// 同步命令会在主线程上跑，界面会卡一下。
#[tauri::command]
pub async fn import_measure(item: ImportItem) -> std::result::Result<DirSize, String> {
    tauri::async_runtime::spawn_blocking(move || import::measure_item(&item))
        .await
        .map_err(|e| e.to_string())
}

/// 开始一次导入。立刻返回，进度和结果都走 `on_event`。
#[tauri::command]
pub fn import_start(
    req: ImportRequest,
    on_event: Channel<ImportEvent>,
) -> std::result::Result<(), String> {
    let mut guard = slot().lock().map_err(|e| e.to_string())?;
    if let Some(running) = guard.as_ref() {
        if !running.finished.load(Ordering::Relaxed) {
            return Err("已经有一个导入任务在跑".to_string());
        }
    }

    let cancel = import::cancel_token();
    let finished = Arc::new(AtomicBool::new(false));
    *guard = Some(Running {
        cancel: cancel.clone(),
        finished: finished.clone(),
    });
    drop(guard);

    let sink = ChannelSink {
        channel: on_event.clone(),
    };
    // 导入是同步文件 IO：放到 blocking 线程池，别占住 async 运行时。
    tauri::async_runtime::spawn_blocking(move || {
        let result = import::import(&req, &sink, &cancel);
        let _ = match result {
            Ok(report) => on_event.send(ImportEvent::Done { report }),
            Err(mcdl_core::Error::Cancelled) => on_event.send(ImportEvent::Cancelled),
            Err(e) => on_event.send(ImportEvent::Failed {
                message: e.to_string(),
            }),
        };
        // 收尾：标成结束，让下一次导入能进得来。
        finished.store(true, Ordering::Relaxed);
    });

    Ok(())
}

/// 取消正在跑的导入。返回是否真的取消了。
///
/// 引擎在文件之间检查标志，所以取消不是瞬间的：它会在下一个文件边界停下，
/// 已经复制过去的文件留在目标目录里（下次再导入会跳过）。
#[tauri::command]
pub fn import_cancel() -> std::result::Result<bool, String> {
    let guard = slot().lock().map_err(|e| e.to_string())?;
    match guard.as_ref() {
        Some(running) if !running.finished.load(Ordering::Relaxed) => {
            running.cancel.store(true, Ordering::Relaxed);
            Ok(true)
        }
        _ => Ok(false),
    }
}
