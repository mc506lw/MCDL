//! 已安装版本的管理：列出、统计体积、删除。
//!
//! 这里的「已安装」是磁盘事实，不是我们自己的记账：只要 `versions/<id>/<id>.json`
//! 存在就算装过，不管它是我们装的还是别的启动器装的。

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::error::{Error, IoContext, Result};
use crate::paths::GamePaths;

/// 磁盘上的一个版本。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledVersion {
    /// `versions/` 下的目录名，也是删除时用的标识
    pub id: String,
    /// release / snapshot / old_beta / old_alpha，读不出来就留空
    pub kind: String,
    /// 认出来的模组加载器，认不出就是 None（原版）
    pub loader: Option<String>,
    pub main_class: String,
    /// 这个版本目录本身占多少字节
    pub bytes: u64,
    pub files: u64,
    /// 目录的最后修改时间（Unix 秒），用来排序
    pub modified: u64,
}

impl InstalledVersion {
    pub fn is_loader(&self) -> bool {
        self.loader.is_some()
    }
}

/// 扫描 `versions/`，列出所有能认出来的版本，最近的排在前面。
///
/// 认不出 JSON 的目录会被跳过——它们多半是装了一半的残留，
/// 与其列出来让人困惑，不如不显示（删除仍然可以按目录名做）。
pub fn list(game_dir: &Path) -> Result<Vec<InstalledVersion>> {
    let paths = GamePaths::new(game_dir);
    let versions_dir = paths.versions();
    let Ok(entries) = std::fs::read_dir(&versions_dir) else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(json_path) = find_version_json(&dir, &id) else {
            continue;
        };
        let Ok(raw) = std::fs::read(&json_path) else {
            continue;
        };
        let Ok(json) = serde_json::from_slice::<serde_json::Value>(&raw) else {
            continue;
        };

        let (bytes, files) = dir_size(&dir);
        out.push(InstalledVersion {
            kind: str_of(&json, "type").unwrap_or_default(),
            main_class: str_of(&json, "mainClass").unwrap_or_default(),
            loader: detect_loader(&id, &json),
            id,
            bytes,
            files,
            modified: modified_secs(&dir),
        });
    }

    out.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.id.cmp(&b.id)));
    Ok(out)
}

/// 删掉一个版本目录。
///
/// 只接受 `versions/` 下的直接子目录名，不接受任何带路径分隔符的输入——
/// 这个函数会被 GUI 直接调用，不能指望调用方先做校验。
pub fn remove(game_dir: &Path, id: &str) -> Result<()> {
    if id.is_empty() || id.contains(['/', '\\']) || id == "." || id == ".." {
        return Err(Error::other(format!("版本名不合法：{id}")));
    }

    let dir: PathBuf = GamePaths::new(game_dir).version_dir(id);
    if !dir.is_dir() {
        return Err(Error::other(format!("没有这个版本：{id}")));
    }

    std::fs::remove_dir_all(&dir).at(&dir)?;
    Ok(())
}

/// 找版本 JSON：优先 `<id>.json`，找不到就取目录里唯一的那个 `.json`。
fn find_version_json(dir: &Path, id: &str) -> Option<PathBuf> {
    let exact = dir.join(format!("{id}.json"));
    if exact.is_file() {
        return Some(exact);
    }
    let mut found = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect::<Vec<_>>();
    if found.len() == 1 {
        found.pop()
    } else {
        None
    }
}

fn str_of(json: &serde_json::Value, key: &str) -> Option<String> {
    json.get(key)?.as_str().map(str::to_owned)
}

/// 从目录名和 `mainClass` 里认加载器。
///
/// 目录名是首选依据（各 installer 的命名稳定），`mainClass` 只做兜底——
/// 因为它会随加载器大版本变化。
fn detect_loader(id: &str, json: &serde_json::Value) -> Option<String> {
    let lower = id.to_ascii_lowercase();
    for (needle, name) in [
        ("fabric-loader", "Fabric"),
        ("quilt-loader", "Quilt"),
        ("neoforge", "NeoForge"),
        ("-forge-", "Forge"),
    ] {
        if lower.contains(needle) {
            return Some(name.to_string());
        }
    }

    let main = str_of(json, "mainClass")
        .unwrap_or_default()
        .to_ascii_lowercase();
    for (needle, name) in [
        ("fabricmc", "Fabric"),
        ("quiltmc", "Quilt"),
        ("neoforged", "NeoForge"),
        ("minecraftforge", "Forge"),
    ] {
        if main.contains(needle) {
            return Some(name.to_string());
        }
    }
    None
}

fn dir_size(dir: &Path) -> (u64, u64) {
    let mut bytes = 0u64;
    let mut files = 0u64;
    let mut stack = vec![dir.to_path_buf()];
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

fn modified_secs(dir: &Path) -> u64 {
    std::fs::metadata(dir)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_path_traversal() {
        let root = Path::new("C:/mc");
        for bad in ["../etc", "a/b", "a\\b", "..", "."] {
            assert!(remove(root, bad).is_err(), "{bad} 应该被拒绝");
        }
    }

    #[test]
    fn detects_loader_from_id() {
        let empty = serde_json::json!({});
        assert_eq!(
            detect_loader("fabric-loader-0.19.5-1.21.1", &empty).as_deref(),
            Some("Fabric")
        );
        assert_eq!(
            detect_loader("1.20.4-forge-49.2.13", &empty).as_deref(),
            Some("Forge")
        );
        assert_eq!(
            detect_loader("neoforge-21.1.264", &empty).as_deref(),
            Some("NeoForge")
        );
        assert_eq!(detect_loader("1.21.1", &empty), None);
    }

    #[test]
    fn detects_loader_from_main_class_as_fallback() {
        let json = serde_json::json!({
            "mainClass": "net.fabricmc.loader.impl.launch.knot.KnotClient"
        });
        assert_eq!(
            detect_loader("weird-name", &json).as_deref(),
            Some("Fabric")
        );
    }
}
