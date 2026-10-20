//! 编排层：把元数据、下载、加载器、Java 串成一次安装。
//!
//! 这是引擎唯一的入口。CLI 和 Tauri 命令层都只调 [`Installer::run`]。

use std::path::PathBuf;
use std::time::Instant;

use crate::download;
use crate::error::{Error, Result};
use crate::java;
use crate::loader::{self, LoaderCtx};
use crate::meta;
use crate::net::{self, HttpClient};
use crate::paths::GamePaths;
use crate::types::{
    DownloadJob, Env, InstallReport, InstallRequest, LogLevel, Progress, ProgressSink, Stage,
    StageKind, VersionDetail,
};

/// 安装器。内部持有一个复用的 HTTP 客户端。
pub struct Installer {
    client: HttpClient,
    env: Env,
}

impl Installer {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: HttpClient::new()?,
            env: Env::default(),
        })
    }

    /// 跑一次完整安装。
    pub async fn run(
        &self,
        req: &InstallRequest,
        sink: &dyn ProgressSink,
    ) -> Result<InstallReport> {
        let started = Instant::now();
        let paths = GamePaths::new(&req.game_dir);
        paths.prepare()?;

        // 1. 元数据
        self.stage(sink, StageKind::Meta, 1, 4, "读取版本清单");
        let manifest = meta::fetch_manifest(&self.client, req.source).await?;
        let summary = manifest
            .find(&req.mc_version)
            .cloned()
            .ok_or_else(|| Error::VersionNotFound(req.mc_version.clone()))?;
        let detail = meta::fetch_detail(&self.client, req.source, &summary).await?;
        sink.emit(Progress::Log {
            level: LogLevel::Info,
            message: format!("版本 {}（{}）", detail.id, detail.kind),
        });

        // 2. Java
        let java_major = detail
            .java_major
            .unwrap_or_else(|| java::required_major(&detail.id));
        let java_path = self.resolve_java(req, java_major, sink).await?;

        // 3. 文件清单
        let (lib_jobs, native_jobs) = self.plan_libraries(&paths, &detail, req.source);
        let client_job = self.plan_client(&paths, &detail, req.source)?;
        let (asset_jobs, asset_index_job) = if req.skip_assets {
            (Vec::new(), None)
        } else {
            self.plan_assets(&paths, &detail, req.source).await?
        };

        let total_files = lib_jobs.len()
            + native_jobs.len()
            + client_job.iter().count()
            + asset_jobs.len()
            + asset_index_job.iter().count();

        // 4. 依赖库
        self.stage(
            sink,
            StageKind::Libraries,
            lib_jobs.len(),
            total_files,
            "下载依赖库",
        );
        let mut outcome = download::run_batch(
            &self.client,
            lib_jobs,
            req.concurrency,
            sink,
            StageKind::Libraries,
        )
        .await?;

        // 5. 资源索引与资源文件
        if let Some(job) = asset_index_job {
            download::download_one(&self.client, &job).await?;
        }
        if !asset_jobs.is_empty() {
            self.stage(
                sink,
                StageKind::Assets,
                asset_jobs.len(),
                total_files,
                "下载资源文件",
            );
            let assets = download::run_batch(
                &self.client,
                asset_jobs,
                req.concurrency,
                sink,
                StageKind::Assets,
            )
            .await?;
            outcome.downloaded += assets.downloaded;
            outcome.skipped += assets.skipped;
            outcome.bytes += assets.bytes;
        }

        // 6. 游戏本体
        if let Some(job) = client_job {
            self.stage(sink, StageKind::Client, 1, total_files, "下载游戏本体");
            download::download_one(&self.client, &job).await?;
        }

        // 7. natives（1.19 以前的老版本需要解压到版本目录）
        for job in &native_jobs {
            download::download_one(&self.client, job).await?;
        }
        self.extract_natives(&paths, &detail)?;

        // 8. 加载器
        let mut installed_version = detail.id.clone();
        if let Some(spec) = &req.loader {
            self.stage(sink, StageKind::Loader, 1, total_files, "安装模组加载器");
            let ctx = LoaderCtx {
                client: &self.client,
                paths: &paths,
                java: &java_path,
                source: req.source,
            };
            let result = loader::install(&ctx, spec, sink).await?;
            if !result.version_id.is_empty() {
                installed_version = result.version_id;
            }
        }

        // 9. 收尾：把解析后的版本 JSON 落盘，方便其它启动器直接识别
        self.write_version_json(&paths, &detail)?;

        let report = InstallReport {
            version_id: detail.id.clone(),
            game_dir: req.game_dir.clone(),
            files: outcome.downloaded,
            bytes: outcome.bytes,
            duration_ms: started.elapsed().as_millis() as u64,
            java: Some(java_path),
            installed_version,
        };
        sink.emit(Progress::Log {
            level: LogLevel::Info,
            message: format!(
                "完成：新增 {} 个文件，共 {:.1} MB，用时 {:.1}s",
                report.files,
                report.bytes as f64 / 1_048_576.0,
                report.duration_ms as f64 / 1000.0
            ),
        });
        Ok(report)
    }

    /// 解析要用的 Java：显式指定 > 本机探测 > 自动下载。
    async fn resolve_java(
        &self,
        req: &InstallRequest,
        major: u32,
        sink: &dyn ProgressSink,
    ) -> Result<PathBuf> {
        if let Some(path) = &req.java {
            if path.exists() {
                return Ok(path.clone());
            }
            return Err(Error::other(format!(
                "指定的 Java 不存在：{}",
                path.display()
            )));
        }

        let found = java::detect();
        if let Some(rt) = java::pick(&found, major) {
            sink.emit(Progress::Log {
                level: LogLevel::Info,
                message: format!("使用本机 Java {}（{}）", rt.major, rt.vendor),
            });
            return Ok(rt.path.clone());
        }

        self.stage(sink, StageKind::Java, 1, 1, "获取 Java 运行时");
        let dir = GamePaths::new(&req.game_dir).root().join("runtime");
        java::ensure(&self.client, major, &dir, sink).await
    }

    /// 组装依赖库与 natives 的下载任务。
    fn plan_libraries(
        &self,
        paths: &GamePaths,
        detail: &VersionDetail,
        source: crate::types::Source,
    ) -> (Vec<DownloadJob>, Vec<DownloadJob>) {
        let mut libs = Vec::new();
        let mut natives = Vec::new();

        for lib in &detail.libraries {
            if !meta::rules_allow(&lib.rules, &self.env) {
                continue;
            }

            if let Some(artifact) = &lib.downloads.artifact {
                let path = artifact
                    .path
                    .clone()
                    .unwrap_or_else(|| maven_to_path(&lib.name));
                let dest = paths.libraries().join(&path);
                let url = if artifact.url.is_empty() {
                    net::library_url(source, &path)
                } else {
                    artifact.url.clone()
                };
                libs.push(DownloadJob {
                    url,
                    dest,
                    sha1: artifact.sha1.clone(),
                    size: artifact.size,
                    label: path,
                });
            }

            // 老版本：按 natives 表挑 classifier
            if let Some(native_table) = &lib.natives {
                if let Some(classifier) = native_table.get(&self.env.os) {
                    let key = classifier.replace("${arch}", arch_bits(&self.env.arch));
                    if let Some(art) = lib.downloads.classifiers.get(&key) {
                        let path = art
                            .path
                            .clone()
                            .unwrap_or_else(|| maven_to_path(&lib.name));
                        let dest = paths.libraries().join(&path);
                        natives.push(DownloadJob {
                            url: art.url.clone(),
                            dest,
                            sha1: art.sha1.clone(),
                            size: art.size,
                            label: path,
                        });
                    }
                }
            }
        }

        (libs, natives)
    }

    fn plan_client(
        &self,
        paths: &GamePaths,
        detail: &VersionDetail,
        _source: crate::types::Source,
    ) -> Result<Option<DownloadJob>> {
        Ok(detail.client.as_ref().map(|c| DownloadJob {
            url: c.url.clone(),
            dest: paths.version_jar(&detail.id),
            sha1: c.sha1.clone(),
            size: c.size,
            label: format!("{}.jar", detail.id),
        }))
    }

    async fn plan_assets(
        &self,
        paths: &GamePaths,
        detail: &VersionDetail,
        source: crate::types::Source,
    ) -> Result<(Vec<DownloadJob>, Option<DownloadJob>)> {
        let Some(index_ref) = &detail.asset_index else {
            return Ok((Vec::new(), None));
        };

        let index_dest = paths
            .asset_indexes()
            .join(format!("{}.json", index_ref.id));
        let index_job = DownloadJob {
            url: index_ref.url.clone(),
            dest: index_dest.clone(),
            sha1: index_ref.sha1.clone(),
            size: index_ref.size,
            label: format!("indexes/{}.json", index_ref.id),
        };

        if !index_dest.exists() {
            download::download_one(&self.client, &index_job).await?;
        }

        let raw = std::fs::read(&index_dest).map_err(|e| Error::io(&index_dest, e))?;
        let index: crate::types::AssetIndex = serde_json::from_slice(&raw)?;

        let mut jobs = Vec::with_capacity(index.objects.len());
        for (name, obj) in &index.objects {
            jobs.push(DownloadJob {
                url: net::asset_url(source, &obj.hash),
                dest: paths.asset_object(&obj.hash),
                sha1: Some(obj.hash.clone()),
                size: Some(obj.size),
                label: name.clone(),
            });
        }
        Ok((jobs, Some(index_job)))
    }

    fn extract_natives(&self, paths: &GamePaths, detail: &VersionDetail) -> Result<()> {
        let dest = paths.natives_dir(&detail.id);
        for lib in &detail.libraries {
            if !meta::rules_allow(&lib.rules, &self.env) {
                continue;
            }
            let Some(natives) = &lib.natives else { continue };
            let Some(classifier) = natives.get(&self.env.os) else {
                continue;
            };
            let key = classifier.replace("${arch}", arch_bits(&self.env.arch));
            let Some(art) = lib.downloads.classifiers.get(&key) else {
                continue;
            };
            let path = art
                .path
                .clone()
                .unwrap_or_else(|| maven_to_path(&lib.name));
            let jar = paths.libraries().join(path);
            let exclude = lib
                .extract
                .as_ref()
                .map(|e| e.exclude.clone())
                .unwrap_or_default();
            download::extract_natives(&jar, &dest, &exclude)?;
        }
        Ok(())
    }

    /// 把合并后的版本 JSON 写回 `versions/<id>/<id>.json`，
    /// 这样官方启动器、PCL 之类也能直接认这个实例。
    fn write_version_json(&self, paths: &GamePaths, detail: &VersionDetail) -> Result<()> {
        let dir = paths.version_dir(&detail.id);
        std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
        let dest = paths.version_json(&detail.id);
        let raw = serde_json::json!({
            "id": detail.id,
            "type": detail.kind,
            "mainClass": detail.main_class,
            "assets": detail.assets,
            "assetIndex": detail.asset_index.as_ref().map(|a| serde_json::json!({
                "id": a.id, "url": a.url, "sha1": a.sha1, "size": a.size
            })),
            "downloads": { "client": detail.client.as_ref().map(|c| serde_json::json!({
                "url": c.url, "sha1": c.sha1, "size": c.size
            })) },
            "libraries": detail.libraries.iter().map(|l| serde_json::json!({
                "name": l.name,
                "downloads": l.downloads,
                "natives": l.natives,
                "rules": l.rules,
                "extract": l.extract,
            })).collect::<Vec<_>>(),
            "javaVersion": detail.java_major.map(|m| serde_json::json!({ "majorVersion": m })),
        });
        std::fs::write(&dest, serde_json::to_vec_pretty(&raw)?)
            .map_err(|e| Error::io(&dest, e))?;
        Ok(())
    }

    fn stage(&self, sink: &dyn ProgressSink, kind: StageKind, files: usize, total: usize, label: &str) {
        sink.emit(Progress::Stage {
            kind,
            index: 0,
            total,
            label: label.to_string(),
        });
        let _ = files;
        let _: Option<Stage> = None;
    }
}

