//! 整合包安装的编排层。
//!
//! **不重复实现下载**：
//!
//! - 整合包压缩包自己 → [`crate::download::run_batch`]
//! - 整合包里的模组文件 → [`crate::download::run_batch`]
//! - 游戏本体 + 加载器 → [`crate::install::Installer::run`]（和「安装」页走同一条路）
//!
//! 这一层只做三件事：解析格式、决定要下什么、把进度按阶段报出去。
//!
//! 阶段固定三步（`total = 3`）：
//!
//! 1. 下载整合包（`.mrpack` / `.zip`）
//! 2. 安装游戏本体与加载器（引擎的 `Progress` 原样转发）
//! 3. 写入整合包文件（下载 `files[]` + 解压 `overrides/`）

use std::path::PathBuf;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::download;
use crate::error::{Error, Result};
use crate::install::Installer;
use crate::isolate;
use crate::net::HttpClient;
use crate::paths::GamePaths;
use crate::types::{
    DownloadJob, InstallReport, InstallRequest, LoaderSpec, LogLevel, Source, StageKind,
};

use super::{
    curseforge, mrpack, EngineSink, FilesSink, PackProgress, PackProgressSink, PackSource,
    PackVersion,
};

/// 整合包安装一共几步。界面拿它画进度。
pub const PHASES: usize = 3;

/// 阶段 1 的两种叫法：从平台下载，还是读本地文件。
const STAGE_FETCH: &str = "下载整合包";
const STAGE_READ_LOCAL: &str = "读取整合包";

/// 本地压缩包是哪一种整合包。安装流程只按格式分派解析，不关心它从哪来。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackFormat {
    Modrinth,
    Curseforge,
}

impl PackFormat {
    pub fn label(self) -> &'static str {
        match self {
            PackFormat::Modrinth => "Modrinth",
            PackFormat::Curseforge => "CurseForge",
        }
    }

    pub fn source(self) -> PackSource {
        match self {
            PackFormat::Modrinth => PackSource::Modrinth,
            PackFormat::Curseforge => PackSource::Curseforge,
        }
    }
}

/// 只读地判断一个本地文件是什么格式：不下载、不写盘、不改文件。
///
/// 失败信息要能区分三种情况，因为用户能做的事完全不同：
/// - 文件不存在 / 不是文件 → 路径写错了；
/// - 打不开 zip → 下载不完整，或者根本不是压缩包；
/// - 是 zip 但两个索引都没有 → 拿错了文件（例如把模组 jar 当整合包）。
pub fn detect_format(archive: &std::path::Path) -> Result<PackFormat> {
    if !archive.exists() {
        return Err(Error::other(format!(
            "找不到这个文件：{}",
            archive.display()
        )));
    }
    if !archive.is_file() {
        return Err(Error::other(format!(
            "这不是一个文件：{}",
            archive.display()
        )));
    }
    if mrpack::read_entry(archive, mrpack::INDEX_NAME)?.is_some() {
        return Ok(PackFormat::Modrinth);
    }
    if mrpack::read_entry(archive, mrpack::CURSE_MANIFEST_NAME)?.is_some() {
        return Ok(PackFormat::Curseforge);
    }
    Err(Error::other(format!(
        "{} 里既没有 {}，也没有 {}，不是整合包",
        archive.display(),
        mrpack::INDEX_NAME,
        mrpack::CURSE_MANIFEST_NAME
    )))
}

fn format_of(source: PackSource) -> PackFormat {
    match source {
        PackSource::Modrinth => PackFormat::Modrinth,
        PackSource::Curseforge => PackFormat::Curseforge,
    }
}

// ---------------------------------------------------------------- 请求

/// 安装一个整合包。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PackInstallRequest {
    /// `.minecraft` 所在目录
    pub game_dir: PathBuf,
    /// 整合包从哪个平台来
    pub source: PackSource,
    /// Modrinth：slug 或 project id；CurseForge：数字 id（字符串）
    pub project_id: String,
    /// 指定版本；不填取最新
    #[serde(default)]
    pub version_id: Option<String>,
    /// 本地文件（`.mrpack` / CurseForge 的 `.zip`）。填了就不走平台下载，
    /// 阶段 1 从「下载整合包」变成「读取整合包」，格式按文件内容判断，
    /// 这时 `project_id` 可以为空。
    #[serde(default)]
    pub archive: Option<PathBuf>,
    /// 界面已经知道的名字，用来给报告和日志起标题（省一次网络请求）
    #[serde(default)]
    pub project_title: Option<String>,
    /// 仅 CurseForge：用户自己申请的 key
    #[serde(default)]
    pub api_key: Option<String>,
    /// Minecraft 文件走官方还是 BMCLAPI
    #[serde(default)]
    pub download_source: Source,
    #[serde(default = "super::default_concurrency")]
    pub concurrency: usize,
    /// 指定 Java；不填则自动探测 / 下载
    #[serde(default)]
    pub java: Option<PathBuf>,
    /// 跳过资源文件
    #[serde(default)]
    pub skip_assets: bool,
    /// 版本隔离：模组写进 `versions/<版本id>/mods/`（开，默认）还是根 `mods/`（关）。
    ///
    /// 落点只有 [`crate::isolate::mods_dir`] 一处算法。版本 id 要等加载器装完才知道，
    /// 所以这个开关在**阶段 3**（写整合包文件）才真正生效。
    #[serde(default = "crate::isolate::default_on")]
    pub isolated: bool,
}

