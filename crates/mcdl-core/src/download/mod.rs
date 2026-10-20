//! 并发下载、SHA1 校验、断点续传、natives 解压。
//!
//! 设计要点：
//! - 已存在且校验通过的文件直接跳过（这就是"断点续传"：按文件粒度）
//! - 并发用固定大小的 worker 池，进度按累计字节抛给 [`ProgressSink`]
//! - 校验失败的文件删掉重下一次，再失败才报错

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::stream::{self, StreamExt, TryStreamExt};
use sha1::{Digest, Sha1};

use crate::error::{Error, IoContext, Result};
use crate::net::HttpClient;
use crate::types::{DownloadJob, DownloadOutcome, Progress, ProgressSink, StageKind};

/// 单个文件的下载超时。client.jar、JDK 这类大文件不能用客户端的 120s 默认值。
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// 进度事件的节流阈值：累计到这个字节数才抛一次，避免小文件把事件刷爆。
const PROGRESS_STEP: u64 = 256 * 1024;

/// 每个文件最多下几次（首次 + 重试一次）。
const ATTEMPTS: usize = 2;

/// 批量下载的共享计数。进度事件只读这几个原子量。
#[derive(Clone)]
struct Counters {
    done: Arc<AtomicUsize>,
    downloaded: Arc<AtomicUsize>,
    skipped: Arc<AtomicUsize>,
    /// 真正从网络上下下来的字节。跳过（已缓存）的文件不计入。
    bytes_done: Arc<AtomicU64>,
    /// 本阶段要下载的字节总量；没有官方大小时按实际下载量补进来。
    bytes_total: Arc<AtomicU64>,
    /// 写进 `DownloadOutcome.bytes`。
    bytes_fetched: Arc<AtomicU64>,
}

impl Counters {
    fn new(bytes_total: u64) -> Self {
        Self {
            done: Arc::new(AtomicUsize::new(0)),
            downloaded: Arc::new(AtomicUsize::new(0)),
            skipped: Arc::new(AtomicUsize::new(0)),
            bytes_done: Arc::new(AtomicU64::new(0)),
            bytes_total: Arc::new(AtomicU64::new(bytes_total)),
            bytes_fetched: Arc::new(AtomicU64::new(0)),
        }
    }

    fn emit(&self, sink: &dyn ProgressSink, stage: StageKind, total: usize) {
        sink.emit(Progress::Files {
            kind: stage,
            done: self.done.load(Ordering::Relaxed),
            total,
            bytes_done: self.bytes_done.load(Ordering::Relaxed),
            bytes_total: self.bytes_total.load(Ordering::Relaxed),
        });
    }
}

