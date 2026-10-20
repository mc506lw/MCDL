//! 模组加载器安装。
//!
//! 一律调用官方 installer 的 headless 模式，不自己实现 Forge 的 processors：
//!
//! - Fabric / Quilt：`java -jar fabric-installer.jar client -dir <gameDir> -mcversion <v> -loader <v>`
//! - Forge / NeoForge：`java -jar forge-installer.jar --installClient`
//!
//! 好处是许可证干净（调用二进制不传染 LGPL）、行为与官方一致；
//! 代价是进度粒度只能靠解析 installer 的 stdout。
//!
//! 这里只依赖 installer 的**命令行契约**（各 installer 的 `--help`），不读它们的源码。

use std::collections::HashSet;
use std::ffi::OsString;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;
use tokio::sync::mpsc::UnboundedSender;

use crate::error::{Error, Result};
use crate::java::hide_console;
use crate::net::HttpClient;
use crate::paths::GamePaths;
use crate::types::{LoaderKind, LoaderSpec, LogLevel, Progress, ProgressSink, Source, StageKind};

/// 安装加载器需要的外部环境。
pub struct LoaderCtx<'a> {
    pub client: &'a HttpClient,
    pub paths: &'a GamePaths,
    pub java: &'a Path,
    pub source: Source,
}

#[derive(Debug, Clone, Default)]
pub struct LoaderResult {
    /// 装完后 `versions/` 下新增的版本目录名，例如 `1.21.1-forge-51.0.33`
    pub version_id: String,
    /// installer 写出的 JSON 里的主类（可空，启动器可自行读 JSON）
    pub main_class: Option<String>,
    /// installer 新增/改动的文件
    pub files: Vec<PathBuf>,
    /// installer 的完整输出，出错时原样带回给用户
    pub log: String,
}

const FABRIC_META: &str = "https://meta.fabricmc.net/v2";
const QUILT_META: &str = "https://meta.quiltmc.org/v3";
const FORGE_MAVEN: &str = "https://maven.minecraftforge.net";
const NEOFORGE_MAVEN: &str = "https://maven.neoforged.net/releases";
/// BMCLAPI 的 maven 透传前缀：用户显式选了镜像时，Forge 系从这里下。
const BMCLAPI_MAVEN: &str = "https://bmclapi2.bangbang93.com/maven";

/// Fabric installer 自身的版本：meta 接口挂了也能装（这是唯一的内置常量）。
const FABRIC_INSTALLER_FALLBACK: (&str, &str) = (
    "1.1.2",
    "https://maven.fabricmc.net/net/fabricmc/fabric-installer/1.1.2/fabric-installer-1.1.2.jar",
);
/// Quilt installer 的兜底版本。
const QUILT_INSTALLER_FALLBACK: (&str, &str) = (
    "0.15.1",
    "https://maven.quiltmc.org/repository/release/org/quiltmc/quilt-installer/0.15.1/quilt-installer-0.15.1.jar",
);

/// 安装一个加载器。
pub async fn install(
    ctx: &LoaderCtx<'_>,
    spec: &LoaderSpec,
    sink: &dyn ProgressSink,
) -> Result<LoaderResult> {
    // Forge / NeoForge 的 installer 发现没有 launcher_profiles.json 会直接退出。
    // GamePaths::prepare 已经建过，这里再兜一次底（比如版本目录是别的启动器建的）。
    ctx.paths.prepare()?;

    let loader_version = resolve_loader_version(ctx, spec).await?;
    tracing::info!(
        loader = %spec.kind.label(),
        mc = %spec.mc_version,
        version = %loader_version,
        "开始安装模组加载器"
    );
    sink.emit(Progress::Log {
        level: LogLevel::Info,
        message: format!(
            "{} {}（Minecraft {}）",
            spec.kind.label(),
            loader_version,
            spec.mc_version
        ),
    });

    let cache = ctx.paths.root().join(".mcdl").join("installers");
    let jar = fetch_installer_with(
        ctx.client,
        spec.kind,
        &spec.mc_version,
        &loader_version,
        &cache,
        ctx.source,
        sink,
    )
    .await?;

    // installer 的 cwd 是游戏目录，所以给它的路径必须是绝对的：
    // 传 `./tmp-fabric` 这种相对路径会被解析成 `<gameDir>/tmp-fabric`。
    let game_dir = absolute(ctx.paths.root());
    let jar = absolute(&jar);

    // installer 自己会往 versions/ 里写目录，装完用差集找出它写了哪个。
    let before: HashSet<String> = version_dirs(ctx.paths).into_iter().collect();

    let mut args: Vec<OsString> = jvm_args();
    args.push("-jar".into());
    args.push(jar.clone().into_os_string());
    args.extend(installer_args(
        spec.kind,
        &spec.mc_version,
        &loader_version,
        &game_dir,
        ctx.source,
    ));

    sink.emit(Progress::Log {
        level: LogLevel::Info,
        message: format!("执行：{} {}", ctx.java.display(), display_args(&args)),
    });
    tracing::debug!(
        java = %ctx.java.display(),
        args = %display_args(&args),
        "运行加载器安装器"
    );

    let (status, lines) = run_java(ctx.java, args, &game_dir, sink).await?;
    let log = lines.join("\n");

    if !status.success() {
        let code = status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "被系统终止".to_string());
        let tail_text = tail(&lines, 40);
        // 最后 40 行常常只剩 installer 的收尾语（"There was an error during installation"），
        // 真正的原因（连不上 maven、Java 版本不对）在更前面，单独摘出来。
        let excerpt: Vec<String> = error_excerpt(&lines, 12)
            .into_iter()
            .filter(|l| !tail_text.contains(l.as_str()))
            .collect();
        let excerpt_block = if excerpt.is_empty() {
            String::new()
        } else {
            format!("\n错误摘录：\n{}", excerpt.join("\n"))
        };
        return Err(Error::Loader(format!(
            "{} 安装器失败（退出码 {code}）。\n最后 40 行输出：\n{tail_text}{excerpt_block}{}",
            spec.kind.label(),
            hint_for(spec.kind, &log)
        )));
    }

    let version_id = pick_version_id(
        spec.kind,
        &spec.mc_version,
        &loader_version,
        &before,
        ctx.paths,
    )
    .ok_or_else(|| {
        Error::Loader(format!(
            "{} 安装器报告成功，但 versions/ 下没有出现新的版本目录。\n最后 40 行输出：\n{}",
            spec.kind.label(),
            tail(&lines, 40)
        ))
    })?;

    let main_class = read_main_class(&ctx.paths.version_json(&version_id));
    let files = list_files(&ctx.paths.version_dir(&version_id));
    tracing::info!(
        version = %version_id,
        main_class = main_class.as_deref().unwrap_or("-"),
        files = files.len(),
        "加载器安装完成"
    );

    sink.emit(Progress::Files {
        kind: StageKind::Loader,
        done: 1,
        total: 1,
        bytes_done: 0,
        bytes_total: 0,
    });
    sink.emit(Progress::Log {
        level: LogLevel::Info,
        message: format!("{} 已装好：{version_id}", spec.kind.label()),
    });

    Ok(LoaderResult {
        version_id,
        main_class,
        files,
        log,
    })
}

/// 某个加载器支持的 MC 版本列表，给 UI 做下拉框。
///
/// 这条会被界面在打开面板时直接调用，所以同样带缓存与超时；
/// 但**不截断**：版本选择器要能翻到 1.7.10 那种老版本。
pub async fn supported_mc_versions(client: &HttpClient, kind: LoaderKind) -> Result<Vec<String>> {
    let key = cache_key(&format!("mc|{}", kind.label()), Source::Official);
    if let Some(hit) = mc_cache_get(&key) {
        return Ok(hit);
    }
    let list = with_timeout(
        supported_mc_versions_with(client, kind, Source::Official),
        || format!(" {} 支持的 Minecraft 版本列表", kind.label()),
    )
    .await?;
    mc_cache_put(&key, &list);
    Ok(list)
}