impl PackInstallRequest {
    pub fn new(
        game_dir: impl Into<PathBuf>,
        source: PackSource,
        project_id: impl Into<String>,
    ) -> Self {
        Self {
            game_dir: game_dir.into(),
            source,
            project_id: project_id.into(),
            version_id: None,
            archive: None,
            project_title: None,
            api_key: None,
            download_source: Source::Official,
            concurrency: super::default_concurrency(),
            java: None,
            skip_assets: false,
            isolated: true,
        }
    }
}

// ---------------------------------------------------------------- 报告

/// 装完一个整合包之后的账。
///
/// `base` 是引擎那次「原版 + 加载器」安装的报告，字段原样保留；
/// 这里的 `pack_*` 只统计整合包自己带来的文件。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackInstallReport {
    pub source: PackSource,
    pub pack_id: String,
    pub pack_name: String,
    pub pack_version: String,
    pub mc_version: String,
    /// 例如 `Fabric 0.16.9`；原版整合包是 `None`
    pub loader: Option<String>,
    /// `versions/` 下可以直接用的版本目录名（来自引擎）
    pub installed_version: String,
    /// 这次模组落在哪（相对 gameDir），例如 `versions/xxx/mods` 或 `mods`
    #[serde(default)]
    pub mods_dir: String,
    /// 这次安装用的是不是版本隔离
    #[serde(default)]
    pub isolated: bool,
    pub game_dir: PathBuf,
    /// 这次真正下载的整合包文件数（不含已存在的）
    pub pack_files: usize,
    /// 这次真正下载的字节数
    pub pack_bytes: u64,
    /// 被跳过的条目：已存在、客户端不需要、或平台不给下载地址
    pub skipped: usize,
    /// 解压出来的覆盖文件数
    pub overrides: usize,
    /// 不致命但用户该知道的事（例如某几个文件作者禁止第三方下载）
    pub warnings: Vec<String>,
    pub duration_ms: u64,
    pub base: InstallReport,
}

// ---------------------------------------------------------------- 主流程

/// 装一个整合包：下载 → 用引擎装原版和加载器 → 铺整合包文件。
pub async fn install_pack(
    req: &PackInstallRequest,
    sink: &dyn PackProgressSink,
) -> Result<PackInstallReport> {
    let started = Instant::now();

    // 本地文件走「导入」，平台走「下载」。两者的阶段 2/3 完全一样，只是阶段 1 不同。
    let local = req
        .archive
        .as_ref()
        .filter(|p| !p.as_os_str().is_empty())
        .cloned();
    if local.is_none() && req.project_id.trim().is_empty() {
        return Err(Error::other("没有指定要装的整合包"));
    }

    let client = HttpClient::new()?;
    let paths = GamePaths::new(&req.game_dir);
    paths.prepare()?;

    // 界面已经知道名字时优先用它（省一次网络请求）；否则等索引里的名字。
    let fallback_name = req
        .project_title
        .clone()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| req.project_id.clone());

    // ---- 阶段 1：拿到压缩包（平台下载 / 读本地文件）
    let (format, archive, fallback_version, pack_id) = match &local {
        Some(path) => {
            begin(sink, 1, STAGE_READ_LOCAL);
            let format = detect_format(path)?;
            let file_name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            sink.emit(PackProgress::Log {
                level: LogLevel::Info,
                message: format!("导入 {}（{}）", file_name, format.label()),
            });
            // 本地文件没有下载量，但界面要立刻看到阶段 1 走完。
            sink.emit(PackProgress::Files {
                done: 1,
                total: 1,
                bytes_done: 0,
                bytes_total: 0,
            });
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            (format, path.clone(), stem.clone(), stem)
        }
        None => {
            begin(sink, 1, STAGE_FETCH);
            let (version, archive) = fetch_archive(&client, req, &paths, sink).await?;
            (
                format_of(req.source),
                archive,
                version.version_number,
                req.project_id.clone(),
            )
        }
    };

    // ---- 读索引：依赖 + 文件清单
    //
    // 这里**不**拼下载任务：模组落点取决于版本隔离，而隔离目录里的版本 id
    // 要等加载器装完才知道（`fabric-loader-0.16.9-1.21.1` 这种）。所以先只把
    // 索引留在 `files` 里，阶段 3 再算落点、再拼任务。
    let plan = build_plan(format, &archive, fallback_name, fallback_version)?;

    let opts = RunOpts {
        game_dir: &req.game_dir,
        source: format.source(),
        api_key: req.api_key.as_deref(),
        download_source: req.download_source,
        concurrency: req.concurrency,
        java: req.java.as_deref(),
        skip_assets: req.skip_assets,
        isolated: req.isolated,
        pack_id,
    };
    run_plan(&client, &opts, plan, &archive, sink, started).await
}

