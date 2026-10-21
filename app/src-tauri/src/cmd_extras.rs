//! 「装完之后补东西」的命令层：Fabric API、OptiFine、自定义版本名。
//!
//! 和 `lib.rs` 里的 `install` 一样，这一层只做三件事：把前端参数交给
//! `mcdl_core::extras`、在宿主侧打开 OptiFine 官方安装器、把错误翻成可读文案。
//! 所有下载、校验、改名逻辑都在引擎里，CLI 和 GUI 不会各有一份。
//!
//! **注册方式（由 `lib.rs` 的负责人做）：**
//!
//! ```ignore
//! invoke_handler(tauri::generate_handler![
//!     // …原有命令…
//!     cmd_extras::extras_resolve,
//!     cmd_extras::extras_apply,
//!     cmd_extras::extras_validate_name,
//!     cmd_extras::extras_optifine_installer,
//! ])
//! ```
//!
//! 不需要 `.manage(...)`：这个模块没有共享状态。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use mcdl_core::extras::{
    self, ExtraStatus, ExtrasPlan, ExtrasReport, ExtrasRequest, NameCheck, OptifineMode,
    OptifineOutcome,
};
use mcdl_core::java;
use mcdl_core::net::HttpClient;
use mcdl_core::paths::GamePaths;
use mcdl_core::LoaderKind;

/// 查一次「这个组合能补什么」。
///
/// 两个网络请求（Modrinth / optifine.net）在引擎里各带 30 秒超时；
/// 查不到只是 `None`，界面要能把「查不到」和「查询失败」分开显示。
#[tauri::command]
pub async fn extras_resolve(
    mc_version: String,
    loader: Option<LoaderKind>,
) -> Result<ExtrasPlan, String> {
    let client = HttpClient::new().map_err(|e| e.to_string())?;
    Ok(extras::plan(&client, &mc_version, loader).await)
}

/// 补齐所有勾选的附加项。每一项的结论都在 `report.notices` 里，逐条给用户看。
#[tauri::command]
pub async fn extras_apply(req: ExtrasRequest) -> Result<ExtrasReport, String> {
    let client = HttpClient::new().map_err(|e| e.to_string())?;
    Ok(extras::apply(&client, &req).await)
}

/// 版本名校验。纯函数，同步返回，界面输入时可以直接调。
#[tauri::command]
pub fn extras_validate_name(name: String) -> NameCheck {
    extras::rename::validate_name(&name)
}

/// 下载 OptiFine 官方安装器并打开它。
///
/// 这不是自动安装：OptiFine 的安装器是 Java GUI，没有可用的命令行模式，
/// 打开之后需要用户在窗口里自己点 Install。界面上必须写清楚这一点。
#[tauri::command]
pub async fn extras_optifine_installer(
    game_dir: String,
    mc_version: String,
) -> Result<OptifineOutcome, String> {
    if game_dir.trim().is_empty() {
        return Err("没有游戏目录，无法打开 OptiFine 安装器".to_string());
    }
    let game_dir = PathBuf::from(game_dir.trim());
    let client = HttpClient::new().map_err(|e| e.to_string())?;
    let paths = GamePaths::new(&game_dir);

    // 1. 解析官方构建（optifine.net 的下载页）
    let build = extras::optifine::resolve_build(&client, &mc_version)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| {
            format!(
                "optifine.net 上没有 Minecraft {mc_version} 的 OptiFine 构建（预览版可能还没出）"
            )
        })?;

    // 2. 下载到 <gameDir>/.mcdl/optifine/（官方来源，本工具不托管）
    let downloaded = extras::optifine::download_installer(&client, &paths, &build)
        .await
        .map_err(|e| e.to_string())?;

    // 3. 用本机 Java 打开它
    let java_path = pick_java(&game_dir, &mc_version)?;
    spawn_installer(&java_path, &downloaded.path, &game_dir)?;

    let mut hint = String::from(
        "安装器窗口已经打开，请在里面点 Install。OptiFine 没有可用的命令行安装模式，这一步只能手动点。",
    );
    if !is_default_game_dir(&game_dir) {
        hint.push_str(
            "\n注意：OptiFine 安装器只会写系统的 .minecraft（%APPDATA%\\.minecraft），\
             不会用这里选定的游戏目录。如果两者不是同一个目录，装完请把 versions/ 下新出现的\
             OptiFine 版本目录和 libraries/optifine/ 一起搬过来；或者改用 Forge + mods/ 路线。",
        );
    }

    Ok(OptifineOutcome {
        status: ExtraStatus::Manual,
        message: format!(
            "已下载 {}（{:.1} MB，来自 optifine.net 官方），并已打开安装器",
            build.filename,
            downloaded.bytes as f64 / 1_048_576.0
        ),
        mode: OptifineMode::Manual,
        build: Some(build),
        installer_path: Some(downloaded.path),
        mod_path: None,
        bytes: downloaded.bytes,
        sha1: Some(downloaded.sha1),
        manual_hint: Some(hint),
    })
}

