//! 版本隔离：模组该落在哪。
//!
//! 隔离是**启动期**语义：游戏自己不会去别的地方找 `mods/`，是启动器按版本决定
//! 加载哪个目录。所以 MCDL 能做的只有两件事：
//!
//! 1. 装的时候把文件放到对应位置 —— 全仓只有 [`mods_dir`] 这一个函数算落点，
//!    任何调用点都不许自己拼 `versions/<id>/mods`；
//! 2. 在界面上说清楚这份文件只有 PCL / HMCL 这类支持隔离的启动器读得到
//!    （[`ISOLATION_NOTICE`]）。
//!
//! 官方启动器只读 `<gameDir>/mods/`，它不认识隔离目录。MCDL 不启动游戏，
//! 也不假装能改变这一点：把开关关掉，或者用支持隔离的启动器，二选一。
//!
//! 布局对齐 PCL / HMCL 的约定：
//!
//! ```text
//! 开：<gameDir>/versions/<版本id>/mods/
//! 关：<gameDir>/mods/
//! ```

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// 默认值：**开**。
///
/// 用户要的是「同目录装第二个整合包不能混」，所以默认就该是隔离的。请求结构体上的
/// `#[serde(default = "crate::isolate::default_on")]` 也用它，保证「字段没传」和
/// 「界面上没动过开关」是同一个结果。
pub fn default_on() -> bool {
    true
}

/// 开关旁边常驻的那句话。前端 `app/src/lib/isolate.ts` 里有同一份文案。
pub const ISOLATION_NOTICE: &str =
    "隔离后的模组只有 PCL / HMCL 这类支持版本隔离的启动器能读到；官方启动器只读根目录的 mods/。";

/// 共享目录名。游戏本体按这个名字找。
const MODS: &str = "mods";
/// 版本目录名。
const VERSIONS: &str = "versions";

/// 一次落点计算的结果。
///
/// `relative` 是给人看的（日志、界面），统一用 `/` 分隔，不依赖平台。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModsTarget {
    /// 绝对路径（或调用方给的 gameDir 拼出来的路径）
    pub dir: PathBuf,
    /// 这次用的是隔离目录还是共享目录
    pub isolated: bool,
    /// 相对 gameDir 的位置，例如 `versions/fabric-loader-0.16.9-1.21.1/mods`
    pub relative: String,
}

impl ModsTarget {
    /// 拼一个文件名进去。调用方只负责给安全的文件名（不含分隔符）。
    pub fn join(&self, file_name: &str) -> PathBuf {
        self.dir.join(file_name)
    }

    /// 日志/界面里用的一行：`versions/<id>/mods/` 或 `mods/`。
    pub fn label(&self) -> String {
        format!("{}/", self.relative)
    }

    /// 落点在隔离目录里、但这次是给官方启动器看的？—— 只用于提示文案。
    pub fn explain(&self) -> &'static str {
        if self.isolated {
            "隔离：只有支持版本隔离的启动器（PCL / HMCL 等）会读这个目录"
        } else {
            "共享：所有启动器都读根目录的 mods/，多个整合包会混在一起"
        }
    }
}

/// **唯一的落点计算入口。**
///
/// - `isolated == true`：`<gameDir>/versions/<version_id>/mods/`
/// - `isolated == false`：`<gameDir>/mods/`
///
/// 关掉隔离时不校验 `version_id`（这时它根本不参与路径），所以调用方即使
/// 还不知道版本目录名也能安全地拿共享目录。开着隔离时 `version_id` 必须是一个
/// 正常的单层目录名，`../` 这类会被 [`validate_version_id`] 拒绝。
pub fn mods_dir(
    game_dir: impl AsRef<Path>,
    version_id: &str,
    isolated: bool,
) -> Result<ModsTarget> {
    let game_dir = game_dir.as_ref();

    if !isolated {
        return Ok(ModsTarget {
            dir: game_dir.join(MODS),
            isolated: false,
            relative: MODS.to_string(),
        });
    }

    let id = validate_version_id(version_id)?;
    Ok(ModsTarget {
        dir: game_dir.join(VERSIONS).join(id).join(MODS),
        isolated: true,
        relative: format!("{VERSIONS}/{id}/{MODS}"),
    })
}