/// 批量下载。`stage` 只用于进度事件里的归类。
pub async fn run_batch(
    client: &HttpClient,
    jobs: Vec<DownloadJob>,
    concurrency: usize,
    sink: &dyn ProgressSink,
    stage: StageKind,
) -> Result<DownloadOutcome> {
    // 同一个目标路径只下一次：合并 inheritsFrom 之后偶尔会出现同名文件。
    let jobs = dedupe_by_dest(jobs);
    let total = jobs.len();
    if total == 0 {
        tracing::debug!(?stage, "没有需要下载的文件");
        return Ok(DownloadOutcome::default());
    }

    // 0 会退化成单线程；过大只会把对端打限流。日志里也要打实际生效的值。
    let concurrency = concurrency.clamp(1, 64);

    // 先判一遍「已经下好了吗」。这一步决定两个总量，之后都不再变：
    // - `done/total` 是文件数，跳过的也算 done，所以文件百分比能到 100%；
    // - 跳过的文件不计入 `bytes_done`，因此 `bytes_total` 也只算真正要下的那些，
    //   否则重装时字节百分比永远停在 30% 之类的数上。
    let mut pending: Vec<DownloadJob> = Vec::with_capacity(total);
    let mut skipped = 0usize;
    for job in jobs {
        if job.dest.exists() && verify(&job.dest, job.sha1.as_deref())? {
            skipped += 1;
            tracing::trace!(dest = %job.dest.display(), "已存在且校验通过，跳过");
        } else {
            pending.push(job);
        }
    }

    let bytes_total: u64 = pending.iter().filter_map(|job| job.size).sum();
    let counters = Counters::new(bytes_total);
    counters.skipped.store(skipped, Ordering::Relaxed);
    counters.done.store(skipped, Ordering::Relaxed);

    tracing::info!(
        ?stage,
        files = total,
        skipped,
        bytes = bytes_total,
        concurrency,
        "开始批量下载"
    );
    counters.emit(sink, stage, total);

    if !pending.is_empty() {
        stream::iter(pending.into_iter().map(|job| {
            let counters = counters.clone();
            async move {
                // 边下边抛进度，按字节节流，避免小文件把事件刷爆
                let mut last_emit = counters.bytes_done.load(Ordering::Relaxed);
                let mut fetched = 0u64;
                let mut on_chunk = |n: u64| {
                    fetched += n;
                    let current = counters.bytes_done.fetch_add(n, Ordering::Relaxed) + n;
                    if current.saturating_sub(last_emit) >= PROGRESS_STEP {
                        last_emit = current;
                        counters.emit(sink, stage, total);
                    }
                };
                fetch_with_retry(client, &job, &mut on_chunk).await?;

                counters.bytes_fetched.fetch_add(fetched, Ordering::Relaxed);
                if job.size.is_none() {
                    // 没有官方大小时把实际字节补进总量，进度条才不会超过 100%。
                    counters.bytes_total.fetch_add(fetched, Ordering::Relaxed);
                }
                counters.downloaded.fetch_add(1, Ordering::Relaxed);
                counters.done.fetch_add(1, Ordering::Relaxed);
                counters.emit(sink, stage, total);
                Ok::<(), Error>(())
            }
        }))
        .buffer_unordered(concurrency)
        .try_collect::<Vec<()>>()
        .await?;
    }

    // 收尾再抛一次，保证接收端能看到 100%。
    counters.emit(sink, stage, total);

    let outcome = DownloadOutcome {
        downloaded: counters.downloaded.load(Ordering::Relaxed),
        skipped: counters.skipped.load(Ordering::Relaxed),
        bytes: counters.bytes_fetched.load(Ordering::Relaxed),
    };
    tracing::info!(
        ?stage,
        downloaded = outcome.downloaded,
        skipped = outcome.skipped,
        bytes = outcome.bytes,
        "批量下载完成"
    );
    Ok(outcome)
}

/// 下载单个文件，必要时校验。目标已存在且校验通过时直接返回。
pub async fn download_one(client: &HttpClient, job: &DownloadJob) -> Result<()> {
    if job.dest.exists() && verify(&job.dest, job.sha1.as_deref())? {
        tracing::debug!(dest = %job.dest.display(), "已存在且校验通过，跳过");
        return Ok(());
    }
    tracing::debug!(url = %job.url, dest = %job.dest.display(), "下载文件");
    fetch_with_retry(client, job, &mut |_| {}).await
}

/// 下到临时文件 → 校验 → 原子改名。失败重试一次，两次都不过就报错。
async fn fetch_with_retry(
    client: &HttpClient,
    job: &DownloadJob,
    // `+ Send` 是硬要求：这个 future 会被 tokio::spawn 包住（CLI、Tauri 都是），
    // 裸 `&mut dyn FnMut` 跨 await 持有会让整个 Installer::run 不是 Send。
    on_chunk: &mut (dyn FnMut(u64) + Send),
) -> Result<()> {
    if let Some(parent) = job.dest.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).at(parent)?;
        }
    }

    let tmp = tmp_path(&job.dest);
    let mut last_error: Option<Error> = None;

    for attempt in 1..=ATTEMPTS {
        if attempt > 1 {
            tracing::warn!(
                url = %job.url,
                dest = %job.dest.display(),
                attempt,
                "上一次没成功，重试"
            );
        }

        match fetch_to_file(client, job, &tmp, on_chunk).await {
            Ok(()) => {
                if verify(&tmp, job.sha1.as_deref())? {
                    replace_file(&tmp, &job.dest)?;
                    return Ok(());
                }
                tracing::warn!(
                    url = %job.url,
                    dest = %job.dest.display(),
                    expected = ?job.sha1,
                    actual = ?sha1_file(&tmp).ok(),
                    "SHA1 与官方值不一致，删掉重下"
                );
                let _ = std::fs::remove_file(&tmp);
                last_error = Some(Error::HashMismatch {
                    path: job.dest.clone(),
                });
            }
            Err(e) => {
                tracing::warn!(url = %job.url, error = %e, "下载失败");
                let _ = std::fs::remove_file(&tmp);
                last_error = Some(e);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| Error::HashMismatch {
        path: job.dest.clone(),
    }))
}

