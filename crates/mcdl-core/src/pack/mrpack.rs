//! `.mrpack` 与 CurseForge 整合包 zip 的格式解析。
//!
//! 格式依据 Modrinth 的公开文档（`modrinth.index.json`、`overrides/`）和
//! CurseForge 的 `manifest.json`，这里只实现安装需要的那部分：
//! 依赖声明、文件清单、覆盖目录解压。
//!
//! 两个格式的压缩包都是普通 zip（deflate），不需要额外特性。

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, IoContext, Result};
use crate::types::{LoaderKind, LoaderSpec};

/// Modrinth 整合包索引的文件名。
pub const INDEX_NAME: &str = "modrinth.index.json";
/// CurseForge 整合包清单的文件名。
pub const CURSE_MANIFEST_NAME: &str = "manifest.json";

/// `overrides/` 对所有端生效。
pub const OVERRIDES_DIR: &str = "overrides";
/// `client-overrides/` 只对客户端生效。
pub const CLIENT_OVERRIDES_DIR: &str = "client-overrides";
/// `server-overrides/` 只对服务端生效，客户端安装必须跳过。
pub const SERVER_OVERRIDES_DIR: &str = "server-overrides";

// ---------------------------------------------------------------- Modrinth

/// `modrinth.index.json`。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ModpackIndex {
    #[serde(default, rename = "formatVersion")]
    pub format_version: u32,
    #[serde(default)]
    pub game: String,
    #[serde(default, rename = "versionId")]
    pub version_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub files: Vec<ModpackFile>,
    /// `minecraft` / `fabric-loader` / `forge` / `neoforge` / `quilt-loader`
    #[serde(default)]
    pub dependencies: HashMap<String, String>,
}

/// 索引里的一个文件。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ModpackFile {
    /// 相对游戏目录的路径，例如 `mods/sodium.jar`
    pub path: String,
    #[serde(default)]
    pub hashes: ModpackHashes,
    #[serde(default)]
    pub env: Option<ModpackEnv>,
    /// 下载地址，通常一条；多条时按顺序试
    #[serde(default)]
    pub downloads: Vec<String>,
    #[serde(default, rename = "fileSize")]
    pub file_size: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ModpackHashes {
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub sha512: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ModpackEnv {
    /// required / optional / unsupported
    #[serde(default)]
    pub client: Option<String>,
    #[serde(default)]
    pub server: Option<String>,
}

impl ModpackFile {
    /// 客户端要不要装它。`env.client == "unsupported"` 是服务端专用模组。
    pub fn needed_on_client(&self) -> bool {
        !matches!(
            self.env.as_ref().and_then(|e| e.client.as_deref()),
            Some("unsupported")
        )
    }

    /// 第一个非空下载地址。
    pub fn url(&self) -> Option<&str> {
        self.downloads
            .iter()
            .map(|u| u.trim())
            .find(|u| !u.is_empty())
    }

    /// 校验过的相对路径。
    pub fn safe_path(&self) -> Result<PathBuf> {
        relative_path(&self.path)
    }
}

impl ModpackIndex {
    /// 这个包要的 Minecraft 版本。
    pub fn mc_version(&self) -> Result<String> {
        self.dependencies
            .get("minecraft")
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                Error::other(format!(
                    "{INDEX_NAME} 没有声明 minecraft 版本，装不了"
                ))
            })
    }

    /// 这个包要的加载器。原版整合包返回 `None`。
    pub fn loader_spec(&self) -> Result<Option<LoaderSpec>> {
        let mc_version = self.mc_version()?;
        // 键名来自 Modrinth 的公开文档；顺序无关，命中哪个算哪个。
        for (key, kind) in [
            ("fabric-loader", LoaderKind::Fabric),
            ("quilt-loader", LoaderKind::Quilt),
            ("forge", LoaderKind::Forge),
            ("neoforge", LoaderKind::NeoForge),
        ] {
            if let Some(version) = self.dependencies.get(key) {
                let version = version.trim();
                if version.is_empty() {
                    continue;
                }
                return Ok(Some(LoaderSpec {
                    kind,
                    mc_version,
                    loader_version: Some(version.to_string()),
                }));
            }
        }
        Ok(None)
    }
}

