//! 游戏目录布局。所有路径都从这里取，避免各模块各拼各的。

use std::path::{Path, PathBuf};

use crate::error::{IoContext, Result};

/// 一个 `.minecraft` 目录。
#[derive(Debug, Clone)]
pub struct GamePaths {
    root: PathBuf,
}

impl GamePaths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn versions(&self) -> PathBuf {
        self.root.join("versions")
    }

    pub fn version_dir(&self, id: &str) -> PathBuf {
        self.versions().join(id)
    }

    pub fn version_json(&self, id: &str) -> PathBuf {
        self.version_dir(id).join(format!("{id}.json"))
    }

    pub fn version_jar(&self, id: &str) -> PathBuf {
        self.version_dir(id).join(format!("{id}.jar"))
    }

    pub fn libraries(&self) -> PathBuf {
        self.root.join("libraries")
    }

    pub fn assets(&self) -> PathBuf {
        self.root.join("assets")
    }

    pub fn asset_indexes(&self) -> PathBuf {
        self.assets().join("indexes")
    }

    pub fn asset_objects(&self) -> PathBuf {
        self.assets().join("objects")
    }

    /// `objects/ab/abcdef…`
    pub fn asset_object(&self, hash: &str) -> PathBuf {
        let (head, _) = hash.split_at(2.min(hash.len()));
        self.asset_objects().join(head).join(hash)
    }

    /// Forge installer 会检查这个文件是否存在，不存在就直接退出。
    pub fn launcher_profiles(&self) -> PathBuf {
        self.root.join("launcher_profiles.json")
    }

    pub fn natives_dir(&self, version_id: &str) -> PathBuf {
        self.version_dir(version_id).join("natives")
    }

    /// 建好安装必须存在的几个目录，并补上 Forge 需要的 `launcher_profiles.json`。
    pub fn prepare(&self) -> Result<()> {
        for dir in [
            self.root.clone(),
            self.versions(),
            self.libraries(),
            self.assets(),
            self.asset_indexes(),
            self.asset_objects(),
        ] {
            std::fs::create_dir_all(&dir).at(&dir)?;
        }

        let profiles = self.launcher_profiles();
        if !profiles.exists() {
            std::fs::write(&profiles, br#"{"profiles":{}}"#).at(&profiles)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_object_uses_hash_prefix() {
        let p = GamePaths::new("/mc");
        let path = p.asset_object("abcdef1234567890");
        assert!(path.ends_with("objects/ab/abcdef1234567890"));
    }

    #[test]
    fn version_paths_follow_mojang_layout() {
        let p = GamePaths::new("/mc");
        assert!(p.version_json("1.21.1").ends_with("versions/1.21.1/1.21.1.json"));
        assert!(p.version_jar("1.21.1").ends_with("versions/1.21.1/1.21.1.jar"));
    }
}
