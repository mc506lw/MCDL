//! Java 运行时：探测本机、按版本需求挑选、缺失时从 Adoptium 取一份。
//!
//! Adoptium（Eclipse Temurin）的二进制可以自由再分发，是这里唯一的内置来源。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Error, Result};
use crate::net::HttpClient;
use crate::types::{JavaRuntime, ProgressSink};

/// `bin/` 下的可执行文件名。
fn java_exe() -> &'static str {
    if cfg!(windows) {
        "java.exe"
    } else {
        "java"
    }
}

/// 扫本机：`JAVA_HOME`、`PATH`、各平台常见安装目录。
pub fn detect() -> Vec<JavaRuntime> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    // 1. JAVA_HOME：最明确的信号。它旁边往往还躺着别的版本
    //    （`D:\java\zulu8|zulu17|zulu21`、sdkman、手动解压的 JDK 都是这种布局）。
    if let Some(home) = std::env::var_os("JAVA_HOME") {
        let home = PathBuf::from(home);
        push_home(&mut candidates, &home);
        if let Some(parent) = home.parent() {
            // 有些发行版把 JAVA_HOME 指到 `<jdk>/jre`，那就在上一级再找一次。
            push_home(&mut candidates, parent);
            // 同级目录：一个目录下放多个 JDK 是最常见的自建布局。
            push_children(&mut candidates, parent);
        }
    }

    // 2. PATH：用户自己配的、或者包管理器装的。
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let exe = dir.join(java_exe());
            if exe.is_file() {
                candidates.push(exe);
            }
        }
    }

    // 3. 各平台常见安装目录：目录本身和它的一级子目录都当成运行时。
    for root in install_roots() {
        push_tree(&mut candidates, &root);
    }

    // 4. 去重（PATH 与 JAVA_HOME 常常指向同一份），并逐个跑 `java -version` 验证。
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut seen_runtime: HashSet<(u32, String, String)> = HashSet::new();
    let mut found = Vec::new();
    for exe in candidates {
        let key = std::fs::canonicalize(&exe).unwrap_or_else(|_| exe.clone());
        if !seen.insert(key) {
            continue;
        }
        match probe(&exe) {
            Some(rt) => {
                // JDK 8 这类发行版同时有 bin/java 和 jre/bin/java，是同一个运行时
                if seen_runtime.insert((rt.major, rt.version.clone(), rt.vendor.clone())) {
                    found.push(rt);
                } else {
                    tracing::debug!(path = %exe.display(), "同一个 Java 运行时的另一份可执行文件");
                }
            }
            None => tracing::debug!(path = %exe.display(), "跳过无法识别的 Java"),
        }
    }

    // 版本高的排前面：调用方按需挑。
    found.sort_by(|a, b| b.major.cmp(&a.major).then_with(|| a.path.cmp(&b.path)));

    // 让用户能在日志里自查"为什么选了这个 Java"。
    if found.is_empty() {
        tracing::info!("本机没有找到可用的 Java，需要时会从 Adoptium 下载");
    } else {
        for rt in &found {
            tracing::info!(
                major = rt.major,
                version = %rt.version,
                vendor = rt.vendor,
                path = %rt.path.display(),
                "发现 Java"
            );
        }
    }
    found
}

/// 在已探测到的运行时里挑一个能用的：优先精确匹配主版本。
///
/// 注意方向：需求是 8、本机是 17 时仍然返回 17（版本高可以跑低版本）；
/// 一个都不满足需求时返回 `None`，由调用方决定去下载对应版本。
pub fn pick(runtimes: &[JavaRuntime], major: u32) -> Option<&JavaRuntime> {
    runtimes
        .iter()
        .filter(|j| j.major >= major)
        .min_by_key(|j| j.major)
}