/// 某个 MC 版本可用的加载器版本列表（UI 下拉框用）。
///
/// - 同一个 `(加载器, MC 版本)` 5 分钟内不重复打网络；
/// - 单次查询最多等 10 秒，超时直接报错，不会把界面挂住；
/// - 严格按版本从新到旧，只返回前 [`VERSION_LIST_LIMIT`] 条。
pub async fn available_loader_versions(
    client: &HttpClient,
    kind: LoaderKind,
    mc_version: &str,
) -> Result<Vec<String>> {
    let all = cached_loader_versions(client, kind, mc_version, Source::Official).await?;
    Ok(limit_versions(all.into_iter().map(|v| v.version).collect()))
}

/// UI 下拉框最多要多少条：上千个版本没人翻，渲染还慢。
const VERSION_LIST_LIMIT: usize = 20;

/// 版本列表的进程内缓存有效期。
const VERSION_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(300);

/// 单次版本列表查询的总超时。
const VERSION_QUERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

fn limit_versions(mut list: Vec<String>) -> Vec<String> {
    list.truncate(VERSION_LIST_LIMIT);
    list
}

/// 版本列表查询的统一包装：超时 + 把错误说清成「查不到版本列表」。
///
/// `Error::Loader` 的文案是「安装加载器失败」，用在查询列表上会误导用户，
/// 所以这里统一换成 `Error::Other`（原样展示）。
async fn with_timeout<T, F, Fut>(future: Fut, what: F) -> Result<T>
where
    F: FnOnce() -> String,
    Fut: std::future::Future<Output = Result<T>>,
{
    with_timeout_for(VERSION_QUERY_TIMEOUT, future, what).await
}

async fn with_timeout_for<T, F, Fut>(
    timeout: std::time::Duration,
    future: Fut,
    what: F,
) -> Result<T>
where
    F: FnOnce() -> String,
    Fut: std::future::Future<Output = Result<T>>,
{
    match tokio::time::timeout(timeout, future).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(Error::Other(format!("查不到{}：{e}", what()))),
        Err(_) => Err(Error::Other(format!(
            "查不到{}：请求超时（{} 秒）",
            what(),
            timeout.as_secs().max(1)
        ))),
    }
}

async fn cached_loader_versions(
    client: &HttpClient,
    kind: LoaderKind,
    mc_version: &str,
    source: Source,
) -> Result<Vec<LoaderVersion>> {
    let key = cache_key(&format!("{}|{}", kind.label(), mc_version), source);
    if let Some(hit) = loader_cache_get(&key) {
        tracing::debug!(loader = kind.label(), mc = mc_version, "版本列表命中缓存");
        return Ok(hit);
    }

    let list = with_timeout(loader_versions(client, kind, mc_version, source), || {
        format!(" {} {} 的可用版本", kind.label(), mc_version)
    })
    .await?;
    loader_cache_put(&key, &list);
    Ok(list)
}

fn cache_key(scope: &str, source: Source) -> String {
    format!("{}|{}", source.label(), scope)
}

/// 带 TTL 的进程内缓存。UI 会反复点下拉框，不能每次都打网络。
#[derive(Default)]
struct Cache<T> {
    entries: std::collections::HashMap<String, (std::time::Instant, T)>,
}

impl<T: Clone> Cache<T> {
    fn get(&mut self, key: &str) -> Option<T> {
        match self.entries.get(key) {
            Some((at, value)) if at.elapsed() <= VERSION_CACHE_TTL => Some(value.clone()),
            Some(_) => {
                self.entries.remove(key);
                None
            }
            None => None,
        }
    }

    fn put(&mut self, key: &str, value: &T) {
        // 顺手清理过期项：进程常驻时缓存不能无界增长
        self.entries
            .retain(|_, (at, _)| at.elapsed() <= VERSION_CACHE_TTL);
        self.entries
            .insert(key.to_string(), (std::time::Instant::now(), value.clone()));
    }
}

fn loader_cache() -> &'static std::sync::Mutex<Cache<Vec<LoaderVersion>>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Cache<Vec<LoaderVersion>>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(Cache::default()))
}

fn mc_cache() -> &'static std::sync::Mutex<Cache<Vec<String>>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Cache<Vec<String>>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(Cache::default()))
}

fn loader_cache_get(key: &str) -> Option<Vec<LoaderVersion>> {
    loader_cache().lock().ok()?.get(key)
}

fn loader_cache_put(key: &str, value: &[LoaderVersion]) {
    if let Ok(mut cache) = loader_cache().lock() {
        cache.put(key, &value.to_vec());
    }
}

fn mc_cache_get(key: &str) -> Option<Vec<String>> {
    mc_cache().lock().ok()?.get(key)
}

fn mc_cache_put(key: &str, value: &[String]) {
    if let Ok(mut cache) = mc_cache().lock() {
        cache.put(key, &value.to_vec());
    }
}

/// 下载 installer jar 到临时目录，返回路径。
pub async fn fetch_installer(
    client: &HttpClient,
    kind: LoaderKind,
    spec: &LoaderSpec,
    cache_dir: &Path,
) -> Result<PathBuf> {
    let loader_version = match &spec.loader_version {
        Some(v) => v.trim().to_string(),
        None => {
            let list =
                cached_loader_versions(client, kind, &spec.mc_version, Source::Official).await?;
            pick_loader_version(&list).ok_or_else(|| {
                Error::Loader(format!(
                    "{} 没有支持 Minecraft {} 的版本",
                    kind.label(),
                    spec.mc_version
                ))
            })?
        }
    };
    fetch_installer_with(
        client,
        kind,
        &spec.mc_version,
        &loader_version,
        cache_dir,
        Source::Official,
        &crate::types::NullSink,
    )
    .await
}

// ---------------------------------------------------------------- 版本解析

#[derive(Debug, Clone)]
struct LoaderVersion {
    version: String,
    stable: bool,
}

async fn resolve_loader_version(ctx: &LoaderCtx<'_>, spec: &LoaderSpec) -> Result<String> {
    if let Some(v) = &spec.loader_version {
        let v = v.trim();
        if !v.is_empty() {
            return Ok(v.to_string());
        }
    }
    let list = cached_loader_versions(ctx.client, spec.kind, &spec.mc_version, ctx.source).await?;
    pick_loader_version(&list).ok_or_else(|| {
        Error::Loader(format!(
            "{} 没有支持 Minecraft {} 的版本",
            spec.kind.label(),
            spec.mc_version
        ))
    })
}