// ---------------------------------------------------------------- CurseForge

/// CurseForge 整合包 zip 里的 `manifest.json`。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct CurseManifest {
    #[serde(default)]
    pub minecraft: CurseMinecraft,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub files: Vec<CurseFileRef>,
    /// 覆盖文件所在的目录名，官方是 `overrides`
    #[serde(default)]
    pub overrides: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct CurseMinecraft {
    #[serde(default)]
    pub version: String,
    #[serde(default, rename = "modLoaders")]
    pub mod_loaders: Vec<CurseLoader>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct CurseLoader {
    /// 形如 `fabric-0.16.9`、`forge-47.2.0`、`neoforge-21.1.264`
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub primary: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct CurseFileRef {
    #[serde(default, rename = "projectID")]
    pub project_id: u64,
    #[serde(default, rename = "fileID")]
    pub file_id: u64,
    #[serde(default)]
    pub required: bool,
}

impl CurseManifest {
    /// `fabric-0.16.9` → `LoaderSpec { Fabric, mc, Some("0.16.9") }`。
    ///
    /// 只认 `primary` 的那一条；没有 primary 就取第一条。
    pub fn loader_spec(&self) -> Result<Option<LoaderSpec>> {
        let mc = self.minecraft.version.trim();
        if mc.is_empty() {
            return Err(Error::other(format!(
                "{CURSE_MANIFEST_NAME} 没有声明 Minecraft 版本，装不了"
            )));
        }
        let picked = self
            .minecraft
            .mod_loaders
            .iter()
            .find(|l| l.primary)
            .or_else(|| self.minecraft.mod_loaders.first());
        let Some(loader) = picked else {
            return Ok(None);
        };
        let (kind, version) = split_loader_id(&loader.id).ok_or_else(|| {
            Error::other(format!(
                "认不出 CurseForge 清单里的加载器：{}",
                loader.id
            ))
        })?;
        Ok(Some(LoaderSpec {
            kind,
            mc_version: mc.to_string(),
            loader_version: version,
        }))
    }

    /// 覆盖文件目录名（清单里可改，默认 `overrides`）。
    pub fn overrides_dir(&self) -> String {
        let raw = self.overrides.trim();
        if raw.is_empty() {
            OVERRIDES_DIR.to_string()
        } else {
            raw.trim_matches('/').to_string()
        }
    }
}

/// `fabric-0.16.9` → `(Fabric, Some("0.16.9"))`。
pub fn split_loader_id(raw: &str) -> Option<(LoaderKind, Option<String>)> {
    let text = raw.trim();
    let (head, tail) = match text.split_once('-') {
        Some((h, t)) => (h, Some(t.trim())),
        None => (text, None),
    };
    let kind = LoaderKind::parse(head)?;
    let version = tail.filter(|v| !v.is_empty()).map(str::to_string);
    Some((kind, version))
}

// ---------------------------------------------------------------- 路径

/// 把整合包里的相对路径变成安全的 [`PathBuf`]。
///
/// 拒绝：绝对路径、盘符、`..`、空路径。整合包是外部数据，不能直接 join。
pub fn relative_path(raw: &str) -> Result<PathBuf> {
    let text = raw.trim().replace('\\', "/");
    if text.is_empty() {
        return Err(Error::other("整合包里有一个空路径"));
    }
    if text.starts_with('/') || text.contains(':') {
        return Err(Error::other(format!(
            "整合包里的路径不是相对路径，拒绝写入：{raw}"
        )));
    }

    let mut out = PathBuf::new();
    for segment in text.split('/') {
        match segment {
            "" | "." => continue,
            ".." => {
                return Err(Error::other(format!(
                    "整合包里的路径试图跳出游戏目录，拒绝写入：{raw}"
                )))
            }
            part => out.push(part),
        }
    }
    if out.as_os_str().is_empty() {
        return Err(Error::other(format!("整合包里的路径是空的：{raw}")));
    }
    Ok(out)
}

// ---------------------------------------------------------------- 读索引

/// 从压缩包里读一个文本条目。条目不存在返回 `Ok(None)`。
pub fn read_entry(archive: &Path, name: &str) -> Result<Option<Vec<u8>>> {
    let file = std::fs::File::open(archive).at(archive)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| {
        Error::other(format!(
            "打不开整合包压缩包（{}）：{e}",
            archive.display()
        ))
    })?;

    let mut entry = match zip.by_name(name) {
        Ok(entry) => entry,
        Err(_) => return Ok(None),
    };
    let mut buf = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut buf).map_err(|e| {
        Error::other(format!(
            "读取整合包里的 {name} 失败（{}）：{e}",
            archive.display()
        ))
    })?;
    Ok(Some(buf))
}