/// 阶段 2/3 需要的一切。`install_pack` 组装它，两条入口（平台 / 本地）共用。
struct RunOpts<'a> {
    game_dir: &'a std::path::Path,
    source: PackSource,
    api_key: Option<&'a str>,
    download_source: Source,
    concurrency: usize,
    java: Option<&'a std::path::Path>,
    skip_assets: bool,
    isolated: bool,
    pack_id: String,
}

/// 装完「包已经拿到、索引已经解析」之后的那一段：本体 + 加载器 → 包内文件 → 覆盖。
async fn run_plan(
    client: &HttpClient,
    opts: &RunOpts<'_>,
    plan: PackPlan,
    archive: &std::path::Path,
    sink: &dyn PackProgressSink,
    started: Instant,
) -> Result<PackInstallReport> {
    let mut warnings: Vec<String> = Vec::new();
    let mut skipped = 0usize;

    sink.emit(PackProgress::Log {
        level: LogLevel::Info,
        message: format!(
            "整合包：{} {} · Minecraft {}{}",
            plan.name,
            plan.version,
            plan.mc_version,
            match &plan.loader {
                Some(l) => format!(" · {}", l.display()),
                None => " · 原版".to_string(),
            }
        ),
    });

    // ---- 阶段 2：游戏本体 + 加载器（复用引擎）
    begin(sink, 2, "安装游戏本体与加载器");
    let base_req = InstallRequest {
        game_dir: opts.game_dir.to_path_buf(),
        mc_version: plan.mc_version.clone(),
        loader: plan.loader.clone(),
        source: opts.download_source,
        concurrency: opts.concurrency.max(1),
        java: opts.java.map(|p| p.to_path_buf()),
        skip_assets: opts.skip_assets,
    };
    let base = Installer::new()?
        .run(&base_req, &EngineSink { sink })
        .await?;

    // ---- 阶段 3：整合包自己的文件 + 覆盖
    begin(sink, 3, "写入整合包文件");

    // 版本隔离的落点：加载器装完才知道 `versions/` 下的最终目录名。
    let version_id = base.installed_version.clone();
    let mods = isolate::mods_dir(opts.game_dir, &version_id, opts.isolated)?;
    sink.emit(PackProgress::Log {
        level: LogLevel::Info,
        message: if opts.isolated {
            format!(
                "版本隔离：模组写入 {}（只有 PCL / HMCL 这类启动器会读这个目录）",
                mods.label()
            )
        } else {
            format!(
                "版本隔离已关闭：模组写入共享的 {}（多个整合包会混在一起）",
                mods.label()
            )
        },
    });

    let (jobs, dropped, mut notes) = match &plan.files {
        PackFiles::Modrinth(index) => {
            mrpack_jobs(index, opts.game_dir, &version_id, opts.isolated)?
        }
        PackFiles::Curseforge(manifest) => {
            curseforge_jobs(client, opts.api_key, manifest, &mods).await?
        }
    };
    skipped += dropped;
    warnings.append(&mut notes);

    let outcome = download::run_batch(
        client,
        jobs,
        opts.concurrency,
        &FilesSink { sink },
        StageKind::Libraries,
    )
    .await?;

    let dirs: Vec<&str> = plan.override_dirs.iter().map(String::as_str).collect();
    let overrides = mrpack::extract_overrides_isolated(
        archive,
        opts.game_dir,
        &dirs,
        Some((&version_id, opts.isolated)),
    )?;

    let report = PackInstallReport {
        source: opts.source,
        pack_id: opts.pack_id.clone(),
        pack_name: plan.name.clone(),
        pack_version: plan.version.clone(),
        mc_version: plan.mc_version.clone(),
        loader: plan.loader.as_ref().map(|l| l.display()),
        installed_version: base.installed_version.clone(),
        mods_dir: mods.relative.clone(),
        isolated: mods.isolated,
        game_dir: opts.game_dir.to_path_buf(),
        pack_files: outcome.downloaded,
        pack_bytes: outcome.bytes,
        skipped: skipped + outcome.skipped,
        overrides,
        warnings,
        duration_ms: started.elapsed().as_millis() as u64,
        base,
    };

    sink.emit(PackProgress::Log {
        level: LogLevel::Info,
        message: format!(
            "完成：{} {} · 整合包文件 {} 个（{:.1} MB）· 覆盖 {} 个 · 用时 {:.1}s",
            report.pack_name,
            report.pack_version,
            report.pack_files,
            report.pack_bytes as f64 / 1_048_576.0,
            report.overrides,
            report.duration_ms as f64 / 1000.0
        ),
    });

    Ok(report)
}