/// 各加载器的版本列表（已按从新到旧排好，带 stable 标记）。
async fn loader_versions(
    client: &HttpClient,
    kind: LoaderKind,
    mc_version: &str,
    source: Source,
) -> Result<Vec<LoaderVersion>> {
    let list = match kind {
        LoaderKind::Fabric => {
            #[derive(Deserialize)]
            struct Entry {
                loader: Inner,
            }
            #[derive(Deserialize)]
            struct Inner {
                version: String,
                #[serde(default)]
                stable: bool,
            }
            let url = format!("{FABRIC_META}/versions/loader/{mc_version}");
            let raw: Vec<Entry> = client.get_json(&url).await?;
            raw.into_iter()
                .map(|e| LoaderVersion {
                    version: e.loader.version,
                    stable: e.loader.stable,
                })
                .collect()
        }
        LoaderKind::Quilt => {
            #[derive(Deserialize)]
            struct Entry {
                loader: Inner,
            }
            #[derive(Deserialize)]
            struct Inner {
                version: String,
            }
            let url = format!("{QUILT_META}/versions/loader/{mc_version}");
            let raw: Vec<Entry> = client.get_json(&url).await?;
            raw.into_iter()
                .map(|e| {
                    let stable = !e.loader.version.contains("beta")
                        && !e.loader.version.contains("alpha")
                        && !e.loader.version.contains("rc");
                    LoaderVersion {
                        version: e.loader.version,
                        stable,
                    }
                })
                .collect()
        }
        LoaderKind::Forge => {
            let base = forge_base(source);
            let url = format!("{base}/net/minecraftforge/forge/maven-metadata.xml");
            let xml = client.get_text(&url).await?;
            let prefix = format!("{mc_version}-");
            xml_versions(&xml)
                .into_iter()
                .filter_map(|v| v.strip_prefix(&prefix).map(|s| s.to_string()))
                .map(|version| LoaderVersion {
                    version,
                    stable: true,
                })
                .collect()
        }
        LoaderKind::NeoForge => {
            let base = neoforge_base(source);
            let mut out = Vec::new();
            if mc_at_most(mc_version, 1, 20, 1) {
                // 1.20.1 及更早：NeoForge 还在 net.neoforged:forge 下，版本号是 `<mc>-<ver>`
                let url = format!("{base}/net/neoforged/forge/maven-metadata.xml");
                if let Ok(xml) = client.get_text(&url).await {
                    let prefix = format!("{mc_version}-");
                    out.extend(
                        xml_versions(&xml)
                            .into_iter()
                            .filter(|v| v.starts_with(&prefix))
                            .map(|version| LoaderVersion {
                                version,
                                stable: true,
                            }),
                    );
                }
            } else {
                let url = format!("{base}/net/neoforged/neoforge/maven-metadata.xml");
                let xml = client.get_text(&url).await?;
                let prefix = neoforge_prefix(mc_version);
                out.extend(
                    xml_versions(&xml)
                        .into_iter()
                        .filter(|v| v.starts_with(&prefix))
                        .filter(|v| !v.contains("alpha") && !v.contains("beta"))
                        .map(|version| LoaderVersion {
                            version,
                            stable: true,
                        }),
                );
            }
            out
        }
    };

    Ok(sort_versions_desc(list))
}

async fn supported_mc_versions_with(
    client: &HttpClient,
    kind: LoaderKind,
    source: Source,
) -> Result<Vec<String>> {
    let mut versions: Vec<String> = match kind {
        LoaderKind::Fabric => {
            #[derive(Deserialize)]
            struct Entry {
                version: String,
            }
            let raw: Vec<Entry> = client
                .get_json(&format!("{FABRIC_META}/versions/game"))
                .await?;
            raw.into_iter().map(|e| e.version).collect()
        }
        LoaderKind::Quilt => {
            #[derive(Deserialize)]
            struct Entry {
                version: String,
            }
            let raw: Vec<Entry> = client
                .get_json(&format!("{QUILT_META}/versions/game"))
                .await?;
            raw.into_iter().map(|e| e.version).collect()
        }
        LoaderKind::Forge => {
            let url = format!(
                "{}/net/minecraftforge/forge/maven-metadata.xml",
                forge_base(source)
            );
            let xml = client.get_text(&url).await?;
            xml_versions(&xml)
                .into_iter()
                .filter_map(|v| v.split_once('-').map(|(mc, _)| mc.to_string()))
                .collect()
        }
        LoaderKind::NeoForge => {
            let base = neoforge_base(source);
            let mut out = Vec::new();
            let url = format!("{base}/net/neoforged/neoforge/maven-metadata.xml");
            if let Ok(xml) = client.get_text(&url).await {
                out.extend(
                    xml_versions(&xml)
                        .into_iter()
                        .filter_map(|v| neoforge_mc_of(&v)),
                );
            }
            let url = format!("{base}/net/neoforged/forge/maven-metadata.xml");
            if let Ok(xml) = client.get_text(&url).await {
                out.extend(
                    xml_versions(&xml)
                        .into_iter()
                        .filter_map(|v| v.split_once('-').map(|(mc, _)| mc.to_string())),
                );
            }
            out
        }
    };

    // 去重并按 MC 版本从新到旧
    let mut seen = HashSet::new();
    versions.retain(|v| !v.is_empty() && seen.insert(v.clone()));
    versions.sort_by(|a, b| compare_versions(b, a));
    Ok(versions)
}

fn sort_versions_desc(list: Vec<LoaderVersion>) -> Vec<LoaderVersion> {
    // 严格按版本从新到旧。UI 下拉框直接照这个顺序渲染；
    // 「自动选哪个」是另一件事，见 pick_loader_version。
    let mut list = list;
    list.sort_by(|a, b| compare_versions(&b.version, &a.version));
    list
}

/// 自动挑一个版本装：优先最新的**稳定版**，没有稳定版才用最新的。
///
/// 注意这跟列表顺序（严格从新到旧）是分开的：Fabric 会把最新的 0.20.x
/// 标成 unstable，而 0.19.5 才是 stable，默认装哪个不该由排序决定。
fn pick_loader_version(list: &[LoaderVersion]) -> Option<String> {
    list.iter()
        .find(|v| v.stable)
        .or_else(|| list.first())
        .map(|v| v.version.clone())
}

/// 版本号比较：按 `.` 分段，每段「前导数字 + 后缀」。
///
/// 这样 `52.1.20 > 52.1.2`、`0.24.0 > 0.23.1`，而 `0.20.0-beta.9` 排在 `0.20.0` 之前。
fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    let pa = version_parts(a);
    let pb = version_parts(b);
    for i in 0..pa.len().max(pb.len()) {
        let x = pa.get(i).copied().unwrap_or((0, ""));
        let y = pb.get(i).copied().unwrap_or((0, ""));
        match x.0.cmp(&y.0) {
            Ordering::Equal => match suffix_rank(x.1)
                .cmp(&suffix_rank(y.1))
                .then_with(|| x.1.cmp(y.1))
            {
                Ordering::Equal => continue,
                other => return other,
            },
            other => return other,
        }
    }
    a.cmp(b)
}

fn version_parts(s: &str) -> Vec<(u64, &str)> {
    s.split('.')
        .map(|part| {
            let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
            let num = digits.parse().unwrap_or(0);
            (num, &part[digits.len()..])
        })
        .collect()
}

/// 预发布后缀（`-beta.9`）比正式版小。
fn suffix_rank(suffix: &str) -> u8 {
    let lower = suffix.to_ascii_lowercase();
    if lower.is_empty() {
        1
    } else if ["alpha", "beta", "rc", "pre", "snapshot"]
        .iter()
        .any(|k| lower.contains(k))
    {
        0
    } else {
        1
    }
}

fn forge_base(source: Source) -> &'static str {
    match source {
        Source::Official => FORGE_MAVEN,
        Source::Bmclapi => BMCLAPI_MAVEN,
    }
}

fn neoforge_base(source: Source) -> &'static str {
    match source {
        Source::Official => NEOFORGE_MAVEN,
        Source::Bmclapi => BMCLAPI_MAVEN,
    }
}

/// NeoForge 版本号前缀：MC 1.21.1 → `21.1.`，MC 1.21 → `21.0.`，MC 26.3 → `26.3.`。
fn neoforge_prefix(mc_version: &str) -> String {
    let nums = mc_numbers(mc_version);
    match nums.first().copied().unwrap_or(1) {
        1 => format!(
            "{}.{}.",
            nums.get(1).copied().unwrap_or(0),
            nums.get(2).copied().unwrap_or(0)
        ),
        major => format!("{major}.{}.", nums.get(1).copied().unwrap_or(0)),
    }
}

