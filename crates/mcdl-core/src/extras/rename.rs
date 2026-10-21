//! 自定义版本名：把 `versions/<自动名>/` 改成用户要的名字。
//!
//! 做法是「改目录名 + 改 JSON 里的 `id` + 把同名 jar 也改掉」，不是复制一份——
//! `libraries/`、`assets/` 都是共享的，复制只会白占空间。
//!
//! 关键约束：
//! - 名字先校验（空、路径分隔符、Windows 保留名、结尾的点/空格都会被系统静默
//!   处理成别的名字，必须提前拒绝）；
//! - 目标已存在时**不覆盖**，直接报错；
//! - 中途任何一步失败都要回滚到原名，不能把刚装好的版本搞丢。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, IoContext, Result};
use crate::paths::GamePaths;

/// 版本名长度上限。Windows 目录名本身能到 255，但版本名会在启动器列表里显示，
/// 太长没有意义；96 已经足够放下 `1.21.1-forge-52.1.20-整合包` 这种。
const MAX_NAME_CHARS: usize = 96;

/// Windows 文件名里不允许出现的字符。
const FORBIDDEN: [char; 9] = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];

/// 名字校验结果。界面拿它做即时反馈，`rename` 里还会再校验一次。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NameCheck {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// 校验一个版本名，返回可直接给用户看的原因。
pub fn validate_name(raw: &str) -> NameCheck {
    match check_name(raw) {
        Ok(()) => NameCheck {
            ok: true,
            reason: None,
        },
        Err(e) => NameCheck {
            ok: false,
            reason: Some(e.to_string()),
        },
    }
}

/// 真正的校验。注意**不 trim**：前后空格在 Windows 上会被系统吃掉，
/// 用户以为叫 `abc ` 实际叫 `abc`，这种歧义要在入口就拒绝。
pub fn check_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(Error::other("版本名不能为空"));
    }
    if name.trim() != name {
        return Err(Error::other("版本名前后不能有空格"));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(Error::other(format!("版本名最多 {MAX_NAME_CHARS} 个字符")));
    }
    if name == "." || name == ".." {
        return Err(Error::other("版本名不能是 . 或 .."));
    }
    if let Some(ch) = name
        .chars()
        .find(|c| FORBIDDEN.contains(c) || c.is_control())
    {
        return Err(Error::other(format!(
            "版本名里不能有 `{ch}`（不能含 / \\ : * ? \" < > | 和控制字符）"
        )));
    }
    if name.ends_with('.') {
        return Err(Error::other("版本名不能以点结尾"));
    }
    if is_reserved(name) {
        return Err(Error::other(format!(
            "`{name}` 是 Windows 的保留设备名，换一个"
        )));
    }
    Ok(())
}

/// Windows 保留设备名：CON / PRN / AUX / NUL / COM1-9 / LPT1-9（大小写不敏感，
/// 带扩展名也一样保留，例如 `con.json`）。
fn is_reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    let bytes = stem.as_bytes();
    bytes.len() == 4
        && (bytes.starts_with(b"COM") || bytes.starts_with(b"LPT"))
        && (b'1'..=b'9').contains(&bytes[3])
}

/// 改名结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameOutcome {
    pub from: String,
    pub to: String,
    /// 改完之后版本 JSON 的路径
    pub version_json: PathBuf,
    /// 版本目录里的同名 jar 也跟着改了（原版有，继承型的加载器版本没有）
    pub jar_renamed: bool,
}

impl RenameOutcome {
    /// 名字没变时的结果：什么都没动，但界面仍然可以显示「已经是这个名字」。
    pub fn unchanged(paths: &GamePaths, id: &str) -> Self {
        Self {
            from: id.to_string(),
            to: id.to_string(),
            version_json: paths.version_json(id),
            jar_renamed: false,
        }
    }
}

/// 把 `versions/<from>/` 改名为 `versions/<to>/`，并重写 JSON 里的 `id`。
///
/// 失败会回滚到 `from`；只有回滚也失败时才可能留下 `versions/<to>/`，
/// 那种情况下错误信息会写明当前实际目录名。
pub fn rename(paths: &GamePaths, from: &str, to: &str) -> Result<RenameOutcome> {
    let from = from.trim();
    let to = to.trim();
    // 源目录名来自磁盘，同样校验：这个函数不能成为路径穿越的入口。
    check_name(from)?;
    check_name(to)?;
    if from == to {
        return Ok(RenameOutcome::unchanged(paths, from));
    }

    let src = paths.version_dir(from);
    let dst = paths.version_dir(to);
    if !src.is_dir() {
        return Err(Error::other(format!("没有这个版本目录：versions/{from}")));
    }
    if dst.exists() {
        return Err(Error::other(format!(
            "versions/{to} 已经存在，换一个名字（不会覆盖）"
        )));
    }

    let src_json = paths.version_json(from);
    if !src_json.is_file() {
        return Err(Error::other(format!(
            "versions/{from}/ 里没有 {from}.json，不动它"
        )));
    }

    // 先把 JSON 读进内存：解析不了就直接报错，目录一步都不动。
    let raw = std::fs::read(&src_json).at(&src_json)?;
    let mut value: serde_json::Value = serde_json::from_slice(&raw).map_err(|e| {
        Error::other(format!(
            "versions/{from}/{from}.json 解析失败，没有改名：{e}"
        ))
    })?;

    std::fs::rename(&src, &dst).at(&src)?;

    match move_files(&dst, from, to, &mut value) {
        Ok(jar_renamed) => Ok(RenameOutcome {
            from: from.to_string(),
            to: to.to_string(),
            version_json: dst.join(format!("{to}.json")),
            jar_renamed,
        }),
        Err(e) => {
            let rollback = rollback(&dst, &src, from, to, &raw);
            match rollback {
                Ok(()) => Err(Error::other(format!(
                    "改名失败，已回滚到 {from}（版本还在）：{e}"
                ))),
                Err(rb) => Err(Error::other(format!(
                    "改名失败（{e}），回滚也失败了（{rb}）；版本现在可能在 versions/{to}，\
                     请手动把目录名和 JSON 里的 id 改回 {from}"
                ))),
            }
        }
    }
}