/// 版本 JSON 里没写 `javaVersion` 时，按 MC 版本推断需要的主版本号。
///
/// | MC | Java |
/// | --- | --- |
/// | ≤ 1.16.5 | 8 |
/// | 1.17 – 1.20.4 | 17 |
/// | 1.20.5 – 1.21.x | 21 |
/// | 26.x+ | 21 起，按 JSON 为准 |
pub fn required_major(mc_version: &str) -> u32 {
    let nums = leading_numbers(mc_version);
    match nums.first().copied() {
        // 1.x 系列：按 minor/patch 分三段
        Some(1) => {
            let minor = nums.get(1).copied().unwrap_or(0);
            let patch = nums.get(2).copied().unwrap_or(0);
            if minor <= 16 {
                8
            } else if (minor, patch) <= (20, 4) {
                17
            } else {
                21
            }
        }
        // 2026 年之后 Mojang 改成了 26.x 这种编号，一律按 21 起，真实需求以 JSON 为准
        Some(major) if major >= 22 => 21,
        // 快照（`24w14a`）之类认不出年代的，按当前主流的 21
        _ => 21,
    }
}

/// 从 Adoptium 下载并解压一个 JRE 到 `dest`，返回 `java` 可执行文件路径。
///
/// 多个主版本共存在同一个 `dest`（`<gameDir>/runtime`）下时不会互相覆盖：
/// 每个主版本落在 `dest/jre-<major>`，这是 1.12 与 1.21 交替启动的必要条件。
pub async fn ensure(
    client: &HttpClient,
    major: u32,
    dest: &Path,
    sink: &dyn ProgressSink,
) -> Result<PathBuf> {
    let root = dest.join(format!("jre-{major}"));
    let exe = root.join("bin").join(java_exe());
    if exe.is_file() {
        tracing::info!(java = %exe.display(), "复用已下载的 Java {major}");
        return Ok(exe);
    }

    let (arch, os) = adoptium_platform();
    let url = format!(
        "https://api.adoptium.net/v3/assets/latest/{major}/hotspot?architecture={arch}&image_type=jre&os={os}"
    );
    tracing::info!(%url, "查询 Adoptium 资产");
    let assets: Vec<AdoptiumAsset> = client.get_json(&url).await?;
    let package = assets
        .into_iter()
        .filter_map(|a| a.binary.package)
        .find(|p| p.name.ends_with(".zip"))
        .ok_or_else(|| {
            Error::other(format!(
                "Adoptium 没有提供 Java {major} 的 {os}/{arch} JRE 压缩包"
            ))
        })?;

    sink.emit(crate::types::Progress::Log {
        level: crate::types::LogLevel::Info,
        message: format!("下载 Java {major}（{}）", package.name),
    });

    let zip_path = dest.join(format!(".jre-{major}.zip.part"));
    if let Some(parent) = zip_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }

    // Adoptium 的二进制挂在 GitHub Releases 上，国内经常连不上。
    // 主源失败时换同一个文件（同名、同厂商）的清华镜像，并在日志里说明。
    if let Err(primary) =
        download_to(client, &package.link, &zip_path, package.size, major, sink).await
    {
        let mirror = tuna_mirror_url(major, &package.name);
        tracing::warn!(error = %primary, %mirror, "Adoptium 官方下载失败，改用清华镜像");
        sink.emit(crate::types::Progress::Log {
            level: crate::types::LogLevel::Warn,
            message: format!("从 GitHub 下载 Java 失败（{}），改用清华镜像", primary),
        });
        download_to(client, &mirror, &zip_path, package.size, major, sink)
            .await
            .map_err(|mirror_err| {
                Error::other(format!(
                    "Java {major} 下载失败：官方源（{}）与镜像都不通。官方源错误：{primary}；镜像错误：{mirror_err}",
                    package.link
                ))
            })?;
    }

    let zip_for_extract = zip_path.clone();
    let root_for_extract = root.clone();
    tokio::task::spawn_blocking(move || {
        extract_zip_stripping_top(&zip_for_extract, &root_for_extract)
    })
    .await
    .map_err(|e| Error::other(format!("解压 Java 的后台任务失败：{e}")))??;

    let _ = std::fs::remove_file(&zip_path);
    if !exe.is_file() {
        return Err(Error::other(format!(
            "Java 解压完成，但没有找到 {}（压缩包结构可能变了）",
            exe.display()
        )));
    }
    tracing::info!(java = %exe.display(), "Java {major} 就绪");
    Ok(exe)
}

