//! 整合包命令层（Modrinth / CurseForge）。
//!
//! 和 `lib.rs` 里的命令一样，这一层只做两件事：把前端参数原样交给 `mcdl_core::pack`，
//! 把引擎抛出的 [`PackProgress`] 通过 IPC channel 推回前端。任何下载、解压、格式解析
//! 都不写在这里。
//!
//! ## 为什么不用 `AppState`
//!
//! 整合包安装和「安装」页的安装是两个独立任务，可以同时跑（装一个原版实例、
//! 同时装一个整合包）。所以这里用自己的进程内静态槽位，不碰 `lib.rs` 的
//! `AppState`——注册时只需要把命令加进 `generate_handler!`，不需要 `manage` 任何东西。

use std::sync::{Mutex, OnceLock};

use mcdl_core::net::HttpClient;
use mcdl_core::pack::{
    self, PackInstallReport, PackInstallRequest, PackProgress, PackProgressSink, PackProject,
    PackSearchQuery, PackSearchResult, PackSource, PackVersion,
};
use serde::Serialize;
use tauri::ipc::Channel;

/// 一次整合包安装向外抛的事件。
///
/// 引擎的 [`PackProgress`] 原样嵌在 `progress` 里，不改字段：契约在 `mcdl_core::pack`，
/// 这一层只加「结束了没有」这几种收尾状态（和 `lib.rs` 的 `InstallEvent` 同构）。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PackEvent {
    Progress { progress: PackProgress },
    Done { report: PackInstallReport },
    Failed { message: String },
    Cancelled,
}

/// 把整合包的进度塞进 channel 的接收端。
struct ChannelSink {
    channel: Channel<PackEvent>,
}

impl PackProgressSink for ChannelSink {
    fn emit(&self, progress: PackProgress) {
        // 前端关掉窗口时 send 会失败，不当作安装失败处理。
        let _ = self.channel.send(PackEvent::Progress { progress });
    }
}

type Slot = Mutex<Option<tauri::async_runtime::JoinHandle<()>>>;

/// 正在跑的整合包安装。取消 = abort 这个句柄，future 被 drop 掉。
///
/// 用 `OnceLock` 而不是 `tauri::State`：这个槽位是整合包功能自己的事，
/// 注册命令时不需要额外 `manage`。
fn running() -> &'static Slot {
    static SLOT: OnceLock<Slot> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// 搜整合包。CurseForge 要在 `query.api_key` 里带上用户自己申请的 key。
#[tauri::command]
pub async fn pack_search(query: PackSearchQuery) -> Result<PackSearchResult, String> {
    let client = HttpClient::new().map_err(|e| e.to_string())?;
    pack::search(&client, &query)
        .await
        .map_err(|e| e.to_string())
}

/// 整合包详情：简介、正文、许可证、支持的版本与加载器。
#[tauri::command]
pub async fn pack_project(
    source: PackSource,
    id: String,
    api_key: Option<String>,
) -> Result<PackProject, String> {
    let client = HttpClient::new().map_err(|e| e.to_string())?;
    pack::project(&client, source, &id, api_key.as_deref())
        .await
        .map_err(|e| e.to_string())
}

/// 某个整合包的所有版本（从新到旧），安装前用它让用户挑版本。
///
/// 界面的下拉框最多要 [`VERSION_LIST_LIMIT`] 条：Modrinth 上热门的整合包能有几百个版本，
/// 全部塞进下拉框既没人翻，序列化也白花时间。安装时传的仍然是精确的 version id，
/// 引擎那边照样按 id 找得到（`pack::resolve_version` 用的是完整列表）。
#[tauri::command]
pub async fn pack_versions(
    source: PackSource,
    id: String,
    api_key: Option<String>,
) -> Result<Vec<PackVersion>, String> {
    let client = HttpClient::new().map_err(|e| e.to_string())?;
    let mut list = pack::versions(&client, source, &id, api_key.as_deref())
        .await
        .map_err(|e| e.to_string())?;
    list.truncate(VERSION_LIST_LIMIT);
    Ok(list)
}

/// 版本下拉框最多显示多少条。
const VERSION_LIST_LIMIT: usize = 60;

/// 开始装一个整合包。立刻返回，进度和结果都走 `on_event`。
#[tauri::command]
pub fn pack_install(req: PackInstallRequest, on_event: Channel<PackEvent>) -> Result<(), String> {
    let slot = running();
    let mut guard = slot.lock().map_err(|e| e.to_string())?;
    if guard.as_ref().is_some_and(|h| !h.inner().is_finished()) {
        return Err("已经有一个整合包在装了，等它跑完或者先取消".to_string());
    }

    let sink = ChannelSink {
        channel: on_event.clone(),
    };
    let handle = tauri::async_runtime::spawn(async move {
        let result = pack::install_pack(&req, &sink).await;
        let _ = match result {
            Ok(report) => on_event.send(PackEvent::Done { report }),
            // 引擎自己判定被取消（正常情况下是 abort，走不到这里）
            Err(mcdl_core::Error::Cancelled) => on_event.send(PackEvent::Cancelled),
            Err(e) => on_event.send(PackEvent::Failed {
                message: e.to_string(),
            }),
        };
        // 任务结束后把槽位空出来，否则这个进程里再也装不了第二个整合包。
        if let Ok(mut slot) = running().lock() {
            *slot = None;
        }
    });

    *guard = Some(handle);
    Ok(())
}

/// 取消正在跑的整合包安装。返回是否真的取消了。
///
/// abort 之后那条任务不会再发任何事件，前端把 `true` 当作「已取消」即可。
/// 已经落盘的整合包文件留着，下次按 SHA1 直接复用。
#[tauri::command]
pub fn pack_cancel() -> Result<bool, String> {
    let mut guard = running().lock().map_err(|e| e.to_string())?;
    match guard.take() {
        Some(handle) => {
            handle.abort();
            Ok(true)
        }
        None => Ok(false),
    }
}