/// NeoForge 版本号 → 对应的 MC 版本。
fn neoforge_mc_of(version: &str) -> Option<String> {
    // 老命名 `1.20.1-47.1.106`：MC 版本直接写在前面
    if let Some((mc, _)) = version.split_once('-') {
        if mc.starts_with('1') {
            return Some(mc.to_string());
        }
    }
    let nums = mc_numbers(version);
    let major = nums.first().copied()?;
    let minor = nums.get(1).copied().unwrap_or(0);
    if major >= 22 {
        // 26.3.0.94 → 26.3
        Some(format!("{major}.{minor}"))
    } else if minor == 0 {
        // 21.0.167 → 1.21
        Some(format!("1.{major}"))
    } else {
        // 21.1.264 → 1.21.1
        Some(format!("1.{major}.{minor}"))
    }
}

/// 版本号开头那串数字；认不出（快照 `24w14a`）就给空。
fn mc_numbers(raw: &str) -> Vec<u32> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_digit() {
            current.push(ch);
        } else if ch == '.' && !current.is_empty() {
            if let Ok(n) = current.parse() {
                out.push(n);
            }
            current.clear();
        } else {
            break;
        }
    }
    if let Ok(n) = current.parse() {
        out.push(n);
    }
    out
}

/// `mc_version` 是否不高于 `major.minor.patch`。
fn mc_at_most(mc_version: &str, major: u32, minor: u32, patch: u32) -> bool {
    let nums = mc_numbers(mc_version);
    let got = (
        nums.first().copied().unwrap_or(0),
        nums.get(1).copied().unwrap_or(0),
        nums.get(2).copied().unwrap_or(0),
    );
    got <= (major, minor, patch)
}

/// 从 maven-metadata.xml 里抠出所有 `<version>`；不值得为它引一个 XML 解析器。
fn xml_versions(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<version>") {
        rest = &rest[start + "<version>".len()..];
        let Some(end) = rest.find("</version>") else {
            break;
        };
        let v = rest[..end].trim();
        if !v.is_empty() {
            out.push(v.to_string());
        }
        rest = &rest[end + "</version>".len()..];
    }
    out
}

// ---------------------------------------------------------------- installer 定位

async fn fetch_installer_with(
    client: &HttpClient,
    kind: LoaderKind,
    mc_version: &str,
    loader_version: &str,
    cache_dir: &Path,
    source: Source,
    sink: &dyn ProgressSink,
) -> Result<PathBuf> {
    let (url, file_name) =
        installer_location(client, kind, mc_version, loader_version, source).await?;
    std::fs::create_dir_all(cache_dir).map_err(|e| Error::io(cache_dir, e))?;
    let dest = cache_dir.join(&file_name);

    if is_jar(&dest) {
        tracing::debug!(path = %dest.display(), "复用已缓存的 installer");
        return Ok(dest);
    }

    tracing::info!(%url, "下载加载器安装器");
    download_file(client, &url, &dest, sink).await?;
    if !is_jar(&dest) {
        let _ = std::fs::remove_file(&dest);
        return Err(Error::Loader(format!(
            "{url} 下载下来不是可用的 jar（可能被镜像改写了）"
        )));
    }
    Ok(dest)
}

async fn installer_location(
    client: &HttpClient,
    kind: LoaderKind,
    mc_version: &str,
    loader_version: &str,
    source: Source,
) -> Result<(String, String)> {
    match kind {
        LoaderKind::Fabric => {
            // meta 同时给出 installer 的版本号和下载地址；接口挂了就退回内置版本。
            #[derive(Deserialize)]
            struct Entry {
                url: String,
                version: String,
                #[serde(default)]
                stable: bool,
            }
            let entries: Vec<Entry> = client
                .get_json(&format!("{FABRIC_META}/versions/installer"))
                .await
                .unwrap_or_default();
            let picked = entries
                .iter()
                .find(|e| e.stable)
                .or_else(|| entries.first());
            Ok(match picked {
                Some(e) => (e.url.clone(), format!("fabric-installer-{}.jar", e.version)),
                None => (
                    FABRIC_INSTALLER_FALLBACK.1.to_string(),
                    format!("fabric-installer-{}.jar", FABRIC_INSTALLER_FALLBACK.0),
                ),
            })
        }
        LoaderKind::Quilt => {
            #[derive(Deserialize)]
            struct Entry {
                url: String,
                version: String,
            }
            let entries: Vec<Entry> = client
                .get_json(&format!("{QUILT_META}/versions/installer"))
                .await
                .unwrap_or_default();
            Ok(match entries.first() {
                Some(e) => (e.url.clone(), format!("quilt-installer-{}.jar", e.version)),
                None => (
                    QUILT_INSTALLER_FALLBACK.1.to_string(),
                    format!("quilt-installer-{}.jar", QUILT_INSTALLER_FALLBACK.0),
                ),
            })
        }
        LoaderKind::Forge => Ok(forge_installer_location(mc_version, loader_version, source)),
        LoaderKind::NeoForge => Ok(neoforge_installer_location(
            mc_version,
            loader_version,
            source,
        )),
    }
}

/// Forge installer 的地址与文件名。forge 版本写成 `47.2.0` 或 `1.20.1-47.2.0` 都认。
fn forge_installer_location(
    mc_version: &str,
    loader_version: &str,
    source: Source,
) -> (String, String) {
    let full = if loader_version.starts_with(&format!("{mc_version}-")) {
        loader_version.to_string()
    } else {
        format!("{mc_version}-{loader_version}")
    };
    let url = format!(
        "{}/net/minecraftforge/forge/{full}/forge-{full}-installer.jar",
        forge_base(source)
    );
    (url, format!("forge-{full}-installer.jar"))
}

/// NeoForge installer 的地址与文件名。1.20.1 及更早走 `net.neoforged:forge`。
fn neoforge_installer_location(
    mc_version: &str,
    loader_version: &str,
    source: Source,
) -> (String, String) {
    let (group, artifact, version) = neoforge_coords(mc_version, loader_version);
    let url = format!(
        "{}/{group}/{artifact}/{version}/{artifact}-{version}-installer.jar",
        neoforge_base(source)
    );
    (url, format!("{artifact}-{version}-installer.jar"))
}

fn neoforge_coords(mc_version: &str, loader_version: &str) -> (&'static str, &'static str, String) {
    if mc_at_most(mc_version, 1, 20, 1) {
        let version = if loader_version.starts_with(&format!("{mc_version}-")) {
            loader_version.to_string()
        } else {
            format!("{mc_version}-{loader_version}")
        };
        ("net/neoforged", "forge", version)
    } else {
        let version = loader_version
            .strip_prefix(&format!("{mc_version}-"))
            .unwrap_or(loader_version)
            .to_string();
        ("net/neoforged", "neoforge", version)
    }
}

/// 流式下载 installer，边下边抛字节进度。
async fn download_file(
    client: &HttpClient,
    url: &str,
    dest: &Path,
    sink: &dyn ProgressSink,
) -> Result<()> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    let resp = client
        .raw()
        .get(url)
        .send()
        .await
        .map_err(|e| Error::Http(format!("下载 {url} 失败：{e}")))?;
    if !resp.status().is_success() {
        return Err(Error::Http(format!(
            "下载 {url} 失败：HTTP {}",
            resp.status()
        )));
    }

    let total = resp.content_length().unwrap_or(0);
    let part = dest.with_extension("part");
    let mut file = tokio::fs::File::create(&part)
        .await
        .map_err(|e| Error::io(&part, e))?;
    let mut stream = resp.bytes_stream();
    let mut done: u64 = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| Error::Http(format!("下载 {url} 中断：{e}")))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| Error::io(&part, e))?;
        done += chunk.len() as u64;
        sink.emit(Progress::Files {
            kind: StageKind::Loader,
            done: 0,
            total: 0,
            bytes_done: done,
            bytes_total: total,
        });
    }
    file.flush().await.map_err(|e| Error::io(&part, e))?;
    drop(file);
    tokio::fs::rename(&part, dest)
        .await
        .map_err(|e| Error::io(dest, e))?;
    tracing::info!(bytes = done, %url, "installer 下载完成");
    Ok(())
}