/// 读并解析 `modrinth.index.json`。
pub fn read_index(archive: &Path) -> Result<ModpackIndex> {
    let raw = read_entry(archive, INDEX_NAME)?.ok_or_else(|| {
        Error::other(format!(
            "这个压缩包里没有 {INDEX_NAME}，不是 Modrinth 格式的整合包"
        ))
    })?;
    parse_index(&raw)
}

/// 解析索引内容（单测直接喂 JSON 走这里，不用造 zip）。
pub fn parse_index(raw: &[u8]) -> Result<ModpackIndex> {
    let index: ModpackIndex = serde_json::from_slice(raw)
        .map_err(|e| Error::other(format!("{INDEX_NAME} 解析失败：{e}")))?;
    if index.files.is_empty() && index.dependencies.is_empty() {
        return Err(Error::other(format!(
            "{INDEX_NAME} 里既没有 files 也没有 dependencies，这个包是空的"
        )));
    }
    // 路径安全先验一遍：别等到下载了一半才发现某个条目想写到 C:\Windows。
    for file in &index.files {
        file.safe_path()?;
    }
    Ok(index)
}

/// 读并解析 CurseForge 的 `manifest.json`。
pub fn read_curse_manifest(archive: &Path) -> Result<CurseManifest> {
    let raw = read_entry(archive, CURSE_MANIFEST_NAME)?.ok_or_else(|| {
        Error::other(format!(
            "这个压缩包里没有 {CURSE_MANIFEST_NAME}，不是 CurseForge 格式的整合包"
        ))
    })?;
    parse_curse_manifest(&raw)
}

pub fn parse_curse_manifest(raw: &[u8]) -> Result<CurseManifest> {
    let manifest: CurseManifest = serde_json::from_slice(raw)
        .map_err(|e| Error::other(format!("{CURSE_MANIFEST_NAME} 解析失败：{e}")))?;
    if manifest.minecraft.version.trim().is_empty() {
        return Err(Error::other(format!(
            "{CURSE_MANIFEST_NAME} 里没有 minecraft.version"
        )));
    }
    Ok(manifest)
}

// ---------------------------------------------------------------- 解压覆盖

/// 把 `overrides/`、`client-overrides/`（或 CurseForge 清单里指定的目录）解到游戏目录。
///
/// - 只解这些前缀下的条目，`modrinth.index.json`、`manifest.json`、`server-overrides/` 一律不落盘；
/// - 路径逐条过 [`relative_path`]，穿越路径直接跳过并记 warn（一个坏条目不该毁掉整个安装）；
/// - 已存在的文件会被覆盖：整合包作者改过的配置就该覆盖掉旧的。
///
/// 返回写入的文件数。
pub fn extract_overrides(archive: &Path, game_dir: &Path, dirs: &[&str]) -> Result<usize> {
    let file = std::fs::File::open(archive).at(archive)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| {
        Error::other(format!(
            "打不开整合包压缩包（{}）：{e}",
            archive.display()
        ))
    })?;

    std::fs::create_dir_all(game_dir).at(game_dir)?;
    let mut written = 0usize;
    let mut skipped = 0usize;

    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|e| {
            Error::other(format!(
                "读取整合包条目失败（{} 第 {index} 项）：{e}",
                archive.display()
            ))
        })?;
        if entry.is_dir() {
            continue;
        }

        // `enclosed_name` 会挡掉绝对路径和 `..`；这里还要按前缀挑目录。
        let Some(name) = entry
            .enclosed_name()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
        else {
            tracing::warn!(entry = entry.name(), "条目路径不安全，跳过");
            skipped += 1;
            continue;
        };

        let Some(relative) = dirs.iter().find_map(|dir| {
            name.strip_prefix(dir)
                .and_then(|rest| rest.strip_prefix('/'))
                .map(str::to_string)
        }) else {
            continue;
        };

        let rel = match relative_path(&relative) {
            Ok(path) => path,
            Err(e) => {
                tracing::warn!(entry = %name, error = %e, "覆盖文件路径不安全，跳过");
                skipped += 1;
                continue;
            }
        };

        let out = game_dir.join(&rel);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).at(parent)?;
        }
        let mut writer = std::fs::File::create(&out).at(&out)?;
        std::io::copy(&mut entry, &mut writer).at(&out)?;
        written += 1;
    }

    tracing::info!(
        archive = %archive.display(),
        written,
        skipped,
        "整合包覆盖文件已解压"
    );
    Ok(written)
}

