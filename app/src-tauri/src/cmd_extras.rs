//! 安装选项相关的命令。占位实现，由 install-extras 替换。

use serde_json::Value;

#[tauri::command]
pub async fn extras_resolve() -> Result<Value, String> {
    Err("尚未实现".into())
}

#[tauri::command]
pub async fn extras_apply() -> Result<Value, String> {
    Err("尚未实现".into())
}

#[tauri::command]
pub async fn extras_validate_name() -> Result<bool, String> {
    Err("尚未实现".into())
}

#[tauri::command]
pub async fn extras_optifine_installer() -> Result<Value, String> {
    Err("尚未实现".into())
}
