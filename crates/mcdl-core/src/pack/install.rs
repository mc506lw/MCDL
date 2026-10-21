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
            project_title: None,
            api_key: None,
            download_source: Source::Official,
            concurrency: super::default_concurrency(),
            java: None,
            skip_assets: false,
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
    if req.project_id.trim().is_empty() {
        return Err(Error::other("没有指定要装的整合包"));
    }

    let client = HttpClient::new()?;
    let paths = GamePaths::new(&req.game_dir);
    paths.prepare()?;

    let mut warnings: Vec<String> = Vec::new();
    let mut skipped = 0usize;

    // 界面已经知道名字时优先用它（省一次网络请求）；否则等索引里的名字。
    let fallback_name = req
        .project_title
        .clone()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| req.project_id.clone());

    // ---- 阶段 1：整合包本体
    begin(sink, 1, "下载整合包");
    let (version, archive) = fetch_archive(&client, req, &paths, sink).await?;

    // ---- 读索引：依赖 + 文件清单
    let plan = match req.source {
        PackSource::Modrinth => {
            let index = mrpack::read_index(&archive)?;
            let mc_version = index.mc_version()?;
            let loader = index.loader_spec()?;
            let (jobs, dropped, mut notes) = mrpack_jobs(&index, &req.game_dir)?;
            skipped += dropped;
            warnings.append(&mut notes);
            let name = if index.name.trim().is_empty() {
                fallback_name.clone()
            } else {
                index.name.trim().to_string()
            };
            let version_text = if index.version_id.trim().is_empty() {
                version.version_number.clone()
            } else {
                index.version_id.trim().to_string()
            };
            PackPlan {
                name,
                version: version_text,
                mc_version,
                loader,
                jobs,
                override_dirs: mrpack::default_override_dirs()
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
            }
        }
        PackSource::Curseforge => {
            let manifest = mrpack::read_curse_manifest(&archive)?;
            let mc_version = manifest.minecraft.version.trim().to_string();
            let loader = manifest.loader_spec()?;
            let (jobs, dropped, mut notes) =
                curseforge_jobs(&client, req, &manifest).await?;
            skipped += dropped;
            warnings.append(&mut notes);
            PackPlan {
                name: if manifest.name.trim().is_empty() {
                    fallback_name.clone()
                } else {
                    manifest.name.trim().to_string()
                },
                version: if manifest.version.trim().is_empty() {
                    version.version_number.clone()
                } else {
                    manifest.version.trim().to_string()
                },
                mc_version,
                loader,
                jobs,
                override_dirs: vec![manifest.overrides_dir()],
            }
        }
    };

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
    for note in &warnings {
        sink.emit(PackProgress::Log {
            level: LogLevel::Warn,
            message: note.clone(),
        });
    }

    // ---- 阶段 2：游戏本体 + 加载器（复用引擎）
    begin(sink, 2, "安装游戏本体与加载器");
    let base_req = InstallRequest {
        game_dir: req.game_dir.clone(),
        mc_version: plan.mc_version.clone(),
        loader: plan.loader.clone(),
        source: req.download_source,
        concurrency: req.concurrency.max(1),
        java: req.java.clone(),
        skip_assets: req.skip_assets,
    };
    let base = Installer::new()?
        .run(&base_req, &EngineSink { sink })
        .await?;

    // ---- 阶段 3：整合包自己的文件 + 覆盖
    begin(sink, 3, "写入整合包文件");
    let outcome = download::run_batch(
        &client,
        plan.jobs.clone(),
        req.concurrency,
        &FilesSink { sink },
        StageKind::Libraries,
    )
    .await?;

    let dirs: Vec<&str> = plan.override_dirs.iter().map(String::as_str).collect();
    let overrides = mrpack::extract_overrides(&archive, &req.game_dir, &dirs)?;

    let report = PackInstallReport {
        source: req.source,
        pack_id: req.project_id.clone(),
        pack_name: plan.name.clone(),
        pack_version: plan.version.clone(),
        mc_version: plan.mc_version.clone(),
        loader: plan.loader.as_ref().map(|l| l.display()),
        installed_version: base.installed_version.clone(),
        game_dir: req.game_dir.clone(),
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

/// 解析后的一次安装计划。
struct PackPlan {
    name: String,
    version: String,
    mc_version: String,
    loader: Option<LoaderSpec>,
    jobs: Vec<DownloadJob>,
    override_dirs: Vec<String>,
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
    download::run_batch(client, vec![job], 1, &FilesSink { sink }, StageKind::Libraries).await?;

    Ok((version, dest))
}

// ---------------------------------------------------------------- 文件清单

/// 把 Modrinth 索引里的 `files[]` 变成下载任务。
///
/// 返回 `(任务, 跳过数, 警告)`。跳过的是「客户端不需要」的条目；
/// 没有下载地址的也跳过，但要给一条警告——静默少装一个模组是最难查的故障。
fn mrpack_jobs(
    index: &mrpack::ModpackIndex,
    game_dir: &std::path::Path,
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
        jobs.push(DownloadJob {
            url: url.to_string(),
            dest: game_dir.join(file.safe_path()?),
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
    req: &PackInstallRequest,
    manifest: &mrpack::CurseManifest,
) -> Result<(Vec<DownloadJob>, usize, Vec<String>)> {
    let key = curseforge::require_key(req.api_key.as_deref())?;
    let mods_dir = req.game_dir.join("mods");
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
            dest: mods_dir.join(super::safe_file_name(&file.filename)),
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
        let (jobs, skipped, warnings) = mrpack_jobs(&index(), dir).unwrap();

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
        assert!(mrpack_jobs(&bad, Path::new("C:/mc")).is_err());
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
        let version = super::super::resolve_version(
            &client,
            PackSource::Modrinth,
            "opti-mized",
            None,
            None,
        )
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
        download::run_batch(&client, vec![job], 1, &crate::types::NullSink, StageKind::Libraries)
            .await
            .unwrap();
        assert!(dest.exists());

        let index = mrpack::read_index(&dest).unwrap();
        println!(
            "索引：{} · {} 个文件 · 依赖 {:?}",
            index.name, index.files.len(), index.dependencies
        );
        let mc = index.mc_version().unwrap();
        let loader = index.loader_spec().unwrap();
        println!(
            "要求 Minecraft {} / 加载器 {}",
            mc,
            loader.as_ref().map(|l| l.display()).unwrap_or_else(|| "原版".into())
        );

        let (jobs, skipped, warnings) = mrpack_jobs(&index, &dir).unwrap();
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