/// 校验版本目录名能不能安全地当一层目录用。
///
/// 只放行「单层、普通」的名字：Mojang 的 `1.21.1`、快照 `24w14a`、加载器拼出来的
/// `fabric-loader-0.16.9-1.21.1`、用户自己改的中文名都通过；分隔符、盘符、
/// 控制字符、`.` / `..` 一律拒绝。返回的是 trim 之后的名字。
pub fn validate_version_id(version_id: &str) -> Result<&str> {
    let id = version_id.trim();

    if id.is_empty() {
        return Err(Error::other(
            "不知道这次装出来的版本目录名，算不出隔离后的 mods 目录",
        ));
    }
    if id == "." || id == ".." {
        return Err(Error::other(format!(
            "版本目录名 `{id}` 不能当目录用（路径穿越）"
        )));
    }
    for ch in id.chars() {
        let bad = matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|');
        if bad || ch.is_control() {
            return Err(Error::other(format!(
                "版本目录名里不能有 `{}`（`{id}`），它会跑到 mods 目录之外",
                ch.escape_default()
            )));
        }
    }

    Ok(id)
}

/// 整合包索引里的相对路径 → 真正落盘的绝对路径。
///
/// 只把 `mods/` 下面的条目搬进隔离目录；`config/`、`resourcepacks/`、
/// `shaderpacks/` 等仍按 PCL 的约定留在游戏根目录（那些目录的隔离属于启动器
/// 自己的事，MCDL 只管模组）。关掉隔离时一切照旧落在 `gameDir` 下。
///
/// 这里再挡一次路径穿越：整合包索引是不可信输入，`mrpack::safe_path` 已经拦过
/// 一遍，落盘的地方值得再拦一遍。
pub fn place_pack_file(
    game_dir: impl AsRef<Path>,
    version_id: &str,
    isolated: bool,
    relative: &str,
) -> Result<PathBuf> {
    let game_dir = game_dir.as_ref();
    let parts = split_relative(relative)?;

    let first = parts[0];
    let under_mods = first.eq_ignore_ascii_case(MODS);
    if isolated && under_mods && parts.len() > 1 {
        let target = mods_dir(game_dir, version_id, true)?;
        let mut path = target.dir;
        for part in &parts[1..] {
            path.push(part);
        }
        return Ok(path);
    }

    let mut path = game_dir.to_path_buf();
    for part in &parts {
        path.push(part);
    }
    Ok(path)
}