// ---------------------------------------------------------------- 探测细节

/// 常见安装根目录：目录本身和它的一级子目录都可能是一个 JDK/JRE。
fn install_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if cfg!(windows) {
        for base in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
            let Some(base) = std::env::var_os(base) else {
                continue;
            };
            let base = PathBuf::from(base);
            for name in [
                "Java",
                "Eclipse Adoptium",
                "Eclipse Foundation",
                "Zulu",
                "Microsoft",
                "Amazon Corretto",
                "BellSoft",
                "Semeru",
                "GraalVM",
                "AdoptOpenJDK",
            ] {
                roots.push(base.join(name));
            }
        }
        // 新版 Adoptium / 微软的安装器默认装到用户目录
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let programs = PathBuf::from(local).join("Programs");
            roots.push(programs.join("Eclipse Adoptium"));
            roots.push(programs.join("Eclipse Foundation"));
            roots.push(programs.join("Microsoft"));
            roots.push(programs.join("Java"));
        }
        // 官方启动器自带的 JRE：runtime/<component>/<platform>/bin/java
        if let Some(appdata) = std::env::var_os("APPDATA") {
            roots.push(PathBuf::from(appdata).join(".minecraft").join("runtime"));
        }
    } else if cfg!(target_os = "macos") {
        roots.push(PathBuf::from("/Library/Java/JavaVirtualMachines"));
    } else {
        for base in ["/usr/lib/jvm", "/usr/java", "/opt/java", "/opt/jdk"] {
            roots.push(PathBuf::from(base));
        }
    }

    // JetBrains / Gradle 下载的 JDK
    if let Some(home) = home_dir() {
        roots.push(home.join(".jdks"));
        roots.push(home.join(".gradle").join("jdks"));
        roots.push(home.join(".sdkman").join("candidates").join("java"));
    }
    roots
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

fn child_dirs(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs
}

/// 把 `<home>/bin/java`、`<home>/jre/bin/java`、`<home>/java` 加进候选。
fn push_home(candidates: &mut Vec<PathBuf>, home: &Path) {
    for prefix in [home.to_path_buf(), home.join("jre")] {
        let exe = prefix.join("bin").join(java_exe());
        if exe.is_file() {
            candidates.push(exe);
        }
    }
    // 传进来的可能直接就是 `bin` 目录
    if home.file_name().and_then(|n| n.to_str()) == Some("bin") {
        let exe = home.join(java_exe());
        if exe.is_file() {
            candidates.push(exe);
        }
    }
}

/// 扫 `dir` 的直接子目录，把每个当成运行时（`D:\java\zulu8`、`zulu17`…）。
fn push_children(candidates: &mut Vec<PathBuf>, dir: &Path) {
    for child in child_dirs(dir) {
        push_home(candidates, &child);
    }
}

/// 目录本身 + 一层子目录都扫（安装根目录的两种常见形态）。
fn push_tree(candidates: &mut Vec<PathBuf>, dir: &Path) {
    push_home(candidates, dir);
    push_children(candidates, dir);
    // Mojang 官方启动器的 JRE 布局：runtime/<component>/<platform>/bin/java
    for component in child_dirs(dir) {
        for platform in child_dirs(&component) {
            push_home(candidates, &platform);
        }
    }
}

