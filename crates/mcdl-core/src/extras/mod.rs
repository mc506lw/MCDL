//! 装完之后补东西：Fabric API、OptiFine、自定义版本名。
//!
//! 这三项都**不进** [`crate::Installer::run`]：
//!
//! - 装原版 + 加载器是一次闭环，任何附加物失败都不该把已经装好的版本回滚掉；
//! - 附加项要联网查 Modrinth / optifine.net，慢且可能挂，不该拖住安装主流程；
//! - 其中 OptiFine 只能做到「一部分组合真能自动装」，把失败原因如实交给界面
//!   比在安装主流程里静默吞掉要好。
//!
//! [`apply`] 固定了执行顺序：先改名（改完版本目录名才是最终结果），再补
//! Fabric API 与 OptiFine（它们只往模组目录里放文件）。每一项互相独立，
//! 一项失败只影响它自己，报告里逐条写清状态与原因。
//!
//! 模组目录不是写死的 `<gameDir>/mods/`：由版本隔离开关决定，算法只有
//! [`crate::isolate::mods_dir`] 一份。改名排在补模组之前，正是因为隔离后的
//! 落点要用改名后的版本目录名。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::net::HttpClient;
use crate::paths::GamePaths;
use crate::types::LoaderKind;

pub mod fabric_api;
pub mod optifine;
pub mod rename;

pub use fabric_api::{FabricApiFile, FabricApiOutcome};
pub use optifine::{OptifineBuild, OptifineMode, OptifineOutcome, OptifinePlan};
pub use rename::{NameCheck, RenameOutcome};

/// 一个附加项的执行结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtraStatus {
    /// 真的做完了
    Done,
    /// 按规则跳过（没有可做的事、已经存在、不适用于这个组合）
    Skipped,
    /// 试过但失败了，`message` 里是能读懂的原因
    Failed,
    /// 需要用户手动完成一步（例如运行官方安装器）
    Manual,
}

impl ExtraStatus {
    pub fn label(self) -> &'static str {
        match self {
            ExtraStatus::Done => "已完成",
            ExtraStatus::Skipped => "已跳过",
            ExtraStatus::Failed => "失败",
            ExtraStatus::Manual => "需要手动完成",
        }
    }
}

/// 给界面看的一条结论：哪一项、什么状态、一句话说明。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtraNotice {
    /// `fabric_api` / `optifine` / `rename`
    pub item: String,
    pub status: ExtraStatus,
    pub message: String,
}

impl ExtraNotice {
    pub fn new(item: impl Into<String>, status: ExtraStatus, message: impl Into<String>) -> Self {
        Self {
            item: item.into(),
            status,
            message: message.into(),
        }
    }
}

/// 一次「补齐附加项」的请求。
///
/// 字段都是可选开关：全部关掉时 [`apply`] 什么都不做，报告里也没有 notices。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtrasRequest {
    /// `.minecraft` 所在目录
    pub game_dir: PathBuf,
    pub mc_version: String,
    /// 这次装的加载器（原版是 None）
    #[serde(default)]
    pub loader: Option<LoaderKind>,
    /// 刚装完的版本目录名（`versions/` 下的名字）。改名要用它，
    /// 由调用方从磁盘扫描拿到——引擎不替调用方猜。
    #[serde(default)]
    pub installed_version: Option<String>,
    /// 补 Fabric API / QFAPI（默认由界面勾选）
    #[serde(default)]
    pub fabric_api: bool,
    /// 尝试自动装 OptiFine（只有 Forge + 1.13+ 会真的自动装）
    #[serde(default)]
    pub optifine: bool,
    /// 自定义版本名；空或 None 表示不改
    #[serde(default)]
    pub rename_to: Option<String>,
    /// 版本隔离：模组写进 `versions/<版本id>/mods/`（开，默认）还是根 `mods/`（关）。
    ///
    /// 版本 id 用的是**改名之后**的目录名（[`apply`] 里先改名，再补模组）。
    #[serde(default = "crate::isolate::default_on")]
    pub isolated: bool,
}

/// 一次「补齐附加项」的结果。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtrasReport {
    /// 全部做完之后的版本目录名（改过名就是新名字）
    pub version_id: String,
    pub notices: Vec<ExtraNotice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fabric_api: Option<FabricApiOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optifine: Option<OptifineOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rename: Option<RenameOutcome>,
}

/// 界面在「开始安装」之前问一次：这两个附加项各能拿到什么。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtrasPlan {
    /// Fabric API / QFAPI 的最新文件；查不到就是 None
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fabric_api: Option<FabricApiFile>,
    /// 查询失败的原因（查不到不等于失败，界面要分开显示）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fabric_api_error: Option<String>,
    pub optifine: OptifinePlan,
}

/// 问一次「这个组合能补什么」。两个网络请求各带超时，慢也不会把界面挂住。
pub async fn plan(client: &HttpClient, mc_version: &str, loader: Option<LoaderKind>) -> ExtrasPlan {
    let mut plan = ExtrasPlan {
        optifine: optifine::plan(client, mc_version, loader).await,
        ..Default::default()
    };

    if let Some(kind) = loader.filter(|k| fabric_api::project_for(*k).is_some()) {
        match fabric_api::resolve(client, kind, mc_version).await {
            Ok(file) => plan.fabric_api = file,
            Err(e) => plan.fabric_api_error = Some(e.to_string()),
        }
    }

    plan
}

