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
            loader: detect_loader_deep(&versions_dir, &id, 5).or_else(|| detect_loader(&id, &json)),
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

/// 从版本 JSON 的 `libraries` 里认加载器，返回 `(名字, 版本号)`。
///
/// 这是**唯一权威的依据**：加载器一定会在 libraries 里留下自己的坐标，
/// 而目录名可以随便改（用户自定义版本名、整合包自带名字都不可信）。
fn detect_from_libraries(json: &serde_json::Value) -> Option<(String, Option<String>)> {
    let libs = json.get("libraries")?.as_array()?;

    // 顺序有意义：neoforge 的坐标里也可能出现 forge 字样，先判 neo。
    const RULES: [(&str, &str); 5] = [
        ("net.neoforged:neoforge:", "NeoForge"),
        ("net.neoforged:forge:", "NeoForge"),
        ("net.fabricmc:fabric-loader:", "Fabric"),
        ("org.quiltmc:quilt-loader:", "Quilt"),
        ("net.minecraftforge:forge:", "Forge"),
    ];

    for lib in libs {
        let Some(name) = lib.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        for (prefix, label) in RULES {
            if let Some(rest) = name.strip_prefix(prefix) {
                // `group:artifact:version[:classifier]`
                let version = rest.split(':').next().filter(|v| !v.is_empty());
                return Some((label.to_string(), version.map(str::to_owned)));
            }
        }
        // OptiFine 作为库出现时（PCL / HMCL 装的实例常见）
        if name.to_ascii_lowercase().contains("optifine") {
            let version = name.rsplit(':').next().filter(|v| !v.is_empty());
            return Some(("OptiFine".to_string(), version.map(str::to_owned)));
        }
    }
    None
}

/// 沿 `inheritsFrom` 往上找加载器。
///
/// 整合包版本（以及 PCL 装的实例）通常只在自己的 JSON 里写
/// `inheritsFrom: "1.20.1-Forge_47.2.0"`，**加载器坐标在父版本里**。
/// 只看自己那一层的 libraries，就会把 Forge/Fabric 的整合包认成原版。
fn detect_loader_deep(versions_dir: &Path, id: &str, depth: usize) -> Option<String> {
    if depth == 0 {
        return None;
    }
    let dir = versions_dir.join(id);
    let json_path = find_version_json(&dir, id)?;
    let raw = std::fs::read(&json_path).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&raw).ok()?;

    if let Some((name, ver)) = detect_from_libraries(&json) {
        return Some(match ver {
            Some(v) => format!("{name} {v}"),
            None => name,
        });
    }

    // 自己这层没有，就去父版本里找
    let parent = json.get("inheritsFrom").and_then(|v| v.as_str())?;
    detect_loader_deep(versions_dir, parent, depth - 1)
}

/// 兜底：从目录名和 `mainClass` 里猜。
///
/// 只在 `libraries` 里找不到加载器坐标时才用——它不可靠（名字可以随便改），
/// 但比"认成原版"强。
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
    fn detects_loader_from_libraries_not_from_name() {
        // 用户把版本改名成 `我的整合包` 之后，目录名完全没有加载器线索，
        // 只有 libraries 里的坐标是可信的。
        let json = serde_json::json!({
            "id": "我的整合包",
            "libraries": [
                {"name": "net.minecraft:client:1.21.1"},
                {"name": "net.fabricmc:fabric-loader:0.19.5"}
            ]
        });
        assert_eq!(
            detect_from_libraries(&json),
            Some(("Fabric".to_string(), Some("0.19.5".to_string())))
        );
    }

    #[test]
    fn follows_inherits_from_to_find_loader() {
        // 整合包版本自己那层通常只有 inheritsFrom，加载器坐标在父版本里。
        let root = std::env::temp_dir().join(format!("mcdl-inst-{}", std::process::id()));
        let versions = root.join("versions");
        std::fs::create_dir_all(versions.join("pack")).unwrap();
        std::fs::create_dir_all(versions.join("1.20.1-Forge_47.2.0")).unwrap();
        std::fs::write(
            versions.join("pack/pack.json"),
            r#"{"id":"pack","inheritsFrom":"1.20.1-Forge_47.2.0","libraries":[]}"#,
        )
        .unwrap();
        std::fs::write(
            versions.join("1.20.1-Forge_47.2.0/1.20.1-Forge_47.2.0.json"),
            r#"{"id":"1.20.1-Forge_47.2.0","libraries":[{"name":"net.minecraftforge:forge:47.2.0"}]}"#,
        )
        .unwrap();

        assert_eq!(
            detect_loader_deep(&versions, "pack", 5).as_deref(),
            Some("Forge 47.2.0")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn neoforge_wins_over_forge_prefix() {
        let json = serde_json::json!({
            "libraries": [{"name": "net.neoforged:neoforge:21.1.264"}]
        });
        assert_eq!(
            detect_from_libraries(&json),
            Some(("NeoForge".to_string(), Some("21.1.264".to_string())))
        );
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