/// 把响应体流式写到 `tmp`，每读到一块就回调一次。
async fn fetch_to_file(
    client: &HttpClient,
    job: &DownloadJob,
    tmp: &Path,
    // 同 `fetch_with_retry`：必须 `+ Send`，否则整个安装 future 不能 spawn。
    on_chunk: &mut (dyn FnMut(u64) + Send),
) -> Result<()> {
    use tokio::io::AsyncWriteExt as _;

    let resp = client
        .raw()
        .get(&job.url)
        .timeout(DOWNLOAD_TIMEOUT)
        .send()
        .await
        .map_err(|e| Error::Http(format!("GET {} 失败：{e}", job.url)))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(Error::Http(format!(
            "GET {} 返回 HTTP {status}",
            job.url
        )));
    }

    let mut file = tokio::fs::File::create(tmp).await.at(tmp)?;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|e| Error::Http(format!("读取 {} 的响应体失败：{e}", job.url)))?;
        file.write_all(&chunk).await.at(tmp)?;
        on_chunk(chunk.len() as u64);
    }
    file.flush().await.at(tmp)?;
    Ok(())
}

/// `a/b.jar` → `a/b.jar.part`（同目录，保证 rename 在同一个卷上）。
fn tmp_path(dest: &Path) -> PathBuf {
    let mut name = dest.as_os_str().to_os_string();
    name.push(".part");
    PathBuf::from(name)
}

/// 覆盖式改名。Windows 上 rename 不能覆盖已存在的文件，先删目标。
fn replace_file(tmp: &Path, dest: &Path) -> Result<()> {
    if dest.exists() {
        std::fs::remove_file(dest).at(dest)?;
    }
    std::fs::rename(tmp, dest).at(dest)?;
    Ok(())
}

fn dedupe_by_dest(jobs: Vec<DownloadJob>) -> Vec<DownloadJob> {
    let mut seen = HashSet::with_capacity(jobs.len());
    jobs.into_iter()
        .filter(|job| seen.insert(job.dest.clone()))
        .collect()
}

/// 算文件的 SHA1（小写十六进制）。
pub fn sha1_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path).at(path)?;
    let mut hasher = Sha1::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).at(path)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// 校验文件；没有期望值时返回 `true`。
///
/// 文件不存在一律返回 `false`：调用方拿它当「这个文件已经就绪了吗」用。
pub fn verify(path: &Path, expected: Option<&str>) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let Some(expected) = expected.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(true);
    };
    let actual = sha1_file(path)?;
    Ok(actual.eq_ignore_ascii_case(expected))
}

/// 把 natives jar 解压到版本目录下的 `natives/`，跳过 `extract.exclude` 里的路径。
///
/// 返回解压出来的文件数。目录结构和 jar 里一致，但会拒绝 `..` 之类的穿越路径。
pub fn extract_natives(jar: &Path, dest: &Path, exclude: &[String]) -> Result<usize> {
    let file = std::fs::File::open(jar).at(jar)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| {
        Error::other(format!(
            "打不开 natives 压缩包（{}）：{e}",
            jar.display()
        ))
    })?;

    std::fs::create_dir_all(dest).at(dest)?;
    let mut extracted = 0usize;

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|e| {
            Error::other(format!(
                "读取 natives 条目失败（{} 第 {index} 项）：{e}",
                jar.display()
            ))
        })?;

        if entry.is_dir() {
            continue;
        }

        // enclosed_name() 会把绝对路径和 `..` 挡掉
        let Some(name) = entry
            .enclosed_name()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
        else {
            tracing::warn!(jar = %jar.display(), entry = entry.name(), "条目路径不安全，跳过");
            continue;
        };

        // natives 包里只该有本机动态库；META-INF 是签名/清单，绝不能落盘
        if name.starts_with("META-INF/") || excluded(&name, exclude) {
            tracing::trace!(entry = %name, "按规则跳过");
            continue;
        }

        // jar 里的路径是 `/` 分隔的；Windows 的 API 也认 `/`，直接 join 即可
        let out = dest.join(&name);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).at(parent)?;
        }
        let mut writer = std::fs::File::create(&out).at(&out)?;
        std::io::copy(&mut entry, &mut writer).at(&out)?;
        extracted += 1;
    }

    tracing::debug!(
        jar = %jar.display(),
        dest = %dest.display(),
        files = extracted,
        "natives 解压完成"
    );
    Ok(extracted)
}