/// 挑一个能跑 OptiFine 安装器的 Java：本机探测到的优先，其次是游戏目录里
/// 自动下载的 `runtime/jre-<major>`。安装器是 Java 8 时代编译的，8 及以上都能跑。
fn pick_java(game_dir: &Path, mc_version: &str) -> Result<PathBuf, String> {
    let detected = java::detect();
    if let Some(rt) = java::pick(&detected, 8) {
        return Ok(rt.path.clone());
    }

    // 安装主流程会把缺的 Java 放到 <gameDir>/runtime/jre-<major>/ 下。
    let runtime = game_dir.join("runtime");
    let exe = if cfg!(windows) { "java.exe" } else { "java" };
    let mut bundled: Vec<(u32, PathBuf)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&runtime) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let Some(major) = name
                .strip_prefix("jre-")
                .and_then(|n| n.parse::<u32>().ok())
            else {
                continue;
            };
            let path = entry.path().join("bin").join(exe);
            if path.is_file() {
                bundled.push((major, path));
            }
        }
    }
    bundled.sort_by_key(|(major, _)| *major);
    if let Some((_, path)) = bundled.iter().find(|(major, _)| *major >= 8) {
        return Ok(path.clone());
    }

    Err(format!(
        "本机没有可用的 Java（需要 Java 8 及以上）来打开 OptiFine 安装器；\
         先装一个 Java，或在「设置」里指定，再重试。Minecraft {mc_version} 的安装不受影响。"
    ))
}

/// 在后台打开安装器，不等待它结束。
///
/// `CREATE_NO_WINDOW` 只是不弹控制台窗口；Swing 的主窗口照样会出现。
fn spawn_installer(java: &Path, installer: &Path, cwd: &Path) -> Result<(), String> {
    let mut cmd = Command::new(java);
    cmd.arg("-jar")
        .arg(installer)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    // 不 wait：安装器是 GUI，用户自己关它。进程句柄丢掉不影响它继续跑。
    cmd.spawn().map_err(|e| {
        format!(
            "启动 OptiFine 安装器失败（Java：{}，jar：{}）：{e}",
            java.display(),
            installer.display()
        )
    })?;
    Ok(())
}

/// 游戏目录是不是系统默认的 `.minecraft`。OptiFine 安装器只认后者。
fn is_default_game_dir(game_dir: &Path) -> bool {
    let Some(default) = default_game_dir() else {
        return false;
    };
    let norm = |p: &Path| {
        std::fs::canonicalize(p)
            .unwrap_or_else(|_| p.to_path_buf())
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_ascii_lowercase()
    };
    norm(game_dir) == norm(&default)
}

fn default_game_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(|base| PathBuf::from(base).join(".minecraft"))
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join("Library/Application Support/minecraft"))
    } else {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".minecraft"))
    }
}