/// 跑一次 `java -version`，把结果解析成 [`JavaRuntime`]。跑不起来就返回 `None`。
fn probe(exe: &Path) -> Option<JavaRuntime> {
    let mut cmd = Command::new(exe);
    cmd.arg("-version");
    hide_console(&mut cmd);
    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }

    // `java -version` 的历史包袱：输出走 stderr。
    let mut text = String::from_utf8_lossy(&output.stderr).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stdout));

    let (major, version) = parse_version(&text)?;
    Some(JavaRuntime {
        path: exe.to_path_buf(),
        major,
        version,
        vendor: vendor_of(&text).to_string(),
        arch: arch_of(&text).to_string(),
    })
}

/// 从 `openjdk version "17.0.19" 2026-04-21 LTS` 里取出版本号与主版本。
fn parse_version(text: &str) -> Option<(u32, String)> {
    let rest = text.split_once("version \"")?.1;
    let end = rest.find('"')?;
    let raw = &rest[..end];
    let mut parts = raw.split(['.', '_', '-', '+']);
    let first = parts.next()?;
    let major = if first == "1" {
        // 1.8.0_402 → 8
        parts.next()?.parse::<u32>().ok()?
    } else {
        first.parse::<u32>().ok()?
    };
    Some((major, raw.to_string()))
}

fn vendor_of(text: &str) -> &'static str {
    let lower = text.to_ascii_lowercase();
    if lower.contains("temurin") || lower.contains("adoptium") {
        "Eclipse Temurin"
    } else if lower.contains("zulu") {
        "Azul Zulu"
    } else if lower.contains("microsoft") {
        "Microsoft OpenJDK"
    } else if lower.contains("corretto") || lower.contains("amazon") {
        "Amazon Corretto"
    } else if lower.contains("graalvm") {
        "GraalVM"
    } else if lower.contains("semeru") || lower.contains("openj9") || lower.contains("ibm") {
        "IBM Semeru"
    } else if lower.contains("liberica") || lower.contains("bellsoft") {
        "BellSoft Liberica"
    } else if lower.contains("oracle") {
        "Oracle JDK"
    } else {
        "OpenJDK"
    }
}

fn arch_of(text: &str) -> &'static str {
    let lower = text.to_ascii_lowercase();
    if lower.contains("aarch64") || lower.contains("arm64") {
        "arm64"
    } else if lower.contains("64-bit") {
        "x64"
    } else if lower.contains("32-bit") {
        "x86"
    } else {
        std::env::consts::ARCH
    }
}

/// 版本号里开头那串数字：`1.21.1-pre1` → `[1, 21, 1]`，`24w14a` → `[]`。
///
/// 只在遇到版本号允许的分隔符（`-`、`+`、空白）时保留已读到的数字；
/// 撞上字母说明这不是 `x.y.z` 形态（快照 `24w14a`），返回空。
fn leading_numbers(raw: &str) -> Vec<u32> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut stopped_on_letter = false;
    for ch in raw.chars() {
        if ch.is_ascii_digit() {
            current.push(ch);
        } else if ch == '.' && !current.is_empty() {
            if let Ok(n) = current.parse() {
                out.push(n);
            }
            current.clear();
        } else if ch.is_ascii_alphabetic() {
            stopped_on_letter = true;
            break;
        } else {
            break;
        }
    }
    if stopped_on_letter {
        return Vec::new();
    }
    if let Ok(n) = current.parse() {
        out.push(n);
    }
    out
}