/// 按格式解析索引，得到这次要装什么。
fn build_plan(
    format: PackFormat,
    archive: &std::path::Path,
    fallback_name: String,
    fallback_version: String,
) -> Result<PackPlan> {
    match format {
        PackFormat::Modrinth => {
            let index = mrpack::read_index(archive)?;
            let mc_version = index.mc_version()?;
            let loader = index.loader_spec()?;
            let name = if index.name.trim().is_empty() {
                fallback_name
            } else {
                index.name.trim().to_string()
            };
            let version = if index.version_id.trim().is_empty() {
                fallback_version
            } else {
                index.version_id.trim().to_string()
            };
            Ok(PackPlan {
                name,
                version,
                mc_version,
                loader,
                files: PackFiles::Modrinth(index),
                override_dirs: mrpack::default_override_dirs()
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
            })
        }
        PackFormat::Curseforge => {
            let manifest = mrpack::read_curse_manifest(archive)?;
            let mc_version = manifest.minecraft.version.trim().to_string();
            let loader = manifest.loader_spec()?;
            let override_dirs = vec![manifest.overrides_dir()];
            let name = if manifest.name.trim().is_empty() {
                fallback_name
            } else {
                manifest.name.trim().to_string()
            };
            let version = if manifest.version.trim().is_empty() {
                fallback_version
            } else {
                manifest.version.trim().to_string()
            };
            Ok(PackPlan {
                name,
                version,
                mc_version,
                loader,
                files: PackFiles::Curseforge(manifest),
                override_dirs,
            })
        }
    }
}

/// 解析后的一次安装计划。
struct PackPlan {
    name: String,
    version: String,
    mc_version: String,
    loader: Option<LoaderSpec>,
    /// 原始文件清单。拼下载任务要等版本目录名确定（版本隔离的落点要用它）。
    files: PackFiles,
    override_dirs: Vec<String>,
}

/// 整合包自带的文件清单，两种平台格式各一份。
enum PackFiles {
    Modrinth(mrpack::ModpackIndex),
    Curseforge(mrpack::CurseManifest),
}

fn begin(sink: &dyn PackProgressSink, index: usize, label: &str) {
    sink.emit(PackProgress::Stage {
        index,
        total: PHASES,
        label: label.to_string(),
    });
}

// ---------------------------------------------------------------- 压缩包

/// 解析目标版本并把它下到 `<gameDir>/.mcdl/packs/`。
async fn fetch_archive(
    client: &HttpClient,
    req: &PackInstallRequest,
    paths: &GamePaths,
    sink: &dyn PackProgressSink,
) -> Result<(PackVersion, PathBuf)> {
    let version = super::resolve_version(
        client,
        req.source,
        &req.project_id,
        req.version_id.as_deref(),
        req.api_key.as_deref(),
    )
    .await?;

    let file = version.archive_file().cloned().ok_or_else(|| {
        Error::other(format!(
            "{} 版本「{}」没有可下载的整合包文件",
            req.source.label(),
            version.version_number
        ))
    })?;

    let cache = super::archive_cache_dir(paths.root());
    std::fs::create_dir_all(&cache).map_err(|e| Error::io(&cache, e))?;
    let dest = cache.join(super::safe_file_name(&file.filename));

    sink.emit(PackProgress::Log {
        level: LogLevel::Info,
        message: format!(
            "{} 版本 {} · {}（{:.1} MB）",
            req.source.label(),
            version.version_number,
            file.filename,
            file.size as f64 / 1_048_576.0
        ),
    });

    let job = DownloadJob {
        url: file.url.clone(),
        dest: dest.clone(),
        sha1: file.sha1.clone(),
        size: Some(file.size).filter(|s| *s > 0),
        label: file.filename.clone(),
    };
    download::run_batch(
        client,
        vec![job],
        1,
        &FilesSink { sink },
        StageKind::Libraries,
    )
    .await?;

    Ok((version, dest))
}

// ---------------------------------------------------------------- 文件清单

/// 把 Modrinth 索引里的 `files[]` 变成下载任务。
///
/// 返回 `(任务, 跳过数, 警告)`。跳过的是「客户端不需要」的条目；
/// 没有下载地址的也跳过，但要给一条警告——静默少装一个模组是最难查的故障。
///
/// 落点一律交给 [`isolate::place_pack_file`]：`mods/` 下的条目跟着版本隔离走，
/// `config/`、`resourcepacks/` 等仍按 PCL 的约定留在游戏根目录。
fn mrpack_jobs(
    index: &mrpack::ModpackIndex,
    game_dir: &std::path::Path,
    version_id: &str,
    isolated: bool,
) -> Result<(Vec<DownloadJob>, usize, Vec<String>)> {
    let mut jobs = Vec::new();
    let mut skipped = 0usize;
    let mut warnings = Vec::new();

    for file in &index.files {
        if !file.needed_on_client() {
            skipped += 1;
            continue;
        }
        let Some(url) = file.url() else {
            skipped += 1;
            warnings.push(format!("{} 没有下载地址，已跳过", file.path));
            continue;
        };
        // safe_path 先挡一次穿越；place_pack_file 再挡一次，因为这里才是落盘点。
        file.safe_path()?;
        let dest = isolate::place_pack_file(game_dir, version_id, isolated, &file.path)?;
        jobs.push(DownloadJob {
            url: url.to_string(),
            dest,
            sha1: file.hashes.sha1.clone(),
            size: file.file_size,
            label: file.path.clone(),
        });
    }

    Ok((jobs, skipped, warnings))
}