/// 压缩包里覆盖目录的候选前缀（按优先级）。
pub fn default_override_dirs() -> Vec<&'static str> {
    vec![OVERRIDES_DIR, CLIENT_OVERRIDES_DIR]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index_json() -> &'static [u8] {
        br#"{
            "formatVersion": 1,
            "game": "minecraft",
            "versionId": "1.0.0",
            "name": "Test Pack",
            "summary": "demo",
            "files": [
                {
                    "path": "mods/sodium.jar",
                    "hashes": { "sha1": "abc", "sha512": "def" },
                    "env": { "client": "required", "server": "unsupported" },
                    "downloads": ["https://example.invalid/sodium.jar"],
                    "fileSize": 123
                },
                {
                    "path": "mods/serveronly.jar",
                    "hashes": { "sha1": "aaa" },
                    "env": { "client": "unsupported", "server": "required" },
                    "downloads": ["https://example.invalid/serveronly.jar"]
                }
            ],
            "dependencies": { "minecraft": "1.21.1", "fabric-loader": "0.16.9" }
        }"#
    }

    #[test]
    fn parses_index_and_dependencies() {
        let index = parse_index(index_json()).unwrap();
        assert_eq!(index.name, "Test Pack");
        assert_eq!(index.version_id, "1.0.0");
        assert_eq!(index.files.len(), 2);
        assert_eq!(index.mc_version().unwrap(), "1.21.1");

        let spec = index.loader_spec().unwrap().unwrap();
        assert_eq!(spec.kind, LoaderKind::Fabric);
        assert_eq!(spec.mc_version, "1.21.1");
        assert_eq!(spec.loader_version.as_deref(), Some("0.16.9"));
    }

    #[test]
    fn client_side_filter_drops_server_only_files() {
        let index = parse_index(index_json()).unwrap();
        let needed: Vec<&str> = index
            .files
            .iter()
            .filter(|f| f.needed_on_client())
            .map(|f| f.path.as_str())
            .collect();
        assert_eq!(needed, ["mods/sodium.jar"]);
    }

    #[test]
    fn file_url_skips_blank_entries() {
        let file = ModpackFile {
            downloads: vec!["".into(), "  ".into(), "https://example.invalid/x.jar".into()],
            ..Default::default()
        };
        assert_eq!(file.url(), Some("https://example.invalid/x.jar"));

        let none = ModpackFile::default();
        assert_eq!(none.url(), None);
    }

    #[test]
    fn vanilla_pack_has_no_loader() {
        let raw = br#"{"formatVersion":1,"files":[{"path":"mods/a.jar","downloads":["https://x/a.jar"]}],"dependencies":{"minecraft":"1.20.1"}}"#;
        let index = parse_index(raw).unwrap();
        assert!(index.loader_spec().unwrap().is_none());
    }

    #[test]
    fn every_loader_key_maps_to_a_loader() {
        for (key, kind) in [
            ("fabric-loader", LoaderKind::Fabric),
            ("quilt-loader", LoaderKind::Quilt),
            ("forge", LoaderKind::Forge),
            ("neoforge", LoaderKind::NeoForge),
        ] {
            let raw = format!(
                r#"{{"files":[{{"path":"mods/a.jar","downloads":["https://x/a.jar"]}}],"dependencies":{{"minecraft":"1.21.1","{key}":"9.9.9"}}}}"#
            );
            let index = parse_index(raw.as_bytes()).unwrap();
            let spec = index.loader_spec().unwrap().unwrap();
            assert_eq!(spec.kind, kind, "键 {key} 应该映射到 {kind:?}");
            assert_eq!(spec.loader_version.as_deref(), Some("9.9.9"));
        }
    }

    #[test]
    fn missing_minecraft_version_is_an_error() {
        let raw = br#"{"files":[{"path":"mods/a.jar","downloads":["https://x/a.jar"]}],"dependencies":{}}"#;
        let index = parse_index(raw).unwrap();
        let err = index.mc_version().unwrap_err().to_string();
        assert!(err.contains("minecraft"), "错误要说清缺什么：{err}");
    }

    #[test]
    fn empty_index_is_an_error() {
        let err = parse_index(br#"{"formatVersion":1}"#).unwrap_err().to_string();
        assert!(err.contains("空的"), "实际是：{err}");
    }

    #[test]
    fn malformed_index_reports_the_file_name() {
        let err = parse_index(b"{ not json").unwrap_err().to_string();
        assert!(err.contains(INDEX_NAME), "实际是：{err}");
    }

    #[test]
    fn path_traversal_is_rejected() {
        let raw = br#"{"files":[{"path":"../../windows/system32/evil.dll","downloads":["https://x/e"]}],"dependencies":{"minecraft":"1.21.1"}}"#;
        let err = parse_index(raw).unwrap_err().to_string();
        assert!(err.contains("跳出游戏目录"), "实际是：{err}");

        let raw = br#"{"files":[{"path":"C:/windows/evil.dll","downloads":["https://x/e"]}],"dependencies":{"minecraft":"1.21.1"}}"#;
        let err = parse_index(raw).unwrap_err().to_string();
        assert!(err.contains("不是相对路径"), "实际是：{err}");
    }

    #[test]
    fn relative_path_normalizes_separators() {
        let p = relative_path("mods\\sub\\a.jar").unwrap();
        assert!(p.ends_with("mods/sub/a.jar") || p.ends_with("mods\\sub\\a.jar"));
        let p = relative_path("./mods//a.jar").unwrap();
        assert!(p.ends_with("mods/a.jar") || p.ends_with("mods\\a.jar"));
        assert!(relative_path("   ").is_err());
        assert!(relative_path("/etc/passwd").is_err());
        assert!(relative_path("mods/../../x").is_err());
    }

    #[test]
    fn curse_manifest_loaders_and_overrides() {
        let raw = br#"{
            "minecraft": {
                "version": "1.21.1",
                "modLoaders": [{ "id": "fabric-0.16.9", "primary": true }]
            },
            "name": "CF Pack",
            "version": "2.1",
            "overrides": "overrides",
            "files": [{ "projectID": 1, "fileID": 2, "required": true }]
        }"#;
        let manifest = parse_curse_manifest(raw).unwrap();
        let spec = manifest.loader_spec().unwrap().unwrap();
        assert_eq!(spec.kind, LoaderKind::Fabric);
        assert_eq!(spec.loader_version.as_deref(), Some("0.16.9"));
        assert_eq!(manifest.overrides_dir(), "overrides");
        assert_eq!(manifest.files.len(), 1);
        assert!(manifest.files[0].required);
    }

    #[test]
    fn curse_manifest_without_loader_is_vanilla() {
        let raw = br#"{"minecraft":{"version":"1.20.1","modLoaders":[]},"files":[]}"#;
        let manifest = parse_curse_manifest(raw).unwrap();
        assert!(manifest.loader_spec().unwrap().is_none());
        assert_eq!(manifest.overrides_dir(), OVERRIDES_DIR);
    }

    #[test]
    fn curse_manifest_without_mc_version_is_an_error() {
        let err = parse_curse_manifest(br#"{"minecraft":{}}"#)
            .unwrap_err()
            .to_string();
        assert!(err.contains("minecraft.version"), "实际是：{err}");
    }

    #[test]
    fn split_loader_id_handles_all_four() {
        assert_eq!(
            split_loader_id("forge-47.2.0"),
            Some((LoaderKind::Forge, Some("47.2.0".into())))
        );
        assert_eq!(
            split_loader_id("neoforge-21.1.264"),
            Some((LoaderKind::NeoForge, Some("21.1.264".into())))
        );
        assert_eq!(
            split_loader_id("quilt-0.27.0"),
            Some((LoaderKind::Quilt, Some("0.27.0".into())))
        );
        assert_eq!(split_loader_id("fabric"), Some((LoaderKind::Fabric, None)));
        assert_eq!(split_loader_id("liteloader-1.0"), None);
    }

    /// 造一个真实 zip，验证覆盖目录解压：只解 overrides / client-overrides，
    /// 跳过 server-overrides、索引本身，以及想穿越的条目。
    #[test]
    fn extract_overrides_writes_only_client_side_entries() {
        use std::io::Write as _;

        let dir = std::env::temp_dir().join(format!(
            "mcdl-mrpack-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let archive = dir.join("pack.mrpack");
        {
            let file = std::fs::File::create(&archive).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for (name, body) in [
                (INDEX_NAME, &br#"{"files":[],"dependencies":{"minecraft":"1.21.1"}}"#[..]),
                ("overrides/config/a.json", &br#"{"a":1}"#[..]),
                ("client-overrides/options.txt", &b"fov:70"[..]),
                ("server-overrides/server.properties", &b"motd=x"[..]),
                ("mods/outside.jar", &b"nope"[..]),
                ("overrides/../evil.txt", &b"nope"[..]),
            ] {
                writer.start_file(name, options).unwrap();
                writer.write_all(body).unwrap();
            }
            writer.finish().unwrap();
        }

        let game_dir = dir.join("game");
        let written =
            extract_overrides(&archive, &game_dir, &default_override_dirs()).unwrap();

        assert_eq!(written, 2, "只该写 overrides 和 client-overrides 里的两个文件");
        assert_eq!(
            std::fs::read_to_string(game_dir.join("config").join("a.json")).unwrap(),
            r#"{"a":1}"#
        );
        assert_eq!(
            std::fs::read_to_string(game_dir.join("options.txt")).unwrap(),
            "fov:70"
        );
        assert!(!game_dir.join("server.properties").exists(), "服务端覆盖不该落盘");
        assert!(!game_dir.join("mods").exists(), "覆盖目录之外的文件不该落盘");
        assert!(!game_dir.join("evil.txt").exists(), "穿越路径必须挡住");
        assert!(!game_dir.join(INDEX_NAME).exists(), "索引本身不是覆盖文件");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 索引不存在时要说清是哪个文件缺了，而不是抛一个 zip 的内部错误。
    #[test]
    fn read_index_reports_missing_entry() {
        use std::io::Write as _;

        let dir = std::env::temp_dir().join(format!(
            "mcdl-mrpack-missing-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let archive = dir.join("not-a-pack.zip");
        {
            let file = std::fs::File::create(&archive).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            writer
                .start_file("readme.txt", zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"hi").unwrap();
            writer.finish().unwrap();
        }

        let err = read_index(&archive).unwrap_err().to_string();
        assert!(err.contains(INDEX_NAME), "实际是：{err}");

        let err = read_curse_manifest(&archive).unwrap_err().to_string();
        assert!(err.contains(CURSE_MANIFEST_NAME), "实际是：{err}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 不是 zip 的文件：错误里要带路径，方便用户知道是哪个包坏了。
    #[test]
    fn read_index_rejects_non_zip() {
        let dir = std::env::temp_dir().join(format!(
            "mcdl-mrpack-notzip-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let archive = dir.join("broken.mrpack");
        std::fs::write(&archive, b"this is not a zip").unwrap();

        let err = read_index(&archive).unwrap_err().to_string();
        assert!(
            err.contains("打不开整合包压缩包"),
            "错误要说明压缩包打不开，实际是：{err}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