/// `exclude` 里的条目按目录前缀理解：`META-INF/` 既排除目录本身也排除其下所有文件。
fn excluded(name: &str, exclude: &[String]) -> bool {
    exclude.iter().any(|pattern| {
        let pattern = pattern.trim_end_matches('/');
        if pattern.is_empty() {
            return false;
        }
        name == pattern || name.starts_with(&format!("{pattern}/"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "mcdl-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn job(dest: PathBuf, sha1: Option<&str>, size: Option<u64>) -> DownloadJob {
        DownloadJob {
            url: "http://127.0.0.1:1/never".to_string(),
            dest,
            sha1: sha1.map(str::to_owned),
            size,
            label: "test".to_string(),
        }
    }

    #[test]
    fn sha1_file_matches_known_value() {
        let dir = temp_dir("sha1");
        let path = dir.join("hello.txt");
        std::fs::write(&path, b"hello").unwrap();
        // echo -n hello | sha1sum
        assert_eq!(
            sha1_file(&path).unwrap(),
            "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_handles_missing_wrong_and_empty() {
        let dir = temp_dir("verify");
        let path = dir.join("a.bin");
        std::fs::write(&path, b"hello").unwrap();
        let good = "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d";

        assert!(verify(&path, Some(good)).unwrap());
        assert!(verify(&path, Some(&good.to_uppercase())).unwrap(), "大小写不敏感");
        assert!(!verify(&path, Some("0000000000000000000000000000000000000000")).unwrap());
        assert!(verify(&path, None).unwrap(), "没有期望值就认为可用");
        assert!(verify(&path, Some("  ")).unwrap(), "空白等于没有期望值");

        let missing = dir.join("nope.bin");
        assert!(!verify(&missing, None).unwrap(), "文件不存在不能算通过");
        assert!(!verify(&missing, Some(good)).unwrap());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 已存在且 SHA1 对得上的文件必须被跳过：不联网也能验证。
    #[tokio::test]
    async fn run_batch_skips_verified_files() {
        let dir = temp_dir("skip");
        let good = "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d";
        let path = dir.join("already.bin");
        std::fs::write(&path, b"hello").unwrap();

        let client = HttpClient::new().unwrap();
        let events = std::sync::Mutex::new(Vec::new());
        let sink = |progress: Progress| events.lock().unwrap().push(progress);

        let outcome = run_batch(
            &client,
            vec![job(path.clone(), Some(good), Some(5))],
            4,
            &sink,
            StageKind::Libraries,
        )
        .await
        .unwrap();

        assert_eq!(outcome.skipped, 1);
        assert_eq!(outcome.downloaded, 0);
        assert_eq!(outcome.bytes, 0);

        // 最后一个事件应该是 done == total，且跳过的字节不参与字节进度
        let events = events.into_inner().unwrap();
        let last = events
            .iter()
            .rev()
            .find_map(|p| match p {
                Progress::Files {
                    done,
                    total,
                    bytes_done,
                    bytes_total,
                    ..
                } => Some((*done, *total, *bytes_done, *bytes_total)),
                _ => None,
            })
            .expect("至少要有一次 Files 事件");
        assert_eq!(
            last,
            (1, 1, 0, 0),
            "跳过的文件算 done 不算字节，bytes_total 也不该把它算进去"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 混合批次：一个已缓存 + 一个注定失败的任务。
    /// 第一条进度事件就该把「跳过」算进 done，而 bytes_total 只算要下的那个。
    #[tokio::test]
    async fn run_batch_counts_skipped_in_done_but_not_in_bytes() {
        let dir = temp_dir("mixed");
        let good = "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d";
        let cached = dir.join("cached.bin");
        std::fs::write(&cached, b"hello").unwrap();

        // 把「父目录」做成普通文件：create_dir_all 必然失败，不用联网
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, b"not a directory").unwrap();
        let doomed = blocker.join("sub").join("nope.jar");

        let client = HttpClient::new().unwrap();
        let events = std::sync::Mutex::new(Vec::new());
        let sink = |progress: Progress| events.lock().unwrap().push(progress);

        let result = run_batch(
            &client,
            vec![
                job(cached.clone(), Some(good), Some(5)),
                job(doomed, Some("ff"), Some(1000)),
            ],
            2,
            &sink,
            StageKind::Libraries,
        )
        .await;
        assert!(result.is_err(), "注定失败的任务应该把错误抛出来");

        let events = events.into_inner().unwrap();
        let first = events
            .iter()
            .find_map(|p| match p {
                Progress::Files {
                    done,
                    total,
                    bytes_done,
                    bytes_total,
                    ..
                } => Some((*done, *total, *bytes_done, *bytes_total)),
                _ => None,
            })
            .expect("开始时就该有一次 Files 事件");
        assert_eq!(
            first,
            (1, 2, 0, 1000),
            "done 含已缓存文件，bytes_total 只含待下载文件"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 空任务列表不该发事件、也不该出错。
    #[tokio::test]
    async fn run_batch_with_no_jobs_is_noop() {
        let client = HttpClient::new().unwrap();
        let outcome = run_batch(&client, Vec::new(), 8, &crate::types::NullSink, StageKind::Assets)
            .await
            .unwrap();
        assert_eq!(outcome.downloaded, 0);
        assert_eq!(outcome.skipped, 0);
        assert_eq!(outcome.bytes, 0);
    }

    #[test]
    fn dedupe_keeps_first_job_per_dest() {
        let dir = temp_dir("dedupe");
        let jobs = vec![
            job(dir.join("a.jar"), Some("a"), Some(1)),
            job(dir.join("a.jar"), Some("b"), Some(2)),
            job(dir.join("b.jar"), None, None),
        ];
        let jobs = dedupe_by_dest(jobs);
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].sha1.as_deref(), Some("a"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extract_natives_skips_meta_inf_and_excluded() {
        let dir = temp_dir("natives");
        let jar = dir.join("natives.jar");
        {
            use std::io::Write as _;
            let file = std::fs::File::create(&jar).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for (name, body) in [
                ("META-INF/MANIFEST.MF", &b"Manifest-Version: 1.0"[..]),
                ("META-INF/versions/9/module-info.class", &b"junk"[..]),
                ("lwjgl.dll", &b"native"[..]),
                ("sub/dir/libfoo.so", &b"nested"[..]),
                ("excluded/skip.txt", &b"nope"[..]),
            ] {
                writer.start_file(name, options).unwrap();
                writer.write_all(body).unwrap();
            }
            writer.finish().unwrap();
        }

        let dest = dir.join("natives-out");
        let exclude = vec!["excluded/".to_string()];
        let count = extract_natives(&jar, &dest, &exclude).unwrap();

        assert_eq!(count, 2, "只该解出 lwjgl.dll 和 sub/dir/libfoo.so");
        assert_eq!(std::fs::read(dest.join("lwjgl.dll")).unwrap(), b"native");
        assert_eq!(
            std::fs::read(dest.join("sub").join("dir").join("libfoo.so")).unwrap(),
            b"nested"
        );
        assert!(!dest.join("META-INF").exists(), "META-INF 不该落盘");
        assert!(!dest.join("excluded").exists(), "extract.exclude 要生效");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn excluded_matches_directory_prefix() {
        let exclude = vec!["META-INF/".to_string(), "junk".to_string()];
        assert!(excluded("META-INF/MANIFEST.MF", &exclude));
        assert!(excluded("META-INF", &exclude));
        assert!(excluded("junk/a.txt", &exclude));
        assert!(!excluded("META-INF2/a.txt", &exclude));
        assert!(!excluded("lwjgl.dll", &exclude));
        assert!(!excluded("anything", &[]));
    }

    #[test]
    fn tmp_path_stays_next_to_destination() {
        let p = tmp_path(Path::new("libraries/a/b.jar"));
        assert_eq!(p, PathBuf::from("libraries/a/b.jar.part"));
    }

    /// 编译期契约：`Installer::run` 返回的 future 必须是 `Send`。
    ///
    /// Tauri 的 `async_runtime::spawn` 和任何 `tokio::spawn` 都要求 `Send`；
    /// 这个测试不需要运行，只要能在 `cargo test` 里编译过就说明契约没被破坏
    /// （历史上 `&mut dyn FnMut(u64)` 就踩过一次）。
    #[test]
    fn installer_run_future_is_send() {
        fn assert_send<F: Send>(_: F) {}

        let installer = crate::Installer::new().unwrap();
        let request = crate::types::InstallRequest::new(".", "1.21.1");
        let sink = crate::types::NullSink;
        assert_send(installer.run(&request, &sink));
    }

    // ------------------------------------------------------------ 真实网络
    //
    // 默认忽略，手动跑：`cargo test -p mcdl-core -- --ignored`

    /// 1.7 之后每个版本都带的 log4j 配置：888 字节，带官方 SHA1。
    const LOG4J_XML: &str = "https://piston-data.mojang.com/v1/objects/bd65e7d2e3c237be76cfbef4c2405033d7f91521/client-1.12.xml";
    const LOG4J_SHA1: &str = "bd65e7d2e3c237be76cfbef4c2405033d7f91521";

    #[tokio::test]
    #[ignore = "需要真实网络"]
    async fn download_real_file_then_skip_on_second_run() {
        let client = HttpClient::new().unwrap();
        let dir = temp_dir("real-small");
        let dest = dir.join("nested").join("client-1.12.xml");
        let job = DownloadJob {
            url: LOG4J_XML.to_string(),
            dest: dest.clone(),
            sha1: Some(LOG4J_SHA1.to_string()),
            size: Some(888),
            label: "client-1.12.xml".to_string(),
        };

        download_one(&client, &job).await.unwrap();
        assert!(dest.exists());
        assert_eq!(std::fs::metadata(&dest).unwrap().len(), 888);
        assert!(verify(&dest, job.sha1.as_deref()).unwrap());
        assert!(!tmp_path(&dest).exists(), "临时文件不能留在磁盘上");

        // 第二次应该直接跳过：把目标内容改坏再跑，会重新下载并恢复
        std::fs::write(&dest, b"corrupted").unwrap();
        assert!(!verify(&dest, job.sha1.as_deref()).unwrap());
        download_one(&client, &job).await.unwrap();
        assert!(verify(&dest, job.sha1.as_deref()).unwrap());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 官方 SHA1 故意写错 → 删掉重下一次 → 仍不一致就报 HashMismatch，且不留下坏文件。
    #[tokio::test]
    #[ignore = "需要真实网络"]
    async fn download_reports_hash_mismatch_after_retry() {
        let client = HttpClient::new().unwrap();
        let dir = temp_dir("real-mismatch");
        let dest = dir.join("client-1.12.xml");
        let job = DownloadJob {
            url: LOG4J_XML.to_string(),
            dest: dest.clone(),
            sha1: Some("0000000000000000000000000000000000000000".to_string()),
            size: None,
            label: "client-1.12.xml".to_string(),
        };

        let err = download_one(&client, &job).await.unwrap_err();
        assert!(
            matches!(err, Error::HashMismatch { .. }),
            "两次校验都不过应该报 HashMismatch，实际是：{err}"
        );
        assert!(!dest.exists(), "校验不过的文件不能落到目标位置");
        assert!(!tmp_path(&dest).exists(), "重试完要清掉临时文件");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 真实网络 + 真实大文件：把 1.21.1 的 client.jar（约 26 MB）下下来校验。
    #[tokio::test]
    #[ignore = "需要真实网络，会下载约 26 MB"]
    async fn download_real_client_jar() {
        use crate::types::Source;

        let client = HttpClient::new().unwrap();
        let manifest = crate::meta::fetch_manifest(&client, Source::Official)
            .await
            .unwrap();
        let summary = manifest.find("1.21.1").expect("清单里应该有 1.21.1");
        let detail = crate::meta::fetch_detail(&client, Source::Official, summary)
            .await
            .unwrap();
        let remote = detail.client.expect("1.21.1 必须有 client.jar");

        let dir = temp_dir("real-client");
        let dest = dir.join("1.21.1.jar");
        let job = DownloadJob {
            url: remote.url.clone(),
            dest: dest.clone(),
            sha1: remote.sha1.clone(),
            size: remote.size,
            label: "1.21.1.jar".to_string(),
        };

        let started = std::time::Instant::now();
        download_one(&client, &job).await.unwrap();
        let elapsed = started.elapsed();

        let size = std::fs::metadata(&dest).unwrap().len();
        assert_eq!(size, remote.size.unwrap(), "落盘大小要和官方一致");
        assert_eq!(
            sha1_file(&dest).unwrap(),
            remote.sha1.clone().unwrap(),
            "SHA1 要和官方一致"
        );
        println!(
            "client.jar {} 字节，下载 + 校验耗时 {:?}",
            size, elapsed
        );

        // 再跑一次必须走跳过分支（不联网）
        let again = std::time::Instant::now();
        download_one(&client, &job).await.unwrap();
        assert!(again.elapsed() < elapsed);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