/// 目录已经改名之后的三步：JSON 改名 → 改 id 写回 → jar 改名。
fn move_files(dir: &Path, from: &str, to: &str, value: &mut serde_json::Value) -> Result<bool> {
    let old_json = dir.join(format!("{from}.json"));
    let new_json = dir.join(format!("{to}.json"));
    std::fs::rename(&old_json, &new_json).at(&old_json)?;

    if let Some(obj) = value.as_object_mut() {
        obj.insert("id".to_string(), serde_json::Value::String(to.to_string()));
    }
    std::fs::write(&new_json, serde_json::to_vec_pretty(value)?).at(&new_json)?;

    let old_jar = dir.join(format!("{from}.jar"));
    let new_jar = dir.join(format!("{to}.jar"));
    let mut jar_renamed = false;
    if old_jar.is_file() {
        std::fs::rename(&old_jar, &new_jar).at(&old_jar)?;
        jar_renamed = true;
    }
    Ok(jar_renamed)
}

/// 尽量把一切恢复成改名之前的样子。每一步都尽力而为，最后把目录改回原名。
fn rollback(dir: &Path, src: &Path, from: &str, to: &str, raw: &[u8]) -> Result<()> {
    let from_json = dir.join(format!("{from}.json"));
    let to_json = dir.join(format!("{to}.json"));

    // 1. JSON 文件改回原名，内容恢复成读到的原始字节
    if to_json.is_file() && !from_json.exists() {
        let _ = std::fs::rename(&to_json, &from_json);
    }
    std::fs::write(&from_json, raw).at(&from_json)?;

    // 2. jar 改回原名
    let from_jar = dir.join(format!("{from}.jar"));
    let to_jar = dir.join(format!("{to}.jar"));
    if to_jar.is_file() && !from_jar.exists() {
        let _ = std::fs::rename(&to_jar, &from_jar);
    }

    // 3. 目录改回原名
    std::fs::rename(dir, src).at(dir)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_game(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("mcdl-rename-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(dir.join("versions")).unwrap();
        dir
    }

    /// 造一个「原版」版本目录：JSON + 同名 jar。
    fn vanilla_version(root: &Path, id: &str) {
        let dir = root.join("versions").join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("{id}.json")),
            format!(
                r#"{{"id":"{id}","type":"release","mainClass":"net.minecraft.client.main.Main"}}"#
            ),
        )
        .unwrap();
        std::fs::write(dir.join(format!("{id}.jar")), b"fake jar").unwrap();
    }

    #[test]
    fn accepts_reasonable_names() {
        for name in [
            "我的世界 1.21",
            "1.21.1-fabric",
            "My Pack v2",
            "生存服-2026",
        ] {
            assert!(check_name(name).is_ok(), "{name} 应该被接受");
        }
    }

    #[test]
    fn rejects_empty_and_whitespace() {
        assert!(check_name("").is_err());
        assert!(check_name("   ").is_err());
        assert!(check_name(" abc").is_err(), "前导空格会被系统吃掉");
        assert!(check_name("abc ").is_err(), "尾随空格会被系统吃掉");
    }

    #[test]
    fn rejects_path_characters_and_traversal() {
        for bad in [
            "a/b", "a\\b", "a:b", "a*b", "a?b", "a\"b", "a<b", "a>b", "a|b", "a\nb", "..", ".",
            "abc.",
        ] {
            assert!(check_name(bad).is_err(), "{bad:?} 应该被拒绝");
        }
    }

    #[test]
    fn rejects_windows_reserved_names() {
        for bad in [
            "CON", "con", "PRN", "aux", "NUL", "COM1", "lpt9", "con.json",
        ] {
            assert!(check_name(bad).is_err(), "{bad} 应该被拒绝");
        }
        // COM0 / COM10 不是保留名
        assert!(check_name("COM0").is_ok());
        assert!(check_name("COM10").is_ok());
        assert!(check_name("CONSOLE").is_ok());
    }

    #[test]
    fn rejects_too_long_names() {
        assert!(check_name(&"a".repeat(MAX_NAME_CHARS)).is_ok());
        assert!(check_name(&"a".repeat(MAX_NAME_CHARS + 1)).is_err());
        // 按字符数算，不是字节数：40 个汉字不该被当成 120 字节拒绝
        assert!(check_name(&"名".repeat(40)).is_ok());
    }

    #[test]
    fn validate_name_reports_reason() {
        let ok = validate_name("我的世界");
        assert!(ok.ok);
        assert!(ok.reason.is_none());

        let bad = validate_name("a/b");
        assert!(!bad.ok);
        assert!(bad.reason.unwrap().contains('/'));
    }

    #[test]
    fn renames_directory_json_and_jar() {
        let root = temp_game("ok");
        vanilla_version(&root, "1.21.1");
        let paths = GamePaths::new(&root);

        let outcome = rename(&paths, "1.21.1", "我的世界").unwrap();
        assert_eq!(outcome.from, "1.21.1");
        assert_eq!(outcome.to, "我的世界");
        assert!(outcome.jar_renamed);

        let dir = root.join("versions").join("我的世界");
        assert!(dir.is_dir());
        assert!(!root.join("versions").join("1.21.1").exists());
        assert!(dir.join("我的世界.jar").is_file());
        assert!(!dir.join("1.21.1.jar").exists());
        assert!(dir.join("我的世界.json").is_file());

        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("我的世界.json")).unwrap()).unwrap();
        assert_eq!(json["id"], "我的世界");
        assert_eq!(json["type"], "release", "其它字段要原样保留");
        assert_eq!(json["mainClass"], "net.minecraft.client.main.Main");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 加载器版本没有同名 jar（`inheritsFrom` 指向原版），改名不能因此报错。
    #[test]
    fn renames_loader_version_without_jar() {
        let root = temp_game("nojar");
        let id = "fabric-loader-0.16.9-1.21.1";
        let dir = root.join("versions").join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("{id}.json")),
            format!(r#"{{"id":"{id}","inheritsFrom":"1.21.1"}}"#),
        )
        .unwrap();
        let paths = GamePaths::new(&root);

        let outcome = rename(&paths, id, "fabric-生存").unwrap();
        assert!(!outcome.jar_renamed);
        let json: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join("versions/fabric-生存/fabric-生存.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(json["id"], "fabric-生存");
        assert_eq!(json["inheritsFrom"], "1.21.1", "继承目标不能被改");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn refuses_to_overwrite_existing_version() {
        let root = temp_game("dup");
        vanilla_version(&root, "1.21.1");
        vanilla_version(&root, "已存在");
        let paths = GamePaths::new(&root);

        let err = rename(&paths, "1.21.1", "已存在").unwrap_err();
        assert!(
            err.to_string().contains("已经存在"),
            "错误要说明重名：{err}"
        );
        // 两边都还在，一个字节都没动
        assert!(root.join("versions/1.21.1/1.21.1.json").is_file());
        assert!(root.join("versions/已存在/已存在.json").is_file());
        let json: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join("versions/已存在/已存在.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(json["id"], "已存在", "目标版本的 id 不能被改");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn refuses_missing_source() {
        let root = temp_game("missing");
        let paths = GamePaths::new(&root);
        let err = rename(&paths, "1.21.1", "新名字").unwrap_err();
        assert!(err.to_string().contains("没有这个版本目录"), "{err}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 目录已经改过去了、但 JSON 改名失败时，必须把目录和 JSON 都回滚。
    #[test]
    fn rolls_back_when_json_step_fails() {
        let root = temp_game("rollback");
        vanilla_version(&root, "1.21.1");
        // 在源目录里放一个同名目录：`1.21.1.json` → `新名字.json` 会因为目标是目录而失败
        std::fs::create_dir_all(root.join("versions/1.21.1/新名字.json")).unwrap();
        let paths = GamePaths::new(&root);

        let err = rename(&paths, "1.21.1", "新名字").unwrap_err();
        assert!(err.to_string().contains("回滚"), "错误要说明回滚了：{err}");

        let src = root.join("versions/1.21.1");
        assert!(src.is_dir(), "目录必须回到原名");
        assert!(
            !root.join("versions/新名字").exists(),
            "失败后不能留下改了一半的目录"
        );
        assert!(src.join("1.21.1.json").is_file(), "JSON 必须回到原名");
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(src.join("1.21.1.json")).unwrap()).unwrap();
        assert_eq!(json["id"], "1.21.1", "回滚后 id 必须还是原名");
        assert!(src.join("1.21.1.jar").is_file(), "jar 也不能丢");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn same_name_is_noop() {
        let root = temp_game("same");
        vanilla_version(&root, "1.21.1");
        let paths = GamePaths::new(&root);
        let outcome = rename(&paths, "1.21.1", "1.21.1").unwrap();
        assert_eq!(outcome.to, "1.21.1");
        assert!(root.join("versions/1.21.1/1.21.1.json").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }
}