/// 转成绝对路径给 installer 用。
///
/// Windows 的 `canonicalize` 会加 `\\?\` 前缀，Java 的 `-jar` 不认这种路径，得剥掉。
fn absolute(path: &Path) -> PathBuf {
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(path))
                .unwrap_or_else(|_| path.to_path_buf())
        }
    });
    let text = resolved.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(stripped) => PathBuf::from(stripped),
        None => resolved,
    }
}

fn is_jar(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 4];
    if f.read_exact(&mut magic).is_err() {
        return false;
    }
    &magic[..2] == b"PK"
}

// ---------------------------------------------------------------- 命令行

/// 所有 installer 都跑在 UTF-8 下：Windows 中文系统默认 GBK，
/// 不强制的话 installer 输出的中文在日志里全是乱码。
fn jvm_args() -> Vec<OsString> {
    [
        "-Dfile.encoding=UTF-8",
        "-Dsun.stdout.encoding=UTF-8",
        "-Dsun.stderr.encoding=UTF-8",
    ]
    .into_iter()
    .map(OsString::from)
    .collect()
}

fn installer_args(
    kind: LoaderKind,
    mc_version: &str,
    loader_version: &str,
    game_dir: &Path,
    source: Source,
) -> Vec<OsString> {
    let dir = game_dir.as_os_str().to_os_string();
    match kind {
        LoaderKind::Fabric => vec![
            "client".into(),
            "-dir".into(),
            dir,
            "-mcversion".into(),
            mc_version.into(),
            "-loader".into(),
            loader_version.into(),
        ],
        LoaderKind::Quilt => vec![
            "install".into(),
            "client".into(),
            mc_version.into(),
            loader_version.into(),
            format!("--install-dir={}", game_dir.display()).into(),
        ],
        LoaderKind::Forge | LoaderKind::NeoForge => {
            let mut args: Vec<OsString> = vec!["--installClient".into(), dir];
            // 只有用户显式选了镜像才加；Forge 系的 installer 支持 --mirror 改写 maven 前缀。
            if source == Source::Bmclapi {
                args.push("--mirror".into());
                args.push(BMCLAPI_MAVEN.into());
            }
            args
        }
    }
}

