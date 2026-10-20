//! 编排层：把元数据、下载、加载器、Java 串成一次安装。
//!
//! 这是引擎唯一的入口。CLI 和 Tauri 命令层都只调 [`Installer::run`]。

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::download;
use crate::error::{Error, Result};
use crate::java;
use crate::loader::{self, LoaderCtx};
use crate::meta;
use crate::net::{self, HttpClient};
use crate::paths::GamePaths;
use crate::types::{
    AssetIndex, DownloadJob, Env, InstallReport, InstallRequest, JavaRuntime, LogLevel, Progress,
    ProgressSink, Source, StageKind, VersionDetail,
};

/// 老版本的资源目录布局。1.8 之后都是 hash 分桶，不需要额外铺开。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssetLayout {
    /// 现代版本：只有 `assets/objects`
    Plain,
    /// 1.7.x：`assets/virtual/<索引名>/<资源名>`
    Virtual,
    /// 1.6 及更早：`<gameDir>/resources/<资源名>`
    Resources,
}

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
    ///
    /// 阶段序列在第一个 [`Progress::Stage`] 事件之前就确定，UI 可以直接显示
    /// 「第几步 / 共几步」。唯一的例外是：版本 JSON 要求的 Java 比按版本号
    /// 预估的更高时，会在中途补插一个 `Java` 阶段，此时 `total` 加一。
    pub async fn run(
        &self,
        req: &InstallRequest,
        sink: &dyn ProgressSink,
    ) -> Result<InstallReport> {
        let started = Instant::now();
        let paths = GamePaths::new(&req.game_dir);
        paths.prepare()?;

        // 探测一次，规划阶段和解析 Java 都复用这份结果（D7：原来跑两遍，多花 1s 且日志重复）
        let detected = java::detect();
        let mut plan = Self::plan_stages(req, &detected);

        // 1. 元数据
        self.begin(sink, &plan, StageKind::Meta, "读取版本清单");
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
        let java_path = self
            .resolve_java(req, java_major, sink, &mut plan, &detected)
            .await?;

        // 3. 文件清单
        let (lib_jobs, native_jobs) = self.plan_libraries(&paths, &detail, req.source);
        let client_job = self.plan_client(&paths, &detail, req.source)?;
        let (asset_jobs, asset_index_job, index_bytes, asset_layout) = if req.skip_assets {
            (Vec::new(), None, 0, AssetLayout::Plain)
        } else {
            self.plan_assets(&paths, &detail, req.source).await?
        };

        // 4. 依赖库
        self.begin(sink, &plan, StageKind::Libraries, "下载依赖库");
        let mut outcome = download::run_batch(
            &self.client,
            lib_jobs,
            req.concurrency,
            sink,
            StageKind::Libraries,
        )
        .await?;

        // 5. 资源索引与资源文件
        // 索引在 plan_assets 里就已经下好了，这里只记账（D4a：以前这笔字节从来没进过 report）。
        outcome.bytes += index_bytes;
        if index_bytes > 0 {
            outcome.downloaded += 1;
        }
        let _ = asset_index_job;
        if !asset_jobs.is_empty() {
            self.begin(sink, &plan, StageKind::Assets, "下载资源文件");
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

        // 5b. 老版本的虚拟资产布局（1.6 及更早 / 1.7.x）
        if asset_layout != AssetLayout::Plain {
            self.materialize_assets(&paths, &detail, asset_layout)?;
        }

        // 6. 游戏本体
        if let Some(job) = client_job {
            self.begin(sink, &plan, StageKind::Client, "下载游戏本体");
            let added = self.download_counted(&job).await?;
            outcome.bytes += added;
            if added > 0 {
                outcome.downloaded += 1;
            }
        }

        // 7. natives（1.19 以前的老版本需要解压到版本目录）
        for job in &native_jobs {
            let added = self.download_counted(job).await?;
            outcome.bytes += added;
            if added > 0 {
                outcome.downloaded += 1;
            }
        }
        self.extract_natives(&paths, &detail)?;

        // 8. 加载器
        let mut installed_version = detail.id.clone();
        if let Some(spec) = &req.loader {
            self.begin(sink, &plan, StageKind::Loader, "安装模组加载器");
            // 官方 installer 自己会往 libraries/ 和 versions/ 里下载库（Forge 约 96 MB），
            // 这部分不经过我们的下载层，只能靠目录快照差分记账（D8）。
            let watched: [&Path; 2] = [&paths.libraries(), &paths.versions()];
            let before = tree_stats(&watched);
            let ctx = LoaderCtx {
                client: &self.client,
                paths: &paths,
                java: &java_path,
                source: req.source,
            };
            let result = loader::install(&ctx, spec, sink).await?;
            let after = tree_stats(&watched);
            outcome.downloaded += after.1.saturating_sub(before.1) as usize;
            outcome.bytes += after.0.saturating_sub(before.0);
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

    // ------------------------------------------------------------ 阶段

    /// 计划本次安装要跑哪些阶段。在发出第一个阶段事件之前调用。
    fn plan_stages(req: &InstallRequest, detected: &[JavaRuntime]) -> Vec<StageKind> {
        let mut plan = vec![StageKind::Meta];
        if Self::needs_java_download(req, detected) {
            plan.push(StageKind::Java);
        }
        plan.push(StageKind::Libraries);
        if !req.skip_assets {
            plan.push(StageKind::Assets);
        }
        plan.push(StageKind::Client);
        if req.loader.is_some() {
            plan.push(StageKind::Loader);
        }
        plan
    }

    /// 本机有没有能用的 Java：没有才需要下载，才需要 `Java` 阶段。
    ///
    /// 这里只能按版本号推断，因为还没拿到版本 JSON；真正的判断在
    /// [`Self::resolve_java`] 里，推断偏低时会补插阶段。
    fn needs_java_download(req: &InstallRequest, detected: &[JavaRuntime]) -> bool {
        if req.java.is_some() {
            return false;
        }
        let major = java::required_major(&req.mc_version);
        java::pick(detected, major).is_none()
    }

    /// 发出阶段开始事件。`index` 是 1 起的真实序号，`total` 是计划总数。
    fn begin(&self, sink: &dyn ProgressSink, plan: &[StageKind], kind: StageKind, label: &str) {
        let planned = plan.iter().position(|k| *k == kind).map(|i| i + 1);
        let index = planned.unwrap_or(plan.len() + 1);
        let total = plan.len().max(index);
        sink.emit(Progress::Stage {
            kind,
            index,
            total,
            label: label.to_string(),
        });
    }

    // ------------------------------------------------------------ Java

    /// 解析要用的 Java：显式指定 > 本机探测 > 自动下载。
    ///
    /// `plan` 是 `&mut`：`plan_stages` 只能按版本号字符串预估 Java 需求，
    /// 而版本 JSON 里的 `javaVersion` 才是准的（例如 26.3 要 Java 25，按字符串只推得出 21）。
    /// 预估偏低时，把 `Java` 阶段插到 `Meta` 之后——因为此刻正好就停在那个位置，
    /// 这样后续阶段的序号会整体后移一位，UI 看到的序号始终是单调递增的。
    async fn resolve_java(
        &self,
        req: &InstallRequest,
        major: u32,
        sink: &dyn ProgressSink,
        plan: &mut Vec<StageKind>,
        detected: &[JavaRuntime],
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

        if let Some(rt) = java::pick(detected, major) {
            // 计划里有这个阶段就照常发一次，避免阶段轨道上留一个空号。
            if plan.contains(&StageKind::Java) {
                self.begin(sink, plan, StageKind::Java, "检查 Java 运行时");
            }
            sink.emit(Progress::Log {
                level: LogLevel::Info,
                message: format!("使用本机 Java {}（{}）", rt.major, rt.vendor),
            });
            return Ok(rt.path.clone());
        }

        // 本机没有能用的：预估漏了就补进计划（紧跟在 Meta 后面），然后下载。
        if !plan.contains(&StageKind::Java) {
            plan.insert(1, StageKind::Java);
        }
        self.begin(sink, plan, StageKind::Java, "获取 Java 运行时");
        let dir = GamePaths::new(&req.game_dir).root().join("runtime");
        java::ensure(&self.client, major, &dir, sink).await
    }

    // ------------------------------------------------------------ 文件清单

    /// 下载单个文件，返回这次真正落盘的字节数，用来补全 [`InstallReport::bytes`]。
    ///
    /// `download_one` 只返回 `()`，所以这里用下载前后的文件大小差来算：
    /// 已存在且校验通过的文件差值为 0，重下的文件按新大小计。
    async fn download_counted(&self, job: &DownloadJob) -> Result<u64> {
        let before = std::fs::metadata(&job.dest)
            .ok()
            .map(|m| (m.len(), m.modified().ok()));
        download::download_one(&self.client, job).await?;
        let after = std::fs::metadata(&job.dest)
            .ok()
            .map(|m| (m.len(), m.modified().ok()));

        match (before, after) {
            // 原本不存在：这次写进去的全算
            (None, Some((len, _))) => Ok(len),
            // 存在但被改写（同尺寸修复也会更新 mtime）：按新大小算，别报 0
            (Some((len, t0)), Some((new_len, t1))) => {
                if t0 != t1 {
                    Ok(new_len.max(len))
                } else {
                    Ok(0)
                }
            }
            _ => Ok(0),
        }
    }

    /// 组装依赖库与 natives 的下载任务。
    fn plan_libraries(
        &self,
        paths: &GamePaths,
        detail: &VersionDetail,
        source: Source,
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
                // 选了镜像就一律走镜像：原版 JSON 里每个 artifact 都自带官方 url，
                // 只在官方源时才用它。
                let url = match source {
                    Source::Bmclapi => net::library_url(source, &path),
                    Source::Official if artifact.url.is_empty() => net::library_url(source, &path),
                    Source::Official => artifact.url.clone(),
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
                        let path = art.path.clone().unwrap_or_else(|| maven_to_path(&lib.name));
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
        source: Source,
    ) -> Result<Option<DownloadJob>> {
        Ok(detail.client.as_ref().map(|c| DownloadJob {
            url: net::client_url(source, &detail.id, &c.url),
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
        source: Source,
    ) -> Result<(Vec<DownloadJob>, Option<DownloadJob>, u64, AssetLayout)> {
        let Some(index_ref) = &detail.asset_index else {
            return Ok((Vec::new(), None, 0, AssetLayout::Plain));
        };

        let index_dest = paths.asset_indexes().join(format!("{}.json", index_ref.id));
        let index_job = DownloadJob {
            url: index_ref.url.clone(),
            dest: index_dest.clone(),
            sha1: index_ref.sha1.clone(),
            size: index_ref.size,
            label: format!("indexes/{}.json", index_ref.id),
        };

        // 索引本身就是一次真实下载，字节必须记账（D4a：以前这笔从来没进过 report）
        let index_bytes = self.download_counted(&index_job).await?;

        let raw = std::fs::read(&index_dest).map_err(|e| Error::io(&index_dest, e))?;
        let index: AssetIndex = serde_json::from_slice(&raw)?;

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

        let layout = if index.map_to_resources {
            AssetLayout::Resources
        } else if index.virtual_ {
            AssetLayout::Virtual
        } else {
            AssetLayout::Plain
        };

        Ok((jobs, Some(index_job), index_bytes, layout))
    }

    /// 老版本按「资源名」读文件，不认 hash 分桶布局，所以要把 objects 再铺开一份。
    ///
    /// - 1.6 及更早（`map_to_resources`）：铺到 `<gameDir>/resources/<资源名>`
    /// - 1.7.x（`virtual`）：铺到 `assets/virtual/<索引名>/<资源名>`
    fn materialize_assets(
        &self,
        paths: &GamePaths,
        detail: &VersionDetail,
        layout: AssetLayout,
    ) -> Result<()> {
        let Some(index_ref) = &detail.asset_index else {
            return Ok(());
        };
        let index_path = paths.asset_indexes().join(format!("{}.json", index_ref.id));
        let raw = std::fs::read(&index_path).map_err(|e| Error::io(&index_path, e))?;
        let index: AssetIndex = serde_json::from_slice(&raw)?;

        let target = match layout {
            AssetLayout::Resources => paths.root().join("resources"),
            AssetLayout::Virtual => paths.assets().join("virtual").join(&index_ref.id),
            AssetLayout::Plain => return Ok(()),
        };
        std::fs::create_dir_all(&target).map_err(|e| Error::io(&target, e))?;

        let mut copied = 0usize;
        for (name, obj) in &index.objects {
            let src = paths.asset_object(&obj.hash);
            let dst = target.join(name);
            if dst.exists() {
                continue;
            }
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
            }
            match std::fs::copy(&src, &dst) {
                Ok(_) => copied += 1,
                Err(e) => tracing::warn!(
                    path = %src.display(),
                    error = %e,
                    "虚拟资产复制失败，跳过这一个"
                ),
            }
        }
        tracing::info!(
            copied,
            target = %target.display(),
            "已铺开老版本的虚拟资产"
        );
        Ok(())
    }

    fn extract_natives(&self, paths: &GamePaths, detail: &VersionDetail) -> Result<()> {
        let dest = paths.natives_dir(&detail.id);
        for lib in &detail.libraries {
            if !meta::rules_allow(&lib.rules, &self.env) {
                continue;
            }
            let Some(natives) = &lib.natives else {
                continue;
            };
            let Some(classifier) = natives.get(&self.env.os) else {
                continue;
            };
            let key = classifier.replace("${arch}", arch_bits(&self.env.arch));
            let Some(art) = lib.downloads.classifiers.get(&key) else {
                continue;
            };
            let path = art.path.clone().unwrap_or_else(|| maven_to_path(&lib.name));
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
    ///
    /// 必须是**可启动**的完整 JSON：`arguments` / `minecraftArguments` / `logging`
    /// 一个都不能少，否则别的启动器拿过去只会看到一堆库却不知道主类怎么调（D2）。
    fn write_version_json(&self, paths: &GamePaths, detail: &VersionDetail) -> Result<()> {
        let dir = paths.version_dir(&detail.id);
        std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
        let dest = paths.version_json(&detail.id);

        let mut root = serde_json::Map::new();
        root.insert("id".into(), detail.id.clone().into());
        root.insert("type".into(), detail.kind.clone().into());
        root.insert("mainClass".into(), detail.main_class.clone().into());
        root.insert("assets".into(), detail.assets.clone().into());

        if let Some(a) = &detail.asset_index {
            root.insert(
                "assetIndex".into(),
                serde_json::json!({
                    "id": a.id,
                    "url": a.url,
                    "sha1": a.sha1,
                    "size": a.size,
                    "totalSize": a.total_size,
                }),
            );
        }

        if let Some(c) = &detail.client {
            root.insert(
                "downloads".into(),
                serde_json::json!({ "client": {
                    "url": c.url,
                    "sha1": c.sha1,
                    "size": c.size,
                }}),
            );
        }

        // 1.13+ 用 arguments 对象；更早是整串 minecraftArguments，两者不能混写。
        if !detail.game_args.is_empty() || !detail.jvm_args.is_empty() {
            root.insert(
                "arguments".into(),
                serde_json::json!({
                    "game": detail.game_args,
                    "jvm": detail.jvm_args,
                }),
            );
        }
        if let Some(legacy) = &detail.legacy_args {
            root.insert("minecraftArguments".into(), legacy.clone().into());
        }

        if let Some(logging) = &detail.logging {
            root.insert(
                "logging".into(),
                serde_json::json!({ "client": {
                    "argument": logging.argument,
                    "file": {
                        "id": logging.file.path,
                        "url": logging.file.url,
                        "sha1": logging.file.sha1,
                        "size": logging.file.size,
                    },
                }}),
            );
        }

        root.insert("libraries".into(), serde_json::to_value(&detail.libraries)?);
        if let Some(major) = detail.java_major {
            root.insert(
                "javaVersion".into(),
                serde_json::json!({ "majorVersion": major }),
            );
        }

        std::fs::write(&dest, serde_json::to_vec_pretty(&root)?)
            .map_err(|e| Error::io(&dest, e))?;
        Ok(())
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

/// 递归统计若干目录的 (字节数, 文件数)。目录不存在时按 0 计。
///
/// 只用来给加载器阶段做前后差分，不需要精确到符号链接之类的边角。
fn tree_stats(dirs: &[&Path]) -> (u64, u64) {
    let mut bytes = 0u64;
    let mut files = 0u64;
    let mut stack: Vec<PathBuf> = dirs.iter().map(|d| d.to_path_buf()).collect();

    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                bytes += meta.len();
                files += 1;
            }
        }
    }

    (bytes, files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::LoaderSpec;

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

    #[test]
    fn plan_skips_optional_stages() {
        let req = InstallRequest {
            java: Some(PathBuf::from("java")),
            skip_assets: true,
            ..InstallRequest::new(".", "1.21.1")
        };
        let plan = Installer::plan_stages(&req, &[]);
        assert!(!plan.contains(&StageKind::Assets));
        assert!(!plan.contains(&StageKind::Loader));
        assert!(plan.contains(&StageKind::Meta));
        assert!(plan.contains(&StageKind::Client));
    }

    #[test]
    fn plan_includes_loader_when_requested() {
        let mut req = InstallRequest::new(".", "1.21.1");
        req.java = Some(PathBuf::from("java"));
        req.loader = Some(LoaderSpec::new(crate::types::LoaderKind::Fabric, "1.21.1"));
        let plan = Installer::plan_stages(&req, &[]);
        assert_eq!(plan.last(), Some(&StageKind::Loader));
    }

    #[test]
    fn plan_puts_java_right_after_meta() {
        // 本机一个 Java 都没有时必须计划 Java 阶段，而且它只能排在 Meta 之后：
        // 中途补插时 `resolve_java` 也插在这个位置，序号才不会倒退。
        let req = InstallRequest::new(".", "1.21.1");
        let plan = Installer::plan_stages(&req, &[]);
        assert_eq!(plan.first(), Some(&StageKind::Meta));
        assert_eq!(plan.get(1), Some(&StageKind::Java));
    }
}