/// 依次补齐所有勾选的附加项。任何一项失败都不会中断后面的项。
pub async fn apply(client: &HttpClient, req: &ExtrasRequest) -> ExtrasReport {
    let paths = GamePaths::new(&req.game_dir);
    let mut report = ExtrasReport {
        version_id: req.installed_version.clone().unwrap_or_default(),
        ..Default::default()
    };

    // 1. 改名。必须排在 mods/ 那两步之前：改完才是最终版本目录名。
    if let Some(target) = req
        .rename_to
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let from = req
            .installed_version
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .to_string();
        if from.is_empty() {
            report.notices.push(ExtraNotice::new(
                "rename",
                ExtraStatus::Skipped,
                "不知道这次装出来的版本目录名，改名已跳过",
            ));
        } else if from == target {
            report.notices.push(ExtraNotice::new(
                "rename",
                ExtraStatus::Skipped,
                format!("版本名没有变化（{from}）"),
            ));
            report.rename = Some(RenameOutcome::unchanged(&paths, &from));
        } else {
            match rename::rename(&paths, &from, target) {
                Ok(outcome) => {
                    report.version_id = outcome.to.clone();
                    report.notices.push(ExtraNotice::new(
                        "rename",
                        ExtraStatus::Done,
                        format!("版本已改名为 {}", outcome.to),
                    ));
                    report.rename = Some(outcome);
                }
                Err(e) => report.notices.push(ExtraNotice::new(
                    "rename",
                    ExtraStatus::Failed,
                    format!("改名失败：{e}"),
                )),
            }
        }
    }

    // 2. Fabric API / QFAPI
    //
    // 落点跟着版本隔离走，版本 id 用改名之后的最终目录名（`report.version_id`）。
    if req.fabric_api {
        let outcome = fabric_api::install(
            client,
            &paths,
            req.loader,
            &req.mc_version,
            &report.version_id,
            req.isolated,
        )
        .await;
        report.notices.push(ExtraNotice::new(
            "fabric_api",
            outcome.status,
            outcome.message.clone(),
        ));
        report.fabric_api = Some(outcome);
    }

    // 3. OptiFine
    if req.optifine {
        let outcome = optifine::install(
            client,
            &paths,
            req.loader,
            &req.mc_version,
            &report.version_id,
            req.isolated,
        )
        .await;
        report.notices.push(ExtraNotice::new(
            "optifine",
            outcome.status,
            outcome.message.clone(),
        ));
        report.optifine = Some(outcome);
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_game(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("mcdl-extras-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(dir.join("versions")).unwrap();
        dir
    }

    /// 全部开关都关掉时不该做任何事，也不该产生 notices。
    #[tokio::test]
    async fn apply_with_everything_off_is_noop() {
        let dir = temp_game("noop");
        let client = HttpClient::new().unwrap();
        let req = ExtrasRequest {
            game_dir: dir.clone(),
            mc_version: "1.21.1".into(),
            loader: None,
            installed_version: None,
            fabric_api: false,
            optifine: false,
            rename_to: None,
            isolated: true,
        };
        let report = apply(&client, &req).await;
        assert!(report.notices.is_empty());
        assert!(report.fabric_api.is_none());
        assert!(report.optifine.is_none());
        assert!(report.rename.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 改名排在 Fabric API 之前：报告里的 version_id 是改完后的名字，
    /// 而且改名失败不会挡住后面的项（这里 fabric_api 用「非 Fabric 加载器」跳过，不联网）。
    #[tokio::test]
    async fn apply_renames_before_other_items() {
        let dir = temp_game("order");
        let versions = dir.join("versions").join("1.21.1");
        std::fs::create_dir_all(&versions).unwrap();
        std::fs::write(
            versions.join("1.21.1.json"),
            br#"{"id":"1.21.1","type":"release"}"#,
        )
        .unwrap();

        let client = HttpClient::new().unwrap();
        let req = ExtrasRequest {
            game_dir: dir.clone(),
            mc_version: "1.21.1".into(),
            loader: None,
            installed_version: Some("1.21.1".into()),
            fabric_api: true, // 原版：Fabric API 那一步会跳过，不会联网
            optifine: false,
            rename_to: Some("我的世界".into()),
            isolated: true,
        };
        let report = apply(&client, &req).await;
        assert_eq!(report.version_id, "我的世界");
        assert!(dir.join("versions").join("我的世界").is_dir());
        assert!(!dir.join("versions").join("1.21.1").exists());
        assert_eq!(report.notices.len(), 2, "改名 + Fabric API 跳过各一条");
        assert_eq!(report.notices[0].item, "rename");
        assert_eq!(report.notices[0].status, ExtraStatus::Done);
        assert_eq!(report.notices[1].item, "fabric_api");
        assert_eq!(report.notices[1].status, ExtraStatus::Skipped);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 协议：界面没传 `isolated` 时默认**开**（和设置页的默认值一致）。
    #[test]
    fn request_defaults_to_isolation_on() {
        let req: ExtrasRequest = serde_json::from_str(
            r#"{"game_dir":"C:/mc","mc_version":"1.21.1","fabric_api":false,"optifine":false}"#,
        )
        .unwrap();
        assert!(req.isolated, "没传字段时必须是隔离的");

        let req: ExtrasRequest = serde_json::from_str(
            r#"{"game_dir":"C:/mc","mc_version":"1.21.1","fabric_api":false,"optifine":false,"isolated":false}"#,
        )
        .unwrap();
        assert!(!req.isolated, "显式关掉必须尊重");
    }
}