/// 子进程不要弹控制台窗口：这是个 GUI 应用。
pub(crate) fn hide_console(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

// ---------------------------------------------------------------- Adoptium

#[derive(Debug, serde::Deserialize)]
struct AdoptiumAsset {
    binary: AdoptiumBinary,
}

#[derive(Debug, serde::Deserialize)]
struct AdoptiumBinary {
    package: Option<AdoptiumPackage>,
}

#[derive(Debug, serde::Deserialize)]
struct AdoptiumPackage {
    link: String,
    name: String,
    #[serde(default)]
    size: Option<u64>,
}

/// Adoptium 的 os / architecture 参数。
fn adoptium_platform() -> (&'static str, &'static str) {
    let os = match std::env::consts::OS {
        "windows" => "windows",
        "macos" => "mac",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "x86" => "x86",
        "aarch64" => "aarch64",
        other => other,
    };
    (arch, os)
}

/// Adoptium 官方文件的清华镜像地址（同一份 Temurin 产物，只是换了托管）。
///
/// 只在官方 GitHub 下载失败时用，且会在日志里明确告知用户。
fn tuna_mirror_url(major: u32, file_name: &str) -> String {
    let (arch, os) = adoptium_platform();
    format!("https://mirrors.tuna.tsinghua.edu.cn/Adoptium/{major}/jre/{arch}/{os}/{file_name}")
}

/// 流式下载到文件，边下边抛字节进度，避免把几十 MB 全塞进内存。
async fn download_to(
    client: &HttpClient,
    url: &str,
    dest: &Path,
    size: Option<u64>,
    major: u32,
    sink: &dyn ProgressSink,
) -> Result<()> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    let resp = client
        .raw()
        .get(url)
        .send()
        .await
        .map_err(|e| Error::Http(format!("{url}：{e}")))?;
    if !resp.status().is_success() {
        return Err(Error::Http(format!("{url}：HTTP {}", resp.status())));
    }

    let total = resp.content_length().or(size).unwrap_or(0);
    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| Error::io(dest, e))?;
    let mut stream = resp.bytes_stream();
    let mut done: u64 = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| Error::Http(format!("{url}：{e}")))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| Error::io(dest, e))?;
        done += chunk.len() as u64;
        sink.emit(crate::types::Progress::Files {
            kind: crate::types::StageKind::Java,
            done: 0,
            total: 0,
            bytes_done: done,
            bytes_total: total,
        });
    }
    file.flush().await.map_err(|e| Error::io(dest, e))?;
    tracing::debug!(bytes = done, major, "Java 压缩包下载完成");
    Ok(())
}