/// 把 `a/b/c`（或 Windows 风格的 `a\b\c`）拆成干净的分量。
///
/// 拒绝：空串、绝对路径、盘符、`.` / `..`、控制字符。
fn split_relative(relative: &str) -> Result<Vec<&str>> {
    let text = relative.trim();
    if text.is_empty() {
        return Err(Error::other("整合包里的路径是空的"));
    }
    if text.starts_with('/') || text.starts_with('\\') {
        return Err(Error::other(format!(
            "整合包里的路径不能是绝对路径：{text}"
        )));
    }

    let mut parts = Vec::new();
    for raw in text.split(['/', '\\']) {
        let part = raw.trim();
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return Err(Error::other(format!(
                "整合包里的路径不能往上跳（..）：{text}"
            )));
        }
        if part.contains(':') || part.chars().any(char::is_control) {
            return Err(Error::other(format!("整合包里的路径含非法字符：{text}")));
        }
        parts.push(part);
    }

    if parts.is_empty() {
        return Err(Error::other(format!("整合包里的路径没有有效内容：{text}")));
    }
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game() -> PathBuf {
        PathBuf::from("C:/mc/.minecraft")
    }

    #[test]
    fn isolated_on_puts_mods_under_the_version_dir() {
        let target = mods_dir(game(), "fabric-loader-0.16.9-1.21.1", true).unwrap();
        assert!(target.isolated);
        assert_eq!(
            target.dir,
            game().join("versions/fabric-loader-0.16.9-1.21.1/mods")
        );
        assert_eq!(target.relative, "versions/fabric-loader-0.16.9-1.21.1/mods");
        assert_eq!(target.label(), "versions/fabric-loader-0.16.9-1.21.1/mods/");
        // 文件落在版本自己的 mods 里，不碰根目录
        assert_eq!(
            target.join("sodium.jar"),
            game().join("versions/fabric-loader-0.16.9-1.21.1/mods/sodium.jar")
        );
    }

    #[test]
    fn isolated_off_puts_mods_at_the_game_root() {
        let target = mods_dir(game(), "fabric-loader-0.16.9-1.21.1", false).unwrap();
        assert!(!target.isolated);
        assert_eq!(target.dir, game().join("mods"));
        assert_eq!(target.relative, "mods");
        assert_eq!(target.join("sodium.jar"), game().join("mods/sodium.jar"));
    }

    /// 关掉隔离时版本名不参与路径，所以传垃圾也不该报错——
    /// 调用方常常在这时还不知道版本目录名。
    #[test]
    fn isolated_off_ignores_the_version_id() {
        for id in ["", "  ", "../evil", "a/b"] {
            let target = mods_dir(game(), id, false).unwrap();
            assert_eq!(target.dir, game().join("mods"), "id = {id:?}");
        }
    }

    /// 真实版本名里有空格、点、中文、下划线，这些都必须放行。
    #[test]
    fn special_but_legal_version_ids_are_kept_verbatim() {
        for id in [
            "1.21.1",
            "24w14a",
            "1.21.1-forge-51.0.33",
            "我的整合包 1.20.1",
            "1.20.1 (优化版)",
            "fabric-loader-0.16.9-1.21.1",
        ] {
            let target = mods_dir(game(), id, true).unwrap();
            assert_eq!(
                target.dir,
                game().join("versions").join(id).join("mods"),
                "id = {id:?}"
            );
        }
    }

    /// 路径穿越必须被拒绝，而且拒绝时不能给出任何路径。
    #[test]
    fn traversal_and_separators_are_rejected() {
        for id in [
            "..",
            ".",
            "../evil",
            "..\\evil",
            "a/b",
            "a\\b",
            "C:evil",
            "/abs",
            "\\\\server\\share",
            "a\u{0}b",
            "a\nb",
            "",
            "   ",
            "mod/../../etc",
        ] {
            assert!(
                mods_dir(game(), id, true).is_err(),
                "这个版本名应该被拒绝：{id:?}"
            );
        }
    }

    /// 就算有人绕过校验硬拼，隔离目录也必须留在 gameDir 里面。
    #[test]
    fn isolated_mods_never_escape_the_game_dir() {
        let target = mods_dir(game(), "我的整合包 1.20.1", true).unwrap();
        assert!(target.dir.starts_with(game()));
        assert_eq!(
            target.dir.components().count(),
            game().components().count() + 3,
            "多出来的只能是 versions/<id>/mods 三层"
        );
    }

    #[test]
    fn place_pack_file_routes_mods_only() {
        let g = game();

        let mods = place_pack_file(&g, "1.21.1-fabric", true, "mods/sodium.jar").unwrap();
        assert_eq!(mods, g.join("versions/1.21.1-fabric/mods/sodium.jar"));

        // 子目录也要保持结构
        let nested = place_pack_file(&g, "1.21.1-fabric", true, "mods/sub/a.jar").unwrap();
        assert_eq!(nested, g.join("versions/1.21.1-fabric/mods/sub/a.jar"));

        // 非 mods 的条目仍按 PCL 的约定留在根目录
        let config = place_pack_file(&g, "1.21.1-fabric", true, "config/x.toml").unwrap();
        assert_eq!(config, g.join("config/x.toml"));

        let packs = place_pack_file(&g, "1.21.1-fabric", true, "resourcepacks/a.zip").unwrap();
        assert_eq!(packs, g.join("resourcepacks/a.zip"));

        // 关掉隔离：mods 也回根目录
        let shared = place_pack_file(&g, "1.21.1-fabric", false, "mods/sodium.jar").unwrap();
        assert_eq!(shared, g.join("mods/sodium.jar"));
    }

    #[test]
    fn place_pack_file_rejects_traversal() {
        let g = game();
        for rel in [
            "../escape.jar",
            "mods/../../escape.jar",
            "/abs/x.jar",
            "",
            "  ",
        ] {
            assert!(
                place_pack_file(&g, "1.21.1", true, rel).is_err(),
                "应该拒绝：{rel:?}"
            );
        }
        // 隔离开着但拿不到版本名：mods 条目必须报错，不能悄悄落回根目录
        assert!(place_pack_file(&g, "", true, "mods/a.jar").is_err());
    }

    /// 界面上常驻的那句话必须提到「谁能读到」，这是这个功能的全部意义。
    #[test]
    fn notice_names_the_launchers_that_support_isolation() {
        for needle in ["PCL", "HMCL", "官方启动器", "mods/"] {
            assert!(
                ISOLATION_NOTICE.contains(needle),
                "提示里缺 `{needle}`：{ISOLATION_NOTICE}"
            );
        }
    }
}