/// 把 CurseForge `manifest.json` 里的 `(projectID, fileID)` 逐条解析成下载任务。
///
/// 这一步必须联网（每个文件一次请求），所以只在真的安装时才做，
/// 搜索和看详情不需要 key 之外的任何东西。
async fn curseforge_jobs(
    client: &HttpClient,
    api_key: Option<&str>,
    manifest: &mrpack::CurseManifest,
    mods_dir: &isolate::ModsTarget,
) -> Result<(Vec<DownloadJob>, usize, Vec<String>)> {
    let key = curseforge::require_key(api_key)?;
    let mut jobs = Vec::new();
    let mut skipped = 0usize;
    let mut warnings = Vec::new();

    for entry in &manifest.files {
        let file = match curseforge::resolve_file(
            client,
            Some(key),
            entry.project_id,
            entry.file_id,
        )
        .await
        {
            Ok(file) => file,
            Err(e) => {
                skipped += 1;
                warnings.push(format!(
                    "project {} / file {} 解析失败：{e}",
                    entry.project_id, entry.file_id
                ));
                continue;
            }
        };

        if !file.downloadable() {
            skipped += 1;
            warnings.push(format!(
                "{}（project {}）的作者禁止第三方下载，需要去 CurseForge 手动下载",
                file.filename, entry.project_id
            ));
            continue;
        }

        jobs.push(DownloadJob {
            url: file.url.clone(),
            dest: mods_dir.join(&super::safe_file_name(&file.filename)),
            sha1: file.sha1.clone(),
            size: Some(file.size).filter(|s| *s > 0),
            label: file.filename.clone(),
        });
    }

    Ok((jobs, skipped, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn index() -> mrpack::ModpackIndex {
        mrpack::parse_index(
            br#"{
                "name": "Test Pack",
                "versionId": "1.0.0",
                "files": [
                    { "path": "mods/sodium.jar", "hashes": { "sha1": "aa" },
                      "downloads": ["https://example.invalid/sodium.jar"], "fileSize": 10 },
                    { "path": "mods/serveronly.jar",
                      "env": { "client": "unsupported" },
                      "downloads": ["https://example.invalid/s.jar"] },
                    { "path": "mods/nourl.jar", "downloads": [] }
                ],
                "dependencies": { "minecraft": "1.21.1", "fabric-loader": "0.16.9" }
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn jobs_land_under_the_game_dir_and_keep_hashes() {
        let dir = Path::new("C:/mc");
        let (jobs, skipped, warnings) = mrpack_jobs(&index(), dir, "1.21.1-fabric", false).unwrap();

        assert_eq!(jobs.len(), 1, "只有 sodium 该下");
        assert_eq!(skipped, 2, "客户端不需要的和没地址的都算跳过");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("没有下载地址"), "{:?}", warnings);

        let job = &jobs[0];
        assert!(job.dest.ends_with("mods/sodium.jar") || job.dest.ends_with("mods\\sodium.jar"));
        assert_eq!(job.sha1.as_deref(), Some("aa"));
        assert_eq!(job.size, Some(10));
        assert_eq!(job.label, "mods/sodium.jar");
    }

    /// 版本隔离开着时，整合包里的模组必须落进 `versions/<id>/mods/`，
    /// 而且 `config/` 这类条目不该跟着跑进去。
    #[test]
    fn jobs_follow_version_isolation() {
        let dir = Path::new("C:/mc");
        let (jobs, _, _) = mrpack_jobs(&index(), dir, "fabric-loader-0.16.9-1.21.1", true).unwrap();
        let dest = &jobs[0].dest;
        assert!(
            dest.ends_with("versions/fabric-loader-0.16.9-1.21.1/mods/sodium.jar")
                || dest.ends_with("versions\\fabric-loader-0.16.9-1.21.1\\mods\\sodium.jar"),
            "实际是 {}",
            dest.display()
        );
        assert!(!dest.starts_with(dir.join("mods")), "不该落到根 mods/");

        // 非 mods 条目留在根目录
        let mut idx = index();
        idx.files.push(mrpack::ModpackFile {
            path: "config/x.toml".into(),
            downloads: vec!["https://example.invalid/x.toml".into()],
            ..Default::default()
        });
        let (jobs, _, _) = mrpack_jobs(&idx, dir, "1.21.1", true).unwrap();
        let config = jobs.iter().find(|j| j.label == "config/x.toml").unwrap();
        assert!(config.dest.ends_with("config/x.toml") || config.dest.ends_with("config\\x.toml"));
        assert!(!config.dest.to_string_lossy().contains("versions"));
    }

    /// 隔离开着但没有版本目录名：宁可报错，也不能悄悄写进根 mods/。
    #[test]
    fn isolated_jobs_refuse_to_guess_the_version_dir() {
        let dir = Path::new("C:/mc");
        assert!(mrpack_jobs(&index(), dir, "", true).is_err());
        // 关掉隔离时版本名无所谓
        assert!(mrpack_jobs(&index(), dir, "", false).is_ok());
    }

    #[test]
    fn jobs_reject_traversal_before_downloading() {
        // parse_index 已经拦过一次；这里再拦一次，因为 jobs 是真正落盘的地方。
        let bad = mrpack::ModpackIndex {
            files: vec![mrpack::ModpackFile {
                path: "../escape.jar".into(),
                downloads: vec!["https://example.invalid/e.jar".into()],
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(mrpack_jobs(&bad, Path::new("C:/mc"), "1.21.1", false).is_err());
        assert!(mrpack_jobs(&bad, Path::new("C:/mc"), "1.21.1", true).is_err());
    }

    #[test]
    fn request_deserializes_with_defaults() {
        // 这是 Tauri 命令层的入参协议：前端只传必填字段也要能反序列化。
        let req: PackInstallRequest = serde_json::from_str(
            r#"{
                "game_dir": "C:/mc",
                "source": "modrinth",
                "project_id": "fabulously-optimized"
            }"#,
        )
        .unwrap();
        assert_eq!(req.concurrency, 16);
        assert_eq!(req.download_source, Source::Official);
        assert!(!req.skip_assets);
        assert!(req.version_id.is_none());
        assert!(req.api_key.is_none());
    }

    #[test]
    fn request_round_trips_through_json() {
        let mut req = PackInstallRequest::new("C:/mc", PackSource::Curseforge, "123");
        req.version_id = Some("456".into());
        req.api_key = Some("secret".into());
        let text = serde_json::to_string(&req).unwrap();
        let back: PackInstallRequest = serde_json::from_str(&text).unwrap();
        assert_eq!(back.source, PackSource::Curseforge);
        assert_eq!(back.version_id.as_deref(), Some("456"));
        assert_eq!(back.api_key.as_deref(), Some("secret"));
    }

    #[test]
    fn phases_are_emitted_with_a_fixed_total() {
        let events = std::sync::Mutex::new(Vec::new());
        let sink = |p: PackProgress| events.lock().unwrap().push(p);
        for i in 1..=PHASES {
            begin(&sink, i, "x");
        }
        let events = events.into_inner().unwrap();
        assert_eq!(events.len(), PHASES);
        for (i, event) in events.iter().enumerate() {
            match event {
                PackProgress::Stage { index, total, .. } => {
                    assert_eq!(*index, i + 1);
                    assert_eq!(*total, PHASES);
                }
                _ => panic!("begin 只该发 Stage"),
            }
        }
    }

    /// 真实网络：下载一个真的 `.mrpack`，解析索引、拼下载任务、解压覆盖文件。
    ///
    /// 手动跑：`cargo test -p mcdl-core pack::install -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "需要真实网络（会下载一个几百 KB 的 .mrpack）"]
    async fn download_and_parse_a_real_mrpack() {
        let client = HttpClient::new().unwrap();
        let version =
            super::super::resolve_version(&client, PackSource::Modrinth, "opti-mized", None, None)
                .await
                .unwrap();
        let file = version.archive_file().cloned().expect("应该有 .mrpack");
        println!(
            "拿到版本 {} · {}（{} 字节）",
            version.version_number, file.filename, file.size
        );

        let dir = temp_game_dir("real-mrpack");
        let dest = dir.join(super::super::safe_file_name(&file.filename));
        let job = DownloadJob {
            url: file.url.clone(),
            dest: dest.clone(),
            sha1: file.sha1.clone(),
            size: Some(file.size),
            label: file.filename.clone(),
        };
        download::run_batch(
            &client,
            vec![job],
            1,
            &crate::types::NullSink,
            StageKind::Libraries,
        )
        .await
        .unwrap();
        assert!(dest.exists());

        let index = mrpack::read_index(&dest).unwrap();
        println!(
            "索引：{} · {} 个文件 · 依赖 {:?}",
            index.name,
            index.files.len(),
            index.dependencies
        );
        let mc = index.mc_version().unwrap();
        let loader = index.loader_spec().unwrap();
        println!(
            "要求 Minecraft {} / 加载器 {}",
            mc,
            loader
                .as_ref()
                .map(|l| l.display())
                .unwrap_or_else(|| "原版".into())
        );

        let (jobs, skipped, warnings) = mrpack_jobs(&index, &dir, "1.21.1", false).unwrap();
        println!(
            "下载任务 {} 个 · 跳过 {} 个 · 警告 {} 条 {:?}",
            jobs.len(),
            skipped,
            warnings.len(),
            warnings
        );
        let first = jobs.first().expect("这个包应该有要下的文件");
        println!(
            "示例：{} → {}（sha1 {:?}）",
            first.url, first.label, first.sha1
        );

        let overrides =
            mrpack::extract_overrides(&dest, &dir, &mrpack::default_override_dirs()).unwrap();
        println!("覆盖文件 {overrides} 个");

        assert!(!mc.is_empty());
        assert!(!jobs.is_empty());
        assert!(jobs.iter().all(|j| j.dest.starts_with(&dir)));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 真实安装一次，把每条进度事件按时间戳打出来。
    ///
    /// 这个用例的用途是回答「装的时候到底有没有进度」：如果所有事件都挤在最后
    /// 一秒，说明引擎是装完才一次性发；如果是几秒到几十秒均匀铺开，说明引擎没问题，
    /// 看不到进度是界面把读数画在了看不见的地方。
    ///
    /// 文件事件很密（每个下载块都可能触发），这里每 400 ms 采样一条，够看出在走。
    ///
    /// ```text
    /// cargo test -p mcdl-core pack::install::tests::real_install_prints_progress -- --ignored --nocapture
    /// ```
    #[tokio::test]
    #[ignore = "真实网络 + 真实安装（约 100 MB），约 20 秒"]
    async fn real_install_prints_progress() {
        use std::sync::Mutex;
        use std::time::{Duration, Instant};

        let started = Instant::now();
        let last = Mutex::new(Instant::now() - Duration::from_secs(5));
        let sink = |p: PackProgress| {
            let at = started.elapsed().as_secs_f64();
            // 采样式打印：400 ms 内只留一条，收尾那条（done == total）一定留。
            let sample = |line: String| {
                let mut last = last.lock().unwrap();
                if last.elapsed() < Duration::from_millis(400) {
                    return;
                }
                *last = Instant::now();
                println!("[{at:>7.2}s] {line}");
            };
            match p {
                PackProgress::Stage {
                    index,
                    total,
                    label,
                } => println!("[{at:>7.2}s] stage {index}/{total} {label}"),
                PackProgress::Files {
                    done,
                    total,
                    bytes_done,
                    bytes_total,
                } => {
                    if done == total {
                        println!("[{at:>7.2}s] files {done}/{total} {bytes_done}/{bytes_total} B  ← 阶段完成");
                        *last.lock().unwrap() = Instant::now();
                    } else {
                        sample(format!("files {done}/{total} {bytes_done}/{bytes_total} B"));
                    }
                }
                PackProgress::Log { level, message } => {
                    println!("[{at:>7.2}s] log {level:?} {message}")
                }
                PackProgress::Engine { progress } => match progress {
                    crate::types::Progress::Stage {
                        index,
                        total,
                        label,
                        ..
                    } => println!("[{at:>7.2}s] engine stage {index}/{total} {label}"),
                    crate::types::Progress::Files {
                        done,
                        total,
                        bytes_done,
                        bytes_total,
                        ..
                    } => {
                        if done == total {
                            println!("[{at:>7.2}s] engine files {done}/{total} {bytes_done}/{bytes_total} B  ← 阶段完成");
                            *last.lock().unwrap() = Instant::now();
                        } else {
                            sample(format!(
                                "engine files {done}/{total} {bytes_done}/{bytes_total} B"
                            ));
                        }
                    }
                    crate::types::Progress::Log { message, .. } => {
                        println!("[{at:>7.2}s] engine log {message}")
                    }
                },
            }
        };

        let dir = temp_game_dir("real-install-progress");
        let mut req = PackInstallRequest::new(&dir, PackSource::Modrinth, "fps");
        // 固定一个 1.20.4 + Fabric 的版本：本机 Java 17 就够，不用下运行时。
        req.version_id = Some("BunXzEyL".into());
        req.skip_assets = true;

        let report = install_pack(&req, &sink).await.unwrap();
        println!(
            "\n=== 报告：{} {} · MC {} · {} · 整合包文件 {} 个 / {} B · 用时 {} ms ===",
            report.pack_name,
            report.pack_version,
            report.mc_version,
            report.loader.as_deref().unwrap_or("原版"),
            report.pack_files,
            report.pack_bytes,
            report.duration_ms
        );
        assert!(report.pack_files > 0);
        assert!(report.duration_ms > 3_000, "真实安装不可能 3 秒内装完");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 临时目录：安装流程会 `create_dir_all`，测试不能往 `C:\mc` 里写。
    fn temp_game_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mcdl-pack-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 造一个只含指定条目的 zip，用来测格式识别。
    fn temp_zip(dir: &Path, name: &str, entries: &[(&str, &[u8])]) -> PathBuf {
        use std::io::Write as _;
        let path = dir.join(name);
        let file = std::fs::File::create(&path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (entry, body) in entries {
            writer.start_file(*entry, options).unwrap();
            writer.write_all(body).unwrap();
        }
        writer.finish().unwrap();
        path
    }

    #[test]
    fn detect_format_tells_the_two_pack_formats_apart() {
        let dir = temp_game_dir("detect");
        let mrpack = temp_zip(
            &dir,
            "a.mrpack",
            &[(
                mrpack::INDEX_NAME,
                br#"{"files":[],"dependencies":{"minecraft":"1.21.1"}}"#,
            )],
        );
        let cf = temp_zip(
            &dir,
            "b.zip",
            &[(
                mrpack::CURSE_MANIFEST_NAME,
                br#"{"minecraft":{"version":"1.20.1"},"files":[]}"#,
            )],
        );
        assert_eq!(detect_format(&mrpack).unwrap(), PackFormat::Modrinth);
        assert_eq!(detect_format(&cf).unwrap(), PackFormat::Curseforge);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 拿错文件时要说清是哪一步错了，用户才知道下一步做什么。
    #[test]
    fn detect_format_explains_each_failure() {
        let dir = temp_game_dir("detect-bad");

        // 1) 路径不存在
        let missing = dir.join("nope.mrpack");
        let err = detect_format(&missing).unwrap_err().to_string();
        assert!(err.contains("找不到这个文件"), "实际是：{err}");

        // 2) 是目录不是文件
        let folder = dir.join("folder");
        std::fs::create_dir_all(&folder).unwrap();
        let err = detect_format(&folder).unwrap_err().to_string();
        assert!(err.contains("不是一个文件"), "实际是：{err}");

        // 3) 不是 zip（下载不完整 / 拿错文件）
        let broken = dir.join("broken.mrpack");
        std::fs::write(&broken, b"not a zip at all").unwrap();
        let err = detect_format(&broken).unwrap_err().to_string();
        assert!(err.contains("打不开整合包压缩包"), "实际是：{err}");

        // 4) 是 zip，但两个索引都没有
        let empty = temp_zip(&dir, "empty.zip", &[("readme.txt", b"hi")]);
        let err = detect_format(&empty).unwrap_err().to_string();
        assert!(err.contains(mrpack::INDEX_NAME), "实际是：{err}");
        assert!(err.contains(mrpack::CURSE_MANIFEST_NAME), "实际是：{err}");
        assert!(err.contains("不是整合包"), "实际是：{err}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 本地导入不需要 project_id；两个都没填才该报错。
    ///
    /// 这里用「索引里没有 minecraft 版本」的假包来停在解析阶段：
    /// 能报出这个错，就说明已经越过了「必须给 project_id」的检查，而且不会联网。
    #[test]
    fn install_accepts_a_local_archive_without_a_project_id() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let dir = temp_game_dir("local-import");
        let archive = temp_zip(
            &dir,
            "pack.mrpack",
            &[(
                mrpack::INDEX_NAME,
                br#"{"files":[{"path":"mods/a.jar","downloads":["https://example.invalid/a.jar"]}],"dependencies":{}}"#,
            )],
        );

        let mut req = PackInstallRequest::new(&dir, PackSource::Modrinth, "");
        req.archive = Some(archive);
        let err = rt
            .block_on(install_pack(&req, &crate::pack::NullPackSink))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("minecraft"),
            "应该停在「索引没写 minecraft 版本」，实际是：{err}"
        );
        assert!(
            !err.contains("没有指定要装的整合包"),
            "本地导入不该要求 project_id"
        );

        // 两个都空才是真错误
        let mut empty = PackInstallRequest::new(&dir, PackSource::Modrinth, "  ");
        empty.archive = None;
        let err = rt
            .block_on(install_pack(&empty, &crate::pack::NullPackSink))
            .unwrap_err()
            .to_string();
        assert!(err.contains("没有指定要装的整合包"), "实际是：{err}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn request_carries_the_local_archive_path() {
        let req: PackInstallRequest = serde_json::from_str(
            r#"{
                "game_dir": "C:/mc",
                "source": "modrinth",
                "project_id": "",
                "archive": "D:/下载/pack.mrpack"
            }"#,
        )
        .unwrap();
        assert_eq!(
            req.archive.as_deref(),
            Some(Path::new("D:/下载/pack.mrpack"))
        );
        // 老前端不传 archive 也要能反序列化
        let req: PackInstallRequest =
            serde_json::from_str(r#"{"game_dir":"C:/mc","source":"modrinth","project_id":"x"}"#)
                .unwrap();
        assert!(req.archive.is_none());
    }

    #[test]
    fn empty_project_id_fails_fast() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let dir = temp_game_dir("empty-id");
        let req = PackInstallRequest::new(&dir, PackSource::Modrinth, "   ");
        let err = rt
            .block_on(install_pack(&req, &crate::pack::NullPackSink))
            .unwrap_err()
            .to_string();
        assert!(err.contains("整合包"), "实际是：{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// CurseForge 没有 key 时，安装必须在联网前就失败，并说清怎么办。
    #[test]
    fn curseforge_install_without_key_fails_with_help() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let dir = temp_game_dir("cf-nokey");
        let req = PackInstallRequest::new(&dir, PackSource::Curseforge, "123");
        let err = rt
            .block_on(install_pack(&req, &crate::pack::NullPackSink))
            .unwrap_err()
            .to_string();
        assert!(err.contains("API key"), "实际是：{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