/// 解压 zip，并剥掉压缩包里统一的顶层目录（Adoptium 的包是 `jdk-21.x-jre/...`）。
fn extract_zip_stripping_top(zip_path: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(zip_path).map_err(|e| Error::io(zip_path, e))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| Error::other(format!("Java 压缩包损坏：{e}")))?;

    // 先确认所有条目共享同一个顶层目录，是才剥；否则原样解压，免得丢文件。
    let mut top: Option<String> = None;
    let mut strip = true;
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| Error::other(format!("读取 Java 压缩包失败：{e}")))?;
        let Some(name) = entry.enclosed_name() else {
            strip = false;
            break;
        };
        let mut comps = name.components();
        let Some(first) = comps.next() else {
            strip = false;
            break;
        };
        if comps.next().is_none() && !entry.is_dir() {
            // 顶层就有文件（不是目录），不能剥
            strip = false;
            break;
        }
        let first = first.as_os_str().to_string_lossy().to_string();
        match &top {
            None => top = Some(first),
            Some(existing) if *existing != first => {
                strip = false;
                break;
            }
            _ => {}
        }
    }

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| Error::other(format!("读取 Java 压缩包失败：{e}")))?;
        // `enclosed_name` 已经挡掉了 `../` 这类越界路径
        let Some(name) = entry.enclosed_name() else {
            continue;
        };
        let rel: PathBuf = if strip {
            name.components().skip(1).collect()
        } else {
            name.to_path_buf()
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        let out = dest.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| Error::io(&out, e))?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        let mut sink = std::fs::File::create(&out).map_err(|e| Error::io(&out, e))?;
        std::io::copy(&mut entry, &mut sink).map_err(|e| Error::io(&out, e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = entry.unix_mode() {
                let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(major: u32) -> JavaRuntime {
        JavaRuntime {
            path: PathBuf::from(format!("/java/{major}/bin/java")),
            major,
            version: format!("{major}.0.0"),
            vendor: "Test".into(),
            arch: "x64".into(),
        }
    }

    #[test]
    fn required_major_follows_mojang_timeline() {
        for (mc, want) in [
            ("1.6.4", 8),
            ("1.12.2", 8),
            ("1.16.5", 8),
            ("1.17", 17),
            ("1.18.2", 17),
            ("1.20.1", 17),
            ("1.20.4", 17),
            ("1.20.5", 21),
            ("1.20.6", 21),
            ("1.21", 21),
            ("1.21.1", 21),
            ("1.21.1-pre1", 21),
            ("26.3", 21),
            ("24w14a", 21),
        ] {
            assert_eq!(required_major(mc), want, "MC {mc}");
        }
    }

    #[test]
    fn pick_prefers_the_closest_enough_major() {
        let runtimes = vec![rt(8), rt(17), rt(21), rt(25)];
        assert_eq!(pick(&runtimes, 17).unwrap().major, 17);
        // 要 21，有 21 就选 21，不选 25
        assert_eq!(pick(&runtimes, 21).unwrap().major, 21);
        // 要 24，没有 24 就退到最小的够用版本 25
        assert_eq!(pick(&runtimes, 24).unwrap().major, 25);
        // 需求是 8、本机是 17：版本高可以跑低版本，必须能选中
        assert_eq!(pick(&[rt(17)], 8).unwrap().major, 17);
        // 本机全都低于需求：返回 None，让调用方去下载，绝不能静默降级
        assert!(pick(&runtimes, 30).is_none());
        assert!(pick(&[rt(8), rt(17)], 21).is_none());
        assert!(pick(&[], 17).is_none());
    }

    #[test]
    fn finds_jdks_sitting_side_by_side() {
        // `D:\java\{zulu8,zulu17,zulu21}` 这种布局：JAVA_HOME 只指到其中一个，
        // 其余的要靠扫它的父目录发现。
        let root = std::env::temp_dir().join(format!("mcdl-jdks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for name in ["zulu8", "zulu17", "zulu21"] {
            let bin = root.join(name).join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            std::fs::write(bin.join(java_exe()), b"fake").unwrap();
        }
        std::fs::create_dir_all(root.join("not-a-jdk")).unwrap();

        let mut candidates = Vec::new();
        push_children(&mut candidates, &root);
        assert_eq!(candidates.len(), 3, "{candidates:?}");

        // 直接给一个 JDK 根目录也能认
        let mut one = Vec::new();
        push_home(&mut one, &root.join("zulu21"));
        assert_eq!(one.len(), 1);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn parses_java_version_strings() {
        let zulu = "openjdk version \"17.0.19\" 2026-04-21 LTS\n\
                    OpenJDK Runtime Environment Zulu17.66+19-CA (build 17.0.19+10-LTS)\n\
                    OpenJDK 64-Bit Server VM Zulu17.66+19-CA (build 17.0.19+10-LTS, mixed mode)";
        let (major, version) = parse_version(zulu).unwrap();
        assert_eq!(major, 17);
        assert_eq!(version, "17.0.19");
        assert_eq!(vendor_of(zulu), "Azul Zulu");
        assert_eq!(arch_of(zulu), "x64");

        let eight = "java version \"1.8.0_402\"\nJava(TM) SE Runtime Environment";
        let (major, version) = parse_version(eight).unwrap();
        assert_eq!(major, 8);
        assert_eq!(version, "1.8.0_402");

        let temurin = "openjdk version \"21.0.12.1\" 2026-07-15 LTS\n\
                       OpenJDK Runtime Environment Temurin-21.0.12.1+1 (build 21.0.12.1+1-LTS)";
        assert_eq!(parse_version(temurin).unwrap().0, 21);
        assert_eq!(vendor_of(temurin), "Eclipse Temurin");

        assert!(parse_version("no version here").is_none());
    }

    #[test]
    fn leading_numbers_reads_the_version_prefix() {
        assert_eq!(leading_numbers("1.21.1"), vec![1, 21, 1]);
        assert_eq!(leading_numbers("1.21"), vec![1, 21]);
        assert_eq!(leading_numbers("26.3"), vec![26, 3]);
        assert_eq!(leading_numbers("1.21.1-pre1"), vec![1, 21, 1]);
        assert!(leading_numbers("24w14a").is_empty());
    }

    #[test]
    fn detect_finds_the_jdk_behind_java_home() {
        // 本机既没有 JAVA_HOME 也没有 PATH 里的 java 时允许空跑。
        if std::env::var_os("JAVA_HOME").is_none()
            && Command::new(java_exe()).arg("-version").output().is_err()
        {
            return;
        }
        let found = detect();
        assert!(!found.is_empty(), "PATH 或 JAVA_HOME 里应该有 java");
        assert!(found.iter().any(|j| j.major >= 8));
        assert!(found.iter().all(|j| j.path.is_file()));
    }

    #[test]
    fn mirror_url_keeps_the_adoptium_layout() {
        let url = tuna_mirror_url(21, "OpenJDK21U-jre_x64_windows_hotspot_21.0.12.1_1.zip");
        assert!(url.starts_with("https://mirrors.tuna.tsinghua.edu.cn/Adoptium/21/jre/"));
        assert!(url.ends_with("/OpenJDK21U-jre_x64_windows_hotspot_21.0.12.1_1.zip"));
    }

    /// 联网验收：`cargo test -p mcdl-core -- --ignored --nocapture java::tests::real_adoptium_ensure`
    #[tokio::test]
    #[ignore = "联网：真的从 Adoptium 下载并解压一个 JRE"]
    async fn real_adoptium_ensure() {
        let client = HttpClient::new().unwrap();
        // 11 是本机肯定没有的主版本，能强制走完整的下载链路
        let dest = std::env::temp_dir().join(format!("mcdl-jre-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dest);
        let sink = crate::types::NullSink;

        let exe = ensure(&client, 11, &dest, &sink).await.unwrap();
        assert!(exe.is_file(), "{}", exe.display());
        assert_eq!(
            exe,
            dest.join("jre-11").join("bin").join(java_exe()),
            "必须落在 dest/jre-11 下"
        );

        let out = Command::new(&exe).arg("-version").output().unwrap();
        let text = String::from_utf8_lossy(&out.stderr).into_owned()
            + &String::from_utf8_lossy(&out.stdout);
        assert!(
            text.contains("version \"11"),
            "解压出来的不是 Java 11：{text}"
        );
        eprintln!("Adoptium Java 11 就绪：{}", exe.display());

        // 第二次必须复用，不重新下载
        let again = ensure(&client, 11, &dest, &sink).await.unwrap();
        assert_eq!(again, exe);

        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn extract_strips_the_single_top_level_dir() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("mcdl-zip-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("jre.zip");
        {
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
            let opts = zip::write::SimpleFileOptions::default();
            zip.add_directory("jdk-21.0.12+1-jre/", opts).unwrap();
            zip.start_file("jdk-21.0.12+1-jre/bin/java.exe", opts)
                .unwrap();
            zip.write_all(b"fake java").unwrap();
            zip.start_file("jdk-21.0.12+1-jre/release", opts).unwrap();
            zip.write_all(b"JAVA_VERSION=\"21\"").unwrap();
            zip.finish().unwrap();
        }
        let dest = dir.join("out");
        extract_zip_stripping_top(&zip_path, &dest).unwrap();
        assert!(dest.join("bin/java.exe").is_file());
        assert!(dest.join("release").is_file());
        assert!(!dest.join("jdk-21.0.12+1-jre").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