/// `net.minecraft:client:1.21.1` → `net/minecraft/client/1.21.1/client-1.21.1.jar`
pub fn maven_to_path(name: &str) -> String {
    let mut parts = name.split(':');
    let group = parts.next().unwrap_or_default().replace('.', "/");
    let artifact = parts.next().unwrap_or_default();
    let version = parts.next().unwrap_or_default();
    let classifier = parts.next();
    let file = match classifier {
        Some(c) => format!("{artifact}-{version}-{c}.jar"),
        None => format!("{artifact}-{version}.jar"),
    };
    format!("{group}/{artifact}/{version}/{file}")
}

fn arch_bits(arch: &str) -> &'static str {
    match arch {
        "x86" | "i686" => "32",
        _ => "64",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maven_coordinates_become_library_path() {
        assert_eq!(
            maven_to_path("net.minecraft:client:1.21.1"),
            "net/minecraft/client/1.21.1/client-1.21.1.jar"
        );
        assert_eq!(
            maven_to_path("org.lwjgl:lwjgl:3.3.3:natives-windows"),
            "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-windows.jar"
        );
    }

    #[test]
    fn arch_suffix_follows_os_arch() {
        assert_eq!(arch_bits("x86_64"), "64");
        assert_eq!(arch_bits("aarch64"), "64");
        assert_eq!(arch_bits("x86"), "32");
    }
}