fn display_args(args: &[OsString]) -> String {
    args.iter()
        .map(|a| {
            let s = a.to_string_lossy();
            if s.contains(' ') {
                format!("\"{s}\"")
            } else {
                s.into_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// 跑 installer，把 stdout / stderr 边读边转成进度事件。
async fn run_java(
    java: &Path,
    args: Vec<OsString>,
    cwd: &Path,
    sink: &dyn ProgressSink,
) -> Result<(std::process::ExitStatus, Vec<String>)> {
    let java_label = java.to_path_buf();
    let java_path = java.to_path_buf();
    let cwd = cwd.to_path_buf();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    // 子进程是阻塞的，扔到 blocking 线程池，别占着 async worker。
    let handle = tokio::task::spawn_blocking(move || spawn_and_pump(java_path, args, cwd, tx));

    let mut lines: Vec<String> = Vec::new();
    let mut shown = 0usize;
    while let Some(line) = rx.recv().await {
        if shown < MAX_SHOWN_LINES && worth_showing(&line) {
            shown += 1;
            sink.emit(Progress::Log {
                level: LogLevel::Info,
                message: line.clone(),
            });
        }
        lines.push(line);
    }

    match handle.await {
        Ok(Ok(status)) => Ok((status, lines)),
        Ok(Err(e)) => Err(Error::Loader(format!(
            "启动 installer 失败（Java：{}）：{e}",
            java_label.display()
        ))),
        Err(e) => Err(Error::Loader(format!("等待 installer 结束时出错：{e}"))),
    }
}

/// installer 的输出太多时不要全部塞进进度事件，日志字段里仍然保留全量。
const MAX_SHOWN_LINES: usize = 400;

fn spawn_and_pump(
    java: PathBuf,
    args: Vec<OsString>,
    cwd: PathBuf,
    tx: UnboundedSender<String>,
) -> std::io::Result<std::process::ExitStatus> {
    let mut cmd = Command::new(&java);
    cmd.args(&args)
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_console(&mut cmd);

    let mut child = cmd.spawn()?;
    let stdout = child.stdout.take().expect("stdout 已设为 piped");
    let stderr = child.stderr.take().expect("stderr 已设为 piped");

    // 两路必须并发读：只读一路的话，另一路管道写满后子进程会永久阻塞在 write 上。
    let tx_err = tx.clone();
    let out_thread = std::thread::spawn(move || pump_lines(stdout, tx));
    let err_thread = std::thread::spawn(move || pump_lines(stderr, tx_err));

    let status = child.wait()?;
    let _ = out_thread.join();
    let _ = err_thread.join();
    Ok(status)
}

fn pump_lines<R: std::io::Read>(reader: R, tx: UnboundedSender<String>) {
    let mut reader = BufReader::new(reader);
    let mut buf: Vec<u8> = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            // 不要求 UTF-8：老 installer 可能吐 GBK，丢整行不如留个能看的近似值
            Ok(0) => break,
            Ok(_) => {
                let text = String::from_utf8_lossy(&buf);
                let line = text.trim_end_matches(['\r', '\n']).to_string();
                if tx.send(line).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

/// Forge 每处理一个 class 就缩进输出一行，这种噪声不往界面上送。
fn worth_showing(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty() && trimmed.len() <= 300 && !line.starts_with(' ') && !line.starts_with('\t')
}

fn tail(lines: &[String], n: usize) -> String {
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

/// 从输出里挑出「像错误」的行，补在最后 40 行之外。
///
/// Forge / NeoForge 失败时会先打一堆异常栈，最后才打一句
/// "There was an error during installation"；只看末尾等于什么都没说。
fn error_excerpt(lines: &[String], max: usize) -> Vec<String> {
    let keys = [
        "exception",
        "error",
        "failed",
        "could not",
        "timed out",
        "unable to",
        "denied",
    ];
    let mut out: Vec<String> = Vec::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        if keys.iter().any(|k| lower.contains(k)) && !out.iter().any(|seen| seen == trimmed) {
            out.push(trimmed.to_string());
        }
    }
    if out.len() > max {
        out.drain(..out.len() - max);
    }
    out
}

fn hint_for(kind: LoaderKind, log: &str) -> String {
    let network_trouble = log.contains("SocketTimeoutException")
        || log.contains("Connect timed out")
        || log.contains("UnknownHostException")
        || log.contains("Connection refused")
        || log.contains("Network is unreachable");

    // NeoForge 的 installer 无视 --mirror，只能直连它自己的 maven；这是本机实测结论。
    if kind == LoaderKind::NeoForge && network_trouble {
        return "\n提示：NeoForge 的 installer 必须能直连 maven.neoforged.net（依赖库由它自己下载），\
                BMCLAPI 的 --mirror 对这部分下载无效，换镜像源也救不了。\
                请先确认该域名可达再重试。"
            .to_string();
    }

    if log.contains("UnsupportedClassVersionError") || log.contains("requires Java") {
        "\n提示：这个加载器需要更高版本的 Java。用 --java 指定一个，或者让它自动下载。".to_string()
    } else if log.contains("launcher_profiles") {
        "\n提示：游戏目录里缺 launcher_profiles.json，安装器拒绝继续。".to_string()
    } else if network_trouble {
        "\n提示：安装器连不上它自己的 maven 源（不是本程序的下载器）。\
         检查网络，或改用镜像重试。"
            .to_string()
    } else {
        String::new()
    }
}

// ---------------------------------------------------------------- 结果识别

fn version_dirs(paths: &GamePaths) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(paths.versions()) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect()
}

/// installer 会用的版本目录名。命名规则各加载器不同，也会随版本变，所以只当"提示"用。
fn expected_version_ids(kind: LoaderKind, mc_version: &str, loader_version: &str) -> Vec<String> {
    let short = loader_version
        .strip_prefix(&format!("{mc_version}-"))
        .unwrap_or(loader_version);
    match kind {
        LoaderKind::Fabric => vec![format!("fabric-loader-{loader_version}-{mc_version}")],
        LoaderKind::Quilt => vec![format!("quilt-loader-{loader_version}-{mc_version}")],
        LoaderKind::Forge => vec![format!("{mc_version}-forge-{short}")],
        LoaderKind::NeoForge => vec![
            format!("neoforge-{loader_version}"),
            format!("neoforge-{short}"),
            format!("{mc_version}-neoforge-{short}"),
        ],
    }
}

fn pick_version_id(
    kind: LoaderKind,
    mc_version: &str,
    loader_version: &str,
    before: &HashSet<String>,
    paths: &GamePaths,
) -> Option<String> {
    let expected = expected_version_ids(kind, mc_version, loader_version);
    let after = version_dirs(paths);
    let has_json = |name: &str| paths.version_json(name).is_file();

    // 1) 新增目录里精确命中
    for name in &expected {
        if !before.contains(name) && after.iter().any(|n| n == name) && has_json(name) {
            return Some(name.clone());
        }
    }

    // 2) 新增目录里挑最像加载器的（installer 的命名规则可能变，不能只认死名字）
    let mut fresh: Vec<&String> = after
        .iter()
        .filter(|n| !before.contains(*n) && has_json(n))
        .collect();
    fresh.sort_by_key(|n| version_score(n, kind, mc_version, &expected));
    if let Some(best) = fresh.last() {
        return Some((*best).clone());
    }

    // 3) 同一个组合重复安装时 installer 直接复用旧目录，不产生新增
    for name in &expected {
        if has_json(name) {
            return Some(name.clone());
        }
    }
    None
}

fn version_score(
    name: &str,
    kind: LoaderKind,
    mc_version: &str,
    expected: &[String],
) -> (u8, u8, u8) {
    let exact = u8::from(expected.iter().any(|e| e == name));
    let keyword = match kind {
        LoaderKind::Fabric => "fabric",
        LoaderKind::Quilt => "quilt",
        LoaderKind::Forge => "forge",
        LoaderKind::NeoForge => "neoforge",
    };
    let has_keyword = u8::from(name.to_ascii_lowercase().contains(keyword));
    let has_mc = u8::from(name.contains(mc_version));
    (exact, has_keyword, has_mc)
}

fn read_main_class(json: &Path) -> Option<String> {
    let raw = std::fs::read(json).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&raw).ok()?;
    value
        .get("mainClass")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn list_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mcdl-loader-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn arg_strings(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn fabric_and_quilt_command_lines() {
        let dir = Path::new("C:/mc");
        let fabric = arg_strings(&installer_args(
            LoaderKind::Fabric,
            "1.21.1",
            "0.16.9",
            dir,
            Source::Official,
        ));
        assert_eq!(
            fabric,
            [
                "client",
                "-dir",
                "C:/mc",
                "-mcversion",
                "1.21.1",
                "-loader",
                "0.16.9"
            ]
        );

        let quilt = arg_strings(&installer_args(
            LoaderKind::Quilt,
            "1.21.1",
            "0.27.0",
            dir,
            Source::Official,
        ));
        assert_eq!(
            quilt,
            [
                "install",
                "client",
                "1.21.1",
                "0.27.0",
                "--install-dir=C:/mc"
            ]
        );
    }

    #[test]
    fn forge_and_neoforge_command_lines() {
        let dir = Path::new("C:/mc");
        let forge = arg_strings(&installer_args(
            LoaderKind::Forge,
            "1.21.1",
            "52.1.20",
            dir,
            Source::Official,
        ));
        assert_eq!(forge, ["--installClient", "C:/mc"]);

        // 镜像源要多带 --mirror
        let mirrored = arg_strings(&installer_args(
            LoaderKind::NeoForge,
            "1.21.1",
            "21.1.264",
            dir,
            Source::Bmclapi,
        ));
        assert_eq!(
            mirrored,
            ["--installClient", "C:/mc", "--mirror", BMCLAPI_MAVEN]
        );
    }

    #[test]
    fn installer_urls_follow_maven_layout() {
        let (url, file) = forge_installer_location("1.21.1", "52.1.20", Source::Official);
        assert_eq!(
            url,
            "https://maven.minecraftforge.net/net/minecraftforge/forge/1.21.1-52.1.20/forge-1.21.1-52.1.20-installer.jar"
        );
        assert_eq!(file, "forge-1.21.1-52.1.20-installer.jar");

        // 用户写全 `1.20.1-47.2.0` 时不能再拼一次 MC 版本
        let (url, _) = forge_installer_location("1.20.1", "1.20.1-47.2.0", Source::Official);
        assert!(url.contains("/1.20.1-47.2.0/forge-1.20.1-47.2.0-installer.jar"));

        let (url, file) = neoforge_installer_location("1.21.1", "21.1.264", Source::Official);
        assert_eq!(
            url,
            "https://maven.neoforged.net/releases/net/neoforged/neoforge/21.1.264/neoforge-21.1.264-installer.jar"
        );
        assert_eq!(file, "neoforge-21.1.264-installer.jar");

        // 1.20.1 的 NeoForge 还在 net.neoforged:forge 下
        let (url, _) = neoforge_installer_location("1.20.1", "47.1.106", Source::Official);
        assert_eq!(
            url,
            "https://maven.neoforged.net/releases/net/neoforged/forge/1.20.1-47.1.106/forge-1.20.1-47.1.106-installer.jar"
        );

        // 镜像源换前缀，路径保持 maven 布局
        let (url, _) = neoforge_installer_location("1.21.1", "21.1.264", Source::Bmclapi);
        assert_eq!(
            url,
            "https://bmclapi2.bangbang93.com/maven/net/neoforged/neoforge/21.1.264/neoforge-21.1.264-installer.jar"
        );
    }

    #[test]
    fn parses_maven_metadata_versions() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<metadata>
  <groupId>net.neoforged</groupId>
  <versioning>
    <latest>26.3.0.94</latest>
    <versions>
      <version>1.20.1-47.1.106</version>
      <version>21.1.264</version>
      <version>26.3.0.94</version>
    </versions>
  </versioning>
</metadata>"#;
        assert_eq!(
            xml_versions(xml),
            ["1.20.1-47.1.106", "21.1.264", "26.3.0.94"]
        );
        assert!(xml_versions("no versions here").is_empty());
    }

    #[test]
    fn maps_neoforge_versions_back_to_minecraft() {
        assert_eq!(neoforge_mc_of("21.1.264").as_deref(), Some("1.21.1"));
        assert_eq!(neoforge_mc_of("21.0.167").as_deref(), Some("1.21"));
        assert_eq!(neoforge_mc_of("20.4.237").as_deref(), Some("1.20.4"));
        assert_eq!(neoforge_mc_of("1.20.1-47.1.106").as_deref(), Some("1.20.1"));
        assert_eq!(neoforge_mc_of("26.3.0.94").as_deref(), Some("26.3"));
        assert_eq!(neoforge_prefix("1.21.1"), "21.1.");
        assert_eq!(neoforge_prefix("1.21"), "21.0.");
        assert_eq!(neoforge_prefix("26.3"), "26.3.");
    }

    #[test]
    fn version_dir_names_match_real_installers() {
        // 这些名字是实测 installer 写出来的目录名
        assert_eq!(
            expected_version_ids(LoaderKind::Fabric, "1.21.1", "0.16.9"),
            ["fabric-loader-0.16.9-1.21.1"]
        );
        assert_eq!(
            expected_version_ids(LoaderKind::Quilt, "1.21.1", "0.27.0"),
            ["quilt-loader-0.27.0-1.21.1"]
        );
        assert_eq!(
            expected_version_ids(LoaderKind::Forge, "1.21.1", "52.1.20"),
            ["1.21.1-forge-52.1.20"]
        );
        assert!(
            expected_version_ids(LoaderKind::NeoForge, "1.21.1", "21.1.264")
                .contains(&"neoforge-21.1.264".to_string())
        );
        // 老 NeoForge 版本号带 MC 前缀
        assert!(
            expected_version_ids(LoaderKind::NeoForge, "1.20.1", "1.20.1-47.1.106")
                .contains(&"1.20.1-neoforge-47.1.106".to_string())
        );
    }

    #[test]
    fn picks_the_new_loader_dir_not_the_vanilla_one() {
        let root = tmp("pick");
        let paths = GamePaths::new(&root);
        paths.prepare().unwrap();

        // 装之前：原版版本目录已经在
        let vanilla = paths.version_dir("1.21.1");
        std::fs::create_dir_all(&vanilla).unwrap();
        std::fs::write(paths.version_json("1.21.1"), b"{}").unwrap();
        let before: HashSet<String> = version_dirs(&paths).into_iter().collect();

        // Forge installer 会顺手写一个原版目录 + 一个加载器目录
        let forge_dir = paths.version_dir("1.21.1-forge-52.1.20");
        std::fs::create_dir_all(&forge_dir).unwrap();
        std::fs::write(
            paths.version_json("1.21.1-forge-52.1.20"),
            br#"{"mainClass":"cpw.mods.bootstraplauncher.BootstrapLauncher"}"#,
        )
        .unwrap();

        let picked =
            pick_version_id(LoaderKind::Forge, "1.21.1", "52.1.20", &before, &paths).unwrap();
        assert_eq!(picked, "1.21.1-forge-52.1.20");
        assert_eq!(
            read_main_class(&paths.version_json(&picked)).as_deref(),
            Some("cpw.mods.bootstraplauncher.BootstrapLauncher")
        );
        assert!(list_files(&forge_dir)
            .iter()
            .any(|p| p.ends_with("1.21.1-forge-52.1.20.json")));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn picks_a_fabric_dir_even_if_naming_changes() {
        let root = tmp("pick-fabric");
        let paths = GamePaths::new(&root);
        paths.prepare().unwrap();
        let before: HashSet<String> = version_dirs(&paths).into_iter().collect();

        // 假设某天 Fabric 改了命名规则
        let dir = paths.version_dir("fabric-1.21.1-custom");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(paths.version_json("fabric-1.21.1-custom"), b"{}").unwrap();

        let picked =
            pick_version_id(LoaderKind::Fabric, "1.21.1", "0.16.9", &before, &paths).unwrap();
        assert_eq!(picked, "fabric-1.21.1-custom");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn finds_an_already_installed_loader_without_a_diff() {
        let root = tmp("pick-existing");
        let paths = GamePaths::new(&root);
        paths.prepare().unwrap();
        let dir = paths.version_dir("fabric-loader-0.16.9-1.21.1");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(paths.version_json("fabric-loader-0.16.9-1.21.1"), b"{}").unwrap();
        let before: HashSet<String> = version_dirs(&paths).into_iter().collect();

        // 没有任何新增，也要认出旧目录
        let picked =
            pick_version_id(LoaderKind::Fabric, "1.21.1", "0.16.9", &before, &paths).unwrap();
        assert_eq!(picked, "fabric-loader-0.16.9-1.21.1");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn surfaces_the_real_cause_not_just_the_last_line() {
        let lines: Vec<String> = [
            "Considering library net.neoforged:neoforge:21.1.264:universal",
            "Downloading library from https://maven.neoforged.net/x.jar",
            "java.net.SocketTimeoutException: Connect timed out",
            "  at java.base/sun.nio.ch.NioSocketImpl.timedFinishConnect(NioSocketImpl.java:551)",
            "There was an error during installation",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        let excerpt = error_excerpt(&lines, 12);
        assert!(excerpt.iter().any(|l| l.contains("SocketTimeoutException")));
        // 收尾语也在摘录里，调用方会去重
        assert!(excerpt.iter().any(|l| l.contains("There was an error")));
        // 纯进度行不该被当成错误
        assert!(!excerpt.iter().any(|l| l.starts_with("Considering library")));

        assert!(hint_for(LoaderKind::Forge, &lines.join("\n")).contains("maven"));
        // NeoForge 的提示必须点名它自己的域名，并说明 --mirror 无效
        let neo_hint = hint_for(LoaderKind::NeoForge, &lines.join("\n"));
        assert!(neo_hint.contains("maven.neoforged.net"), "{neo_hint}");
        assert!(neo_hint.contains("--mirror"), "{neo_hint}");
        assert!(error_excerpt(&[], 12).is_empty());
    }

    #[test]
    fn limits_the_ui_dropdown() {
        let list: Vec<String> = (0..50).map(|i| format!("0.{i}.0")).collect();
        assert_eq!(limit_versions(list).len(), VERSION_LIST_LIMIT);
        assert_eq!(VERSION_LIST_LIMIT, 20);
    }

    #[test]
    fn version_cache_hits_and_keeps_keys_apart() {
        let key = "测试源|Fabric|9.9.9";
        mc_cache_put(key, &["0.19.5".to_string(), "0.19.4".to_string()]);
        assert_eq!(
            mc_cache_get(key),
            Some(vec!["0.19.5".to_string(), "0.19.4".to_string()])
        );
        // 换个 MC 版本不能串味
        assert!(mc_cache_get("测试源|Fabric|9.9.8").is_none());
        // 换加载器/来源也不能串味
        assert!(mc_cache_get("测试源|Quilt|9.9.9").is_none());

        // 加载器版本缓存是另一份（带 stable 标记），互不覆盖
        let lkey = "测试源|Fabric|9.9.7";
        loader_cache_put(
            lkey,
            &[LoaderVersion {
                version: "0.19.5".into(),
                stable: true,
            }],
        );
        assert_eq!(loader_cache_get(lkey).unwrap()[0].version, "0.19.5");
        assert!(mc_cache_get(lkey).is_none());
    }

    #[tokio::test]
    async fn version_query_timeout_is_reported_clearly() {
        let err = with_timeout_for(
            std::time::Duration::from_millis(10),
            async {
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                Ok(vec!["0.19.5".to_string()])
            },
            || " Fabric 1.21.1 的可用版本".to_string(),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("查不到 Fabric 1.21.1 的可用版本"), "{err}");
        assert!(err.contains("超时"), "{err}");
    }

    #[tokio::test]
    async fn version_query_error_is_not_a_loader_install_error() {
        let err = with_timeout_for(
            std::time::Duration::from_secs(5),
            async { Err::<Vec<String>, _>(Error::Http("maven 挂了".into())) },
            || " Fabric 1.21.1 的可用版本".to_string(),
        )
        .await
        .unwrap_err()
        .to_string();
        // 不能出现「安装加载器失败」这种会误导用户的措辞
        assert!(!err.contains("安装加载器失败"), "{err}");
        assert!(err.contains("查不到"), "{err}");
        assert!(err.contains("maven 挂了"), "{err}");
    }

    #[test]
    fn keeps_the_last_lines_for_the_error_message() {
        let lines: Vec<String> = (0..100).map(|i| format!("line {i}")).collect();
        let tail = tail(&lines, 40);
        assert!(tail.starts_with("line 60"));
        assert!(tail.ends_with("line 99"));
        assert_eq!(tail.lines().count(), 40);
    }

    #[test]
    fn filters_installer_noise_from_progress_events() {
        assert!(worth_showing("Installing 1.21.1 with fabric 0.16.9"));
        assert!(!worth_showing("  Copying net/minecraft/Util.class"));
        assert!(!worth_showing("\tPatching foo"));
        assert!(!worth_showing("   "));
        assert!(!worth_showing(&"x".repeat(400)));
    }

    #[test]
    fn sorts_versions_naturally() {
        // maven 元数据里 52.1.2 排在 52.1.20 后面，不能按字符串排
        let mut forge = vec!["52.1.2", "52.1.20", "52.1.9", "52.1.0"];
        forge.sort_by(|a, b| compare_versions(b, a));
        assert_eq!(forge, ["52.1.20", "52.1.9", "52.1.2", "52.1.0"]);

        let mut quilt = vec!["0.23.0", "0.24.0", "0.23.1", "0.20.0-beta.9"];
        quilt.sort_by(|a, b| compare_versions(b, a));
        assert_eq!(quilt, ["0.24.0", "0.23.1", "0.23.0", "0.20.0-beta.9"]);

        let mut neo = vec!["21.1.3", "21.1.264", "21.1.20"];
        neo.sort_by(|a, b| compare_versions(b, a));
        assert_eq!(neo, ["21.1.264", "21.1.20", "21.1.3"]);

        // UI 列表严格从新到旧（不稳定版也在里面，只是顺序按版本）
        let list = vec![
            LoaderVersion {
                version: "0.19.5".into(),
                stable: true,
            },
            LoaderVersion {
                version: "0.20.1".into(),
                stable: false,
            },
        ];
        let sorted = sort_versions_desc(list);
        assert_eq!(sorted[0].version, "0.20.1");

        // 但自动选版本要优先稳定版，哪怕它旧一点
        assert_eq!(pick_loader_version(&sorted).as_deref(), Some("0.19.5"));
        // 全是预发布版时才退而求其次
        let all_unstable = vec![LoaderVersion {
            version: "0.20.1".into(),
            stable: false,
        }];
        assert_eq!(
            pick_loader_version(&all_unstable).as_deref(),
            Some("0.20.1")
        );
        assert!(pick_loader_version(&[]).is_none());
    }

    #[test]
    fn reads_jar_magic() {
        let dir = tmp("jar");
        let jar = dir.join("a.jar");
        std::fs::write(&jar, b"PK\x03\x04rest").unwrap();
        assert!(is_jar(&jar));
        let not_jar = dir.join("b.jar");
        std::fs::write(&not_jar, b"<html>404</html>").unwrap();
        assert!(!is_jar(&not_jar));
        assert!(!is_jar(&dir.join("missing.jar")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------------ 联网验收
    // 这些会真的下载、真的跑 installer，默认不跑：
    //   cargo test -p mcdl-core -- --ignored --nocapture loader::tests::real

    #[tokio::test]
    #[ignore = "联网：查询 Fabric 版本列表"]
    async fn real_fabric_version_lists() {
        let client = HttpClient::new().unwrap();

        // 第一次走网络
        let started = std::time::Instant::now();
        let loaders = available_loader_versions(&client, LoaderKind::Fabric, "1.21.1")
            .await
            .unwrap();
        let cold = started.elapsed();
        assert!(!loaders.is_empty());
        assert!(
            loaders.len() <= VERSION_LIST_LIMIT,
            "UI 列表必须限条数：{}",
            loaders.len()
        );
        // 从新到旧
        for pair in loaders.windows(2) {
            assert_ne!(
                compare_versions(&pair[0], &pair[1]),
                std::cmp::Ordering::Less,
                "列表没按新到旧排：{:?}",
                &loaders[..loaders.len().min(5)]
            );
        }
        eprintln!(
            "fabric loaders（前 5）：{:?}（冷查询 {cold:?}）",
            &loaders[..5.min(loaders.len())]
        );

        // 第二次必须命中进程内缓存，不再打网络
        let started = std::time::Instant::now();
        let again = available_loader_versions(&client, LoaderKind::Fabric, "1.21.1")
            .await
            .unwrap();
        let warm = started.elapsed();
        assert_eq!(loaders, again);
        assert!(
            warm < std::time::Duration::from_millis(50),
            "缓存没生效：{warm:?}"
        );
        eprintln!("fabric loaders 二次查询命中缓存：{warm:?}");

        let mcs = supported_mc_versions(&client, LoaderKind::Fabric)
            .await
            .unwrap();
        assert!(mcs.contains(&"1.21.1".to_string()));
        eprintln!("fabric 支持 {} 个 MC 版本", mcs.len());

        // 公开的 fetch_installer 也要能用（缓存目录随便给一个）
        let cache =
            std::env::temp_dir().join(format!("mcdl-installer-cache-{}", std::process::id()));
        let spec = LoaderSpec::new(LoaderKind::Fabric, "1.21.1").with_version("0.16.9");
        let jar = fetch_installer(&client, LoaderKind::Fabric, &spec, &cache)
            .await
            .unwrap();
        assert!(is_jar(&jar), "{}", jar.display());
        eprintln!("installer 缓存：{}", jar.display());
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[tokio::test]
    #[ignore = "联网：查询 Forge / NeoForge / Quilt 版本列表"]
    async fn real_other_version_lists() {
        let client = HttpClient::new().unwrap();

        let forge = available_loader_versions(&client, LoaderKind::Forge, "1.21.1")
            .await
            .unwrap();
        assert!(forge.iter().any(|v| v.starts_with("52.")), "{forge:?}");
        eprintln!(
            "forge 1.21.1 loaders（前 3）：{:?}",
            &forge[..3.min(forge.len())]
        );

        let forge_mcs = supported_mc_versions(&client, LoaderKind::Forge)
            .await
            .unwrap();
        assert!(forge_mcs.contains(&"1.21.1".to_string()));
        assert!(
            forge_mcs.contains(&"1.12.2".to_string()),
            "老版本也要在列表里"
        );
        eprintln!("forge 支持 {} 个 MC 版本", forge_mcs.len());

        let quilt = available_loader_versions(&client, LoaderKind::Quilt, "1.21.1")
            .await
            .unwrap();
        assert!(!quilt.is_empty());
        eprintln!(
            "quilt 1.21.1 loaders（前 3）：{:?}",
            &quilt[..3.min(quilt.len())]
        );

        // 官方 maven.neoforged.net 在部分网络下不通，这里走镜像验证解析逻辑
        let neo = loader_versions(&client, LoaderKind::NeoForge, "1.21.1", Source::Bmclapi)
            .await
            .unwrap();
        assert!(
            neo.iter().any(|v| v.version.starts_with("21.1.")),
            "{neo:?}"
        );
        eprintln!(
            "neoforge 1.21.1 loaders（前 3）：{:?}",
            neo.iter().take(3).map(|v| &v.version).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    #[ignore = "联网：真的装一次 Fabric"]
    async fn real_fabric_install() {
        let client = HttpClient::new().unwrap();
        let root = std::env::temp_dir().join(format!("mcdl-real-fabric-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = GamePaths::new(&root);
        paths.prepare().unwrap();

        let java = crate::java::detect()
            .into_iter()
            .find(|j| j.major >= 17)
            .expect("本机需要 Java 17+");

        let ctx = LoaderCtx {
            client: &client,
            paths: &paths,
            java: &java.path,
            source: Source::Official,
        };
        let spec = LoaderSpec::new(LoaderKind::Fabric, "1.21.1").with_version("0.16.9");
        let result = install(&ctx, &spec, &crate::types::NullSink).await.unwrap();

        eprintln!("version_id = {}", result.version_id);
        eprintln!("main_class = {:?}", result.main_class);
        eprintln!("files = {}", result.files.len());
        eprintln!(
            "log 末尾：\n{}",
            tail(
                &result.log.lines().map(str::to_string).collect::<Vec<_>>(),
                10
            )
        );

        assert!(paths.version_dir(&result.version_id).is_dir());
        assert!(paths.version_json(&result.version_id).is_file());
        assert!(result.version_id.starts_with("fabric-loader-"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
