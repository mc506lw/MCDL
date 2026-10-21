//! 跨启动器扫描与导入。
//!
//! 目标：用户点一下「扫描」，就把本机其它启动器（PCL / PCL CE / HMCL / BakaXL /
//! Prism / MultiMC / 官方启动器）留下的 `.minecraft` 与实例目录找出来，列清楚，
//! 勾选之后导入到当前目标目录。
//!
//! 四条硬规则：
//!
//! 1. **不写死任何用户路径**：所有位置都从 [`ScanEnv`]（环境变量推导）出发；
//!    找不到就是空列表，不报错。
//! 2. **不动源目录**：导入只读源、只写目标。硬链也只加一个名字，不改内容。
//! 3. **扫描要快**：只读配置文件和目录名，不递归统计体积——体积用 [`measure`] 懒算。
//! 4. **不猜**：每个「找到」都挂一条证据（[`Probe`]），找不到的也留一条说明，
//!    界面据此回答「为什么没扫到 HMCL」。
//!
//! 这里解析的配置格式（PCL 的 ini、HMCL 的 json、Prism 的 mmc-pack.json）都是公开事实，
//! 没有搬运任何启动器的代码。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{Error, IoContext, Result};
use crate::isolate;
use crate::types::LogLevel;

// ---------------------------------------------------------------- 启动器

/// 认出来的启动器家族。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Launcher {
    Pcl,
    PclCe,
    Hmcl,
    BakaXl,
    Prism,
    MultiMc,
    /// 官方启动器
    Vanilla,
    /// 有 `.minecraft` 但认不出是谁留下的
    Unknown,
}

impl Launcher {
    pub fn label(self) -> &'static str {
        match self {
            Launcher::Pcl => "PCL",
            Launcher::PclCe => "PCL CE",
            Launcher::Hmcl => "HMCL",
            Launcher::BakaXl => "BakaXL",
            Launcher::Prism => "Prism",
            Launcher::MultiMc => "MultiMC",
            Launcher::Vanilla => "官方启动器",
            Launcher::Unknown => "未知来源",
        }
    }

    /// 从「配置目录存在」这种弱证据里能不能算找到。
    pub fn is_known(self) -> bool {
        !matches!(self, Launcher::Unknown)
    }
}

/// 一个可导入单元是哪一类目录。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    /// 一个 `.minecraft`：版本在 `versions/<id>/`
    GameDir,
    /// Prism 实例：`instance.cfg` + `mmc-pack.json` + `minecraft/`
    PrismInstance,
    /// MultiMC 实例，结构同上
    MultiMcInstance,
}

impl ItemKind {
    pub fn label(self) -> &'static str {
        match self {
            ItemKind::GameDir => "游戏目录",
            ItemKind::PrismInstance => "Prism 实例",
            ItemKind::MultiMcInstance => "MultiMC 实例",
        }
    }

    /// 实例目录本身没有 `versions/<id>/`，版本要由 MCDL 重新装。
    pub fn is_instance(self) -> bool {
        !matches!(self, ItemKind::GameDir)
    }
}

// ---------------------------------------------------------------- 扫描结果

/// 一个版本目录（`versions/<id>/`），只读目录名和版本 JSON，不碰体积。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportVersion {
    /// `versions/` 下的目录名，也是导入时用的标识
    pub id: String,
    /// release / snapshot / old_beta / old_alpha；实例的合成条目是 `instance`
    pub kind: String,
    /// 认出来的加载器，原版是 None
    pub loader: Option<String>,
    /// 能不能按目录复制过去。实例的合成条目是 false（它没有目录）
    pub importable: bool,
}

/// 扫到的一项。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportItem {
    /// 稳定标识：规范化后的路径，界面上用来做勾选的 key
    pub key: String,
    /// 识别到的来源，第一个是主来源（可能同时被官方启动器和 PCL 共用）
    pub launchers: Vec<Launcher>,
    pub kind: ItemKind,
    /// 显示名：Prism 实例取 `instance.cfg` 里的 `name=`，游戏目录取目录名
    pub name: String,
    /// 要导入的根。实例是实例目录本身
    pub path: PathBuf,
    /// 真正的 `.minecraft`（实例是 `path/minecraft`）
    pub game_dir: PathBuf,
    pub versions: Vec<ImportVersion>,
    pub has_mods: bool,
    pub has_config: bool,
    pub has_saves: bool,
    /// 目录最后修改时间（Unix 秒），用来排序
    pub modified: u64,
    /// 找到它的证据，界面直接显示
    pub evidence: Vec<String>,
}

impl ImportItem {
    pub fn main_launcher(&self) -> Launcher {
        self.launchers.first().copied().unwrap_or(Launcher::Unknown)
    }

    /// 「PCL + 官方启动器」
    pub fn launcher_label(&self) -> String {
        if self.launchers.is_empty() {
            return Launcher::Unknown.label().to_string();
        }
        self.launchers
            .iter()
            .map(|l| l.label())
            .collect::<Vec<_>>()
            .join(" + ")
    }

    /// 去重后的加载器列表：`Fabric`、`Forge`，原版就是空。
    pub fn loaders(&self) -> Vec<String> {
        let mut seen = Vec::new();
        for v in &self.versions {
            if let Some(l) = &v.loader {
                if !seen.iter().any(|s: &String| s == l) {
                    seen.push(l.clone());
                }
            }
        }
        seen
    }

    /// 能按目录复制过去的版本名。
    pub fn importable_versions(&self) -> Vec<String> {
        self.versions
            .iter()
            .filter(|v| v.importable)
            .map(|v| v.id.clone())
            .collect()
    }
}

/// 一次探针：扫了哪里、找到没有、为什么。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Probe {
    pub launcher: Launcher,
    pub path: PathBuf,
    pub found: bool,
    /// 中文说明，界面原样显示
    pub note: String,
}

/// 一次扫描的结果。扫描本身不会失败：找不到就是空列表。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScanResult {
    pub items: Vec<ImportItem>,
    pub probes: Vec<Probe>,
    /// 实际 stat 过的候选路径数
    pub scanned: usize,
    pub duration_ms: u64,
}

impl ScanResult {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 找到了几个版本（所有 item 加起来）。
    pub fn version_count(&self) -> usize {
        self.items.iter().map(|i| i.versions.len()).sum()
    }
}

/// 目录体积。懒加载用，扫描阶段不算。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirSize {
    pub bytes: u64,
    pub files: u64,
}

impl DirSize {
    fn add(&mut self, other: DirSize) {
        self.bytes += other.bytes;
        self.files += other.files;
    }
}

// ---------------------------------------------------------------- 环境

/// 推导路径用的环境。全部字段都由环境变量填充，测试里可以自己造。
#[derive(Debug, Clone, Default)]
pub struct ScanEnv {
    /// Windows `%APPDATA%`；macOS `~/Library/Application Support`；Linux `$XDG_CONFIG_HOME`
    pub app_data: Option<PathBuf>,
    /// 放实例的那一层数据目录：Windows 同 `%APPDATA%`；Linux `$XDG_DATA_HOME`
    pub data_home: Option<PathBuf>,
    /// Windows `%LOCALAPPDATA%`，其它平台没有
    pub local_data: Option<PathBuf>,
    /// 用户主目录
    pub home: Option<PathBuf>,
}

impl ScanEnv {
    /// 从本进程的环境变量推导。读不到就是 None，绝不猜。
    pub fn from_process() -> Self {
        let var = |key: &str| {
            std::env::var_os(key)
                .map(PathBuf::from)
                .filter(|p| !p.as_os_str().is_empty())
        };
        let home = var("USERPROFILE").or_else(|| var("HOME"));
        let mac_app_data = || home.as_ref().map(|h| h.join("Library/Application Support"));
        let app_data = if cfg!(windows) {
            var("APPDATA")
        } else if cfg!(target_os = "macos") {
            mac_app_data()
        } else {
            var("XDG_CONFIG_HOME").or_else(|| home.as_ref().map(|h| h.join(".config")))
        };
        let data_home = if cfg!(windows) {
            var("APPDATA")
        } else if cfg!(target_os = "macos") {
            mac_app_data()
        } else {
            var("XDG_DATA_HOME").or_else(|| home.as_ref().map(|h| h.join(".local/share")))
        };
        Self {
            app_data,
            data_home,
            local_data: if cfg!(windows) {
                var("LOCALAPPDATA")
            } else {
                None
            },
            home,
        }
    }
}

// ---------------------------------------------------------------- 扫描

/// 用本机环境扫一遍。
pub fn scan_default() -> ScanResult {
    scan(&ScanEnv::from_process())
}

/// 扫一遍。永远不会返回 Err：读不了配置就当没有，路径不存在就当没有。
pub fn scan(env: &ScanEnv) -> ScanResult {
    let started = Instant::now();
    let mut probes: Vec<Probe> = Vec::new();
    // 路径 → 证据（来自各启动器的配置文件）
    let mut from_config: Vec<(PathBuf, Launcher, String)> = Vec::new();
    let mut scanned = 0usize;

    // ---------- 1. PCL / PCL CE ----------
    // PCL 的配置目录在 %APPDATA%\PCL；它自己的目录列表在游戏目录的 PCL\Setup.ini。
    for dir in roots_of(env, "PCL") {
        if dir.is_dir() {
            scanned += 1;
            probes.push(Probe {
                launcher: Launcher::Pcl,
                path: dir.clone(),
                found: true,
                note: "找到 PCL 的配置目录".to_string(),
            });
            // 顺手从 config.json 里捞路径（有的版本把目录记在这里）
            let cfg = dir.join("config.json");
            if let Some(text) = read_small_text(&cfg) {
                for p in harvest_paths(&text) {
                    from_config.push((
                        p,
                        Launcher::Pcl,
                        "PCL 的 config.json 里记着这个目录".into(),
                    ));
                }
            }
        } else {
            probes.push(Probe {
                launcher: Launcher::Pcl,
                path: dir,
                found: false,
                note: "没有这个目录".to_string(),
            });
        }
    }

    // 每个游戏目录里的 PCL\Setup.ini 都要读——PCL 记住的其它目录写在那里。
    for game_dir in default_game_dirs(env) {
        let setup = game_dir.join("PCL").join("Setup.ini");
        let Some(text) = read_small_text(&setup) else {
            continue;
        };
        scanned += 1;
        let ini = Ini::parse(&text);
        let ce = text.to_ascii_uppercase().contains("PCLCE");
        let launcher = if ce { Launcher::PclCe } else { Launcher::Pcl };
        let found = ini.path_values().len();
        probes.push(Probe {
            launcher,
            path: setup.clone(),
            found: true,
            note: if found == 0 {
                "读到 PCL 的 Setup.ini，但里面没有记住的游戏目录".to_string()
            } else {
                format!("读到 PCL 的 Setup.ini，里面记着 {found} 个目录")
            },
        });
        for raw in ini.path_values() {
            for p in split_candidates(&raw) {
                from_config.push((p, launcher, "PCL 的 Setup.ini 里记着这个目录".to_string()));
            }
        }
    }

    // ---------- 2. HMCL ----------
    // hmcl.json 在 %APPDATA%\.hmcl\ 或游戏目录里；里面 config.gameDir 记着当前目录，
    // 多目录版本还有别的字段名，所以这里不认键名，认「能指到游戏目录的字符串」。
    for dir in roots_of(env, ".hmcl") {
        let json = dir.join("hmcl.json");
        match read_small_text(&json) {
            Some(text) => {
                scanned += 1;
                let found = harvest_paths(&text);
                probes.push(Probe {
                    launcher: Launcher::Hmcl,
                    path: json.clone(),
                    found: true,
                    note: if found.is_empty() {
                        "读到 HMCL 的 hmcl.json，但里面没有可用的游戏目录".to_string()
                    } else {
                        format!("读到 HMCL 的 hmcl.json，里面记着 {} 个目录", found.len())
                    },
                });
                for p in found {
                    from_config.push((
                        p,
                        Launcher::Hmcl,
                        "HMCL 的 hmcl.json 里记着这个目录".into(),
                    ));
                }
            }
            None => probes.push(Probe {
                launcher: Launcher::Hmcl,
                path: json,
                found: false,
                note: "没有这个文件".to_string(),
            }),
        }
    }

    // ---------- 3. BakaXL ----------
    // BakaXL 的配置目录在 %APPDATA%\BakaXL，具体文件名各版本不同，
    // 所以只读目录里的文本文件、捞里面像路径的字符串。
    for dir in roots_of(env, "BakaXL") {
        if !dir.is_dir() {
            probes.push(Probe {
                launcher: Launcher::BakaXl,
                path: dir,
                found: false,
                note: "没有这个目录".to_string(),
            });
            continue;
        }
        scanned += 1;
        let mut found: Vec<PathBuf> = Vec::new();
        for file in text_files_in(&dir, 8) {
            if let Some(text) = read_small_text(&file) {
                found.extend(harvest_paths(&text));
            }
        }
        found.sort();
        found.dedup();
        probes.push(Probe {
            launcher: Launcher::BakaXl,
            path: dir,
            found: true,
            note: if found.is_empty() {
                "找到 BakaXL 的配置目录，但里面没有可读的游戏目录".to_string()
            } else {
                format!("找到 BakaXL 的配置目录，里面记着 {} 个目录", found.len())
            },
        });
        for p in found {
            from_config.push((p, Launcher::BakaXl, "BakaXL 的配置里记着这个目录".into()));
        }
    }

    // ---------- 4. 游戏目录 ----------
    let mut items: Vec<ImportItem> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut candidates: Vec<PathBuf> = default_game_dirs(env);
    candidates.extend(from_config.iter().map(|(p, _, _)| p.clone()));

    for dir in candidates {
        let key = norm_key(&dir);
        if !seen.insert(key.clone()) {
            continue;
        }
        scanned += 1;
        if !dir.is_dir() {
            continue;
        }

        let mut launchers: Vec<Launcher> = Vec::new();
        let mut evidence: Vec<String> = Vec::new();
        for (path, launcher, note) in &from_config {
            if norm_key(path) == key {
                push_unique(&mut launchers, *launcher);
                evidence.push(note.clone());
            }
        }
        let markers = markers_in(&dir, env);
        for (launcher, note) in markers {
            push_unique(&mut launchers, launcher);
            evidence.push(note);
        }

        let versions = list_versions(&dir);
        let has_mods = dir_has_entries(&dir.join("mods"));
        let has_config = dir_has_entries(&dir.join("config"));
        let has_saves = dir_has_entries(&dir.join("saves"));

        // 只有「认出了启动器」或「里面确实有东西」才列出来，空目录不占位置。
        if launchers.is_empty() && versions.is_empty() && !has_mods && !has_config {
            probes.push(Probe {
                launcher: Launcher::Unknown,
                path: dir.clone(),
                found: true,
                note: "目录存在，但既没有启动器标记也没有版本".to_string(),
            });
            continue;
        }

        if launchers.is_empty() {
            launchers.push(Launcher::Unknown);
        }
        let name = dir
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| dir.to_string_lossy().into_owned());
        items.push(ImportItem {
            key,
            launchers,
            kind: ItemKind::GameDir,
            name,
            path: dir.clone(),
            game_dir: dir.clone(),
            versions,
            has_mods,
            has_config,
            has_saves,
            modified: modified_secs(&dir),
            evidence,
        });
    }

    // ---------- 5. Prism / MultiMC 实例 ----------
    for (root, launcher, kind) in instance_roots(env) {
        if !root.is_dir() {
            probes.push(Probe {
                launcher,
                path: root,
                found: false,
                note: "没有这个目录".to_string(),
            });
            continue;
        }
        scanned += 1;
        let mut count = 0usize;
        for item in scan_instances(&root, launcher, kind) {
            let key = norm_key(&item.path);
            if !seen.insert(key) {
                continue;
            }
            count += 1;
            items.push(item);
        }
        probes.push(Probe {
            launcher,
            path: root,
            found: true,
            note: format!("找到 {count} 个实例"),
        });
    }

    items.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.path.cmp(&b.path)));
    ScanResult {
        items,
        probes,
        scanned,
        duration_ms: started.elapsed().as_millis() as u64,
    }
}

/// 每个平台可能的游戏目录。不做平台判断，全都 stat 一次——多两次 stat 比多一段
/// `cfg!` 分支便宜，也让 Linux / macOS 的布局能在任何平台上被测到。
fn default_game_dirs(env: &ScanEnv) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(app) = &env.app_data {
        out.push(app.join(".minecraft"));
    }
    if let Some(data) = &env.data_home {
        out.push(data.join(".minecraft"));
    }
    if let Some(home) = &env.home {
        out.push(home.join(".minecraft"));
        // macOS 官方启动器（小写、没有点）
        out.push(home.join("Library/Application Support/minecraft"));
    }
    if let Some(local) = &env.local_data {
        out.push(local.join(".minecraft"));
    }
    out
}

/// 某个启动器的配置目录可能在哪。名字不带点 / 带点两种写法都试。
fn roots_of(env: &ScanEnv, name: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for base in [&env.app_data, &env.data_home, &env.local_data, &env.home]
        .into_iter()
        .flatten()
    {
        let candidate = base.join(name);
        if !out.iter().any(|p| norm_key(p) == norm_key(&candidate)) {
            out.push(candidate);
        }
    }
    out
}

/// Prism / MultiMC 的 `instances` 目录。
fn instance_roots(env: &ScanEnv) -> Vec<(PathBuf, Launcher, ItemKind)> {
    let mut out: Vec<(PathBuf, Launcher, ItemKind)> = Vec::new();
    let mut push = |path: PathBuf, launcher: Launcher, kind: ItemKind| {
        if !out.iter().any(|(p, _, _)| norm_key(p) == norm_key(&path)) {
            out.push((path, launcher, kind));
        }
    };
    for (name, launcher, kind) in [
        ("PrismLauncher", Launcher::Prism, ItemKind::PrismInstance),
        ("MultiMC", Launcher::MultiMc, ItemKind::MultiMcInstance),
    ] {
        for base in [&env.app_data, &env.data_home].into_iter().flatten() {
            push(base.join(name).join("instances"), launcher, kind);
        }
        if let Some(home) = &env.home {
            push(
                home.join(".local/share").join(name).join("instances"),
                launcher,
                kind,
            );
        }
    }
    // 老 MultiMC 在 Linux 上放在 ~/.multimc
    if let Some(home) = &env.home {
        push(
            home.join(".multimc").join("instances"),
            Launcher::MultiMc,
            ItemKind::MultiMcInstance,
        );
    }
    out
}

/// 一个游戏目录里的「启动器标记」。
fn markers_in(dir: &Path, env: &ScanEnv) -> Vec<(Launcher, String)> {
    let mut out = Vec::new();

    let pcl_dir = dir.join("PCL");
    let setup = pcl_dir.join("Setup.ini");
    if setup.is_file() {
        let ce = read_small_text(&setup)
            .map(|t| t.to_ascii_uppercase().contains("PCLCE"))
            .unwrap_or(false);
        out.push((
            if ce { Launcher::PclCe } else { Launcher::Pcl },
            "游戏目录里有 PCL\\Setup.ini".to_string(),
        ));
    } else if pcl_dir.is_dir() {
        out.push((Launcher::Pcl, "游戏目录里有 PCL 文件夹".to_string()));
    }

    if dir.join("hmcl.json").is_file() {
        out.push((Launcher::Hmcl, "游戏目录里有 hmcl.json".to_string()));
    }

    // 官方启动器：launcher_profiles.json 里有非空的 profiles。
    // 空 profiles 不算——MCDL 自己装 Forge 时就会写一个 `{"profiles":{}}`。
    if let Some(text) = read_small_text(&dir.join("launcher_profiles.json")) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
            let profiles = json.get("profiles").and_then(|p| p.as_object());
            if profiles.is_some_and(|p| !p.is_empty()) {
                out.push((
                    Launcher::Vanilla,
                    "launcher_profiles.json 里有官方启动器的档案".to_string(),
                ));
            }
        }
    }

    // PCL 的配置目录存在，那它的默认游戏目录就是 %APPDATA%\.minecraft。
    if let Some(app) = &env.app_data {
        if norm_key(&app.join(".minecraft")) == norm_key(dir) && app.join("PCL").is_dir() {
            out.push((
                Launcher::Pcl,
                "PCL 的配置目录存在，它的默认游戏目录就是这里".to_string(),
            ));
        }
    }

    out
}

/// 扫一个 instances 目录。
fn scan_instances(root: &Path, launcher: Launcher, kind: ItemKind) -> Vec<ImportItem> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let instance_cfg = path.join("instance.cfg");
        let pack = path.join("mmc-pack.json");
        if !instance_cfg.is_file() && !pack.is_file() {
            continue;
        }

        let game_dir = path.join("minecraft");
        let name = read_small_text(&instance_cfg)
            .and_then(|text| Ini::parse(&text).get_any("name"))
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| {
                path.file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });

        let mut versions = list_versions(&game_dir);
        let mut evidence = vec![format!("{} 的 instances 目录里的实例", launcher.label())];
        if let Some((mc, loader)) = read_mmc_pack(&pack) {
            evidence.push(format!(
                "mmc-pack.json 声明：{mc}{}",
                match &loader {
                    Some(l) => format!(" / {l}"),
                    None => String::new(),
                }
            ));
            if versions.is_empty() {
                // 实例没有独立的 versions 目录：这一条只用来显示「它是什么版本」，
                // 导入时不会被复制（importable = false）。id 只放 MC 版本号，
                // 加载器在 loader 字段里，界面并排显示，不重复。
                versions.push(ImportVersion {
                    id: mc.clone(),
                    kind: "instance".to_string(),
                    loader: loader.clone(),
                    importable: false,
                });
            }
        }

        out.push(ImportItem {
            key: norm_key(&path),
            launchers: vec![launcher],
            kind,
            name,
            path: path.clone(),
            game_dir: game_dir.clone(),
            versions,
            has_mods: dir_has_entries(&game_dir.join("mods")),
            has_config: dir_has_entries(&game_dir.join("config")),
            has_saves: dir_has_entries(&game_dir.join("saves")),
            modified: modified_secs(&path),
            evidence,
        });
    }
    out
}

/// 读 Prism / MultiMC 的 `mmc-pack.json`：`components` 里挑出 MC 版本和加载器。
fn read_mmc_pack(path: &Path) -> Option<(String, Option<String>)> {
    let text = read_small_text(path)?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let components = json.get("components")?.as_array()?;
    let mut mc = None;
    let mut loader = None;
    for c in components {
        let uid = c.get("uid").and_then(|v| v.as_str()).unwrap_or_default();
        let version = c
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        match uid {
            "net.minecraft" => mc = Some(version),
            "net.fabricmc.fabric-loader" => loader = Some(format!("Fabric {version}")),
            "org.quiltmc.quilt-loader" => loader = Some(format!("Quilt {version}")),
            "net.minecraftforge" => loader = Some(format!("Forge {version}")),
            "net.neoforged" => loader = Some(format!("NeoForge {version}")),
            _ => {}
        }
    }
    mc.map(|mc| (mc, loader))
}

/// 列 `versions/`。只读目录名和版本 JSON，不统计体积。
fn list_versions(game_dir: &Path) -> Vec<ImportVersion> {
    let versions_dir = game_dir.join("versions");
    let Ok(entries) = std::fs::read_dir(&versions_dir) else {
        return Vec::new();
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
        // 没有 <id>.json 的目录是装了一半的残留，列出来只会让人以为能导入。
        let Some(json) = read_json(&dir.join(format!("{id}.json"))) else {
            continue;
        };
        out.push(ImportVersion {
            kind: json
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            loader: detect_loader(&id, &json),
            importable: true,
            id,
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// 从目录名和 `mainClass` 认加载器。
///
/// 和 `installed.rs` 里的判定保持一致：目录名优先（各 installer 的命名稳定），
/// `mainClass` 兜底（它会随加载器大版本变化）。
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
    let main = json
        .get("mainClass")
        .and_then(|v| v.as_str())
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

// ---------------------------------------------------------------- 配置解析

/// 一份 ini。PCL 的 `Setup.ini`、Prism 的 `instance.cfg` 都是这个形状。
#[derive(Debug, Clone, Default)]
pub struct Ini {
    entries: Vec<IniEntry>,
}

#[derive(Debug, Clone)]
struct IniEntry {
    section: String,
    key: String,
    value: String,
}

impl Ini {
    pub fn parse(text: &str) -> Self {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut entries = Vec::new();
        let mut section = String::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix('[') {
                if let Some(name) = rest.strip_suffix(']') {
                    section = name.trim().to_string();
                    continue;
                }
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            entries.push(IniEntry {
                section: section.clone(),
                key: key.trim().to_string(),
                value: unquote(value.trim()),
            });
        }
        Self { entries }
    }

    /// 某一节里的键。PCL 的 ini 有 `[Setup]` 这种节，键名在节内唯一。
    pub fn get_in(&self, section: &str, key: &str) -> Option<String> {
        self.entries
            .iter()
            .find(|e| e.section.eq_ignore_ascii_case(section) && e.key.eq_ignore_ascii_case(key))
            .map(|e| e.value.clone())
    }

    /// 某个键的第一个值（不分节）。
    pub fn get_any(&self, key: &str) -> Option<String> {
        self.entries
            .iter()
            .find(|e| e.key.eq_ignore_ascii_case(key))
            .map(|e| e.value.clone())
    }

    /// 所有看起来是路径的值：键名里带 folder / dir / path，或者值本身就是路径。
    ///
    /// 不认死 `LaunchFolderSelect` 这种键名——PCL 换过名字，认「像路径」比认「键名」稳。
    pub fn path_values(&self) -> Vec<String> {
        let mut out = Vec::new();
        for e in &self.entries {
            let key = e.key.to_ascii_lowercase();
            let keyed = key.contains("folder") || key.contains("dir") || key.contains("path");
            if keyed || looks_like_absolute_path(&e.value) {
                out.push(e.value.clone());
            }
        }
        out
    }
}

fn unquote(raw: &str) -> String {
    let bytes = raw.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        return raw[1..raw.len() - 1].to_string();
    }
    raw.to_string()
}

/// 一个值里可能塞了多个路径（`C:\a\.minecraft|PCL`），拆开。
fn split_candidates(raw: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for part in raw.split(['|', '\n', '\r']) {
        let part = part.trim().trim_matches('"').trim_matches('\'');
        if part.is_empty() || !looks_like_absolute_path(part) {
            continue;
        }
        let p = PathBuf::from(part);
        if p.is_dir() {
            out.push(p);
        }
    }
    out
}

/// 从一段文本（json / ini / 自定义格式）里捞出所有「指到一个游戏目录」的字符串。
///
/// 各启动器的字段名换过好几轮，所以不认字段名：认「字符串 + 它确实是个游戏目录」。
pub fn harvest_paths(text: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for token in tokenize(text) {
        if !looks_like_absolute_path(&token) {
            continue;
        }
        let path = PathBuf::from(&token);
        if !looks_like_game_dir(&path) {
            continue;
        }
        let key = norm_key(&path);
        if seen.insert(key) {
            out.push(path);
        }
    }
    out
}

/// 把文本切成候选 token：引号、等号、逗号、分号、空白、括号都当分隔符。
fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '"' | '\'' | '=' | ',' | ';' | '{' | '}' | '[' | ']' | '(' | ')' | '<' | '>' | '*'
            )
    })
    .map(|s| s.trim().to_string())
    .filter(|s| s.len() > 2)
    .collect()
}

/// Windows 盘符路径、UNC 路径，或 Unix 绝对路径。
fn looks_like_absolute_path(s: &str) -> bool {
    if s.len() < 3 {
        return false;
    }
    let bytes = s.as_bytes();
    if bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/') && bytes[0].is_ascii_alphabetic()
    {
        return true;
    }
    if s.starts_with("\\\\") || s.starts_with("//") {
        return true;
    }
    s.starts_with('/') && !s.starts_with("//")
}

/// 「这看起来像一个 `.minecraft`」：目录存在，并且有游戏目录该有的东西。
fn looks_like_game_dir(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    name == ".minecraft"
        || path.join("versions").is_dir()
        || path.join("launcher_profiles.json").is_file()
        || path.join("minecraft").join("versions").is_dir()
}

// ---------------------------------------------------------------- 懒加载体积

/// 一个目录的递归体积。跳过符号链接，避免环。
pub fn measure(dir: &Path) -> DirSize {
    let mut size = DirSize::default();
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
                size.bytes += meta.len();
                size.files += 1;
            }
        }
    }
    size
}

/// 一项「导入会搬多少」：各版本目录 + mods/config/saves。
///
/// 故意不算 libraries / assets：它们是共享的，导入也不搬，算进来只会吓人。
pub fn measure_item(item: &ImportItem) -> DirSize {
    let mut size = DirSize::default();
    for id in item.importable_versions() {
        size.add(measure(&item.game_dir.join("versions").join(id)));
    }
    for sub in ["mods", "config", "saves"] {
        size.add(measure(&item.game_dir.join(sub)));
    }
    size
}

// ---------------------------------------------------------------- 导入

/// 一次导入请求。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportRequest {
    /// 源：游戏目录，或 Prism / MultiMC 的实例目录
    pub source: PathBuf,
    /// 目标 `.minecraft`
    pub target: PathBuf,
    /// 要复制的 `versions/<id>`。实例没有目录，这里留空
    #[serde(default)]
    pub version_ids: Vec<String>,
    #[serde(default)]
    pub include_mods: bool,
    #[serde(default)]
    pub include_config: bool,
    #[serde(default)]
    pub include_saves: bool,
    /// 目标同名时覆盖。默认 false：已存在的就跳过
    #[serde(default)]
    pub overwrite: bool,
    /// 同盘硬链代替复制。默认 false：复制更安全，改目标不会影响源
    #[serde(default)]
    pub hardlink: bool,
    /// 版本隔离：导入的 mods 放进 `versions/<版本id>/mods/`（开，默认）还是根 `mods/`。
    ///
    /// 没有可导入的版本时退回共享目录，并在日志里说清楚原因。
    #[serde(default = "crate::isolate::default_on")]
    pub isolated: bool,
}

impl ImportRequest {
    /// 版本 + mods + config，不覆盖。
    pub fn new(source: impl Into<PathBuf>, target: impl Into<PathBuf>) -> Self {
        Self {
            source: source.into(),
            target: target.into(),
            version_ids: Vec::new(),
            include_mods: true,
            include_config: true,
            include_saves: false,
            overwrite: false,
            hardlink: false,
            isolated: true,
        }
    }
}

/// 导入过程中向外抛的事件。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImportProgress {
    /// 开始搬一类东西（一个版本目录 / mods / config）
    Stage {
        index: usize,
        total: usize,
        label: String,
    },
    /// 已处理的文件数。`done` 包含跳过的文件，所以它会走到 total
    Files {
        done: usize,
        total: usize,
        bytes_done: u64,
        bytes_total: u64,
        current: String,
    },
    Log {
        level: LogLevel,
        message: String,
    },
}

/// 进度接收端。GUI 用 channel，CLI 用 stdout。
pub trait ImportSink: Send + Sync {
    fn emit(&self, progress: ImportProgress);
}

impl<F> ImportSink for F
where
    F: Fn(ImportProgress) + Send + Sync,
{
    fn emit(&self, progress: ImportProgress) {
        self(progress);
    }
}

/// 什么都不做的接收端，给测试用。
pub struct NullSink;

impl ImportSink for NullSink {
    fn emit(&self, _progress: ImportProgress) {}
}

/// 取消令牌。导入是同步 IO，靠一个原子标志在文件之间检查，比 abort future 干净。
pub type CancelToken = Arc<AtomicBool>;

pub fn cancel_token() -> CancelToken {
    Arc::new(AtomicBool::new(false))
}

fn check_cancel(cancel: &CancelToken) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

/// 导入结果。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportReport {
    pub target: PathBuf,
    /// 真搬过去的版本
    pub versions: Vec<String>,
    /// 目标里已经有、跳过的版本
    pub skipped_versions: Vec<String>,
    pub files: usize,
    pub bytes: u64,
    /// 目标里已经有、跳过的文件
    pub skipped_files: usize,
    /// 走硬链的文件数
    pub links: usize,
    /// 复制失败的文件数（不影响其它文件）
    pub failed: usize,
    pub duration_ms: u64,
}

impl ImportReport {
    /// 什么都没搬、也没跳过任何东西。
    pub fn is_empty(&self) -> bool {
        self.files == 0 && self.skipped_files == 0 && self.versions.is_empty()
    }
}

/// 一类要搬的东西。
struct PlanStage {
    label: String,
    src: PathBuf,
    dst: PathBuf,
    version_id: Option<String>,
}

/// 执行一次导入。源目录只读。
pub fn import(
    req: &ImportRequest,
    sink: &dyn ImportSink,
    cancel: &CancelToken,
) -> Result<ImportReport> {
    let started = Instant::now();
    let source = resolve_game_dir(&req.source);
    let target = req.target.clone();

    // 目标不能是源自己，也不能互相包含——那种「导入」只会把文件搬进自己。
    let src_key = norm_key(&source);
    let dst_key = norm_key(&target);
    if src_key == dst_key {
        return Err(Error::other("源目录和目标目录是同一个，不需要导入"));
    }
    if dst_key.starts_with(&format!("{src_key}/")) || src_key.starts_with(&format!("{dst_key}/")) {
        return Err(Error::other("源目录和目标目录互相包含，无法导入"));
    }

    // ---------- 规划 ----------
    let mut stages: Vec<PlanStage> = Vec::new();
    let mut skipped_versions: Vec<String> = Vec::new();
    // 这次真的会搬过去的版本 id。mods 的隔离落点要用它。
    let mut planned_versions: Vec<String> = Vec::new();

    for raw in &req.version_ids {
        let id = sanitize_version_id(raw)?;
        let src = source.join("versions").join(&id);
        if !src.is_dir() {
            sink.emit(ImportProgress::Log {
                level: LogLevel::Warn,
                message: format!("源目录里没有版本 {id}，跳过"),
            });
            continue;
        }
        let dst = target.join("versions").join(&id);
        if dst.exists() && !req.overwrite {
            skipped_versions.push(id.clone());
            sink.emit(ImportProgress::Log {
                level: LogLevel::Warn,
                message: format!("目标里已经有版本 {id}，跳过（不覆盖）"),
            });
            continue;
        }
        planned_versions.push(id.clone());
        stages.push(PlanStage {
            label: format!("版本 {id}"),
            src,
            dst,
            version_id: Some(id),
        });
    }

    // mods 是实例级的东西：隔离开着时，源 mods 要跟着导入的版本进各自的
    // `versions/<id>/mods/`。落点只走 isolate::mods_dir。
    if req.include_mods {
        let src = source.join("mods");
        if src.is_dir() {
            if req.isolated && !planned_versions.is_empty() {
                for id in &planned_versions {
                    match isolate::mods_dir(&target, id, true) {
                        Ok(mods) => stages.push(PlanStage {
                            label: format!("mods → {}", mods.label()),
                            src: src.clone(),
                            dst: mods.dir,
                            version_id: None,
                        }),
                        Err(e) => sink.emit(ImportProgress::Log {
                            level: LogLevel::Warn,
                            message: format!("版本 {id} 的模组落点算不出来，跳过它的 mods：{e}"),
                        }),
                    }
                }
            } else {
                if req.isolated {
                    sink.emit(ImportProgress::Log {
                        level: LogLevel::Warn,
                        message: "版本隔离开着，但这次没有可导入的版本，模组只能放进共享的 mods/\
                                  （官方启动器读得到，PCL / HMCL 的隔离实例读不到）"
                            .to_string(),
                    });
                }
                let shared = isolate::mods_dir(&target, "", false)?.dir;
                stages.push(PlanStage {
                    label: "mods".to_string(),
                    src,
                    dst: shared,
                    version_id: None,
                });
            }
        }
    }

    for (sub, include) in [("config", req.include_config), ("saves", req.include_saves)] {
        if !include {
            continue;
        }
        let src = source.join(sub);
        if !src.is_dir() {
            continue;
        }
        stages.push(PlanStage {
            label: sub.to_string(),
            src,
            dst: target.join(sub),
            version_id: None,
        });
    }

    if stages.is_empty() {
        sink.emit(ImportProgress::Log {
            level: LogLevel::Info,
            message: "没有需要导入的内容".to_string(),
        });
        return Ok(ImportReport {
            target,
            skipped_versions,
            duration_ms: started.elapsed().as_millis() as u64,
            ..Default::default()
        });
    }

    // ---------- 数一遍（进度条要知道总量） ----------
    let mut counters = Counters::new();
    for stage in &stages {
        check_cancel(cancel)?;
        let size = measure(&stage.src);
        counters.planned_files += size.files as usize;
        counters.planned_bytes += size.bytes;
    }
    sink.emit(ImportProgress::Log {
        level: LogLevel::Info,
        message: format!(
            "计划：{} 项，{} 个文件，{} MB",
            stages.len(),
            counters.planned_files,
            counters.planned_bytes / 1_048_576
        ),
    });

    // ---------- 搬 ----------
    let total = stages.len();
    let mut versions: Vec<String> = Vec::new();
    for (i, stage) in stages.iter().enumerate() {
        check_cancel(cancel)?;
        sink.emit(ImportProgress::Stage {
            index: i + 1,
            total,
            label: stage.label.clone(),
        });
        copy_tree(
            &stage.src,
            &stage.dst,
            req.overwrite,
            req.hardlink,
            &mut counters,
            sink,
            cancel,
        )?;
        if let Some(id) = &stage.version_id {
            versions.push(id.clone());
        }
    }

    // 收尾再报一次，保证进度条一定走到 100%
    sink.emit(ImportProgress::Files {
        done: counters.processed.max(counters.planned_files),
        total: counters.planned_files,
        bytes_done: counters.bytes_done,
        bytes_total: counters.planned_bytes,
        current: String::new(),
    });

    Ok(ImportReport {
        target,
        versions,
        skipped_versions,
        files: counters.copied,
        bytes: counters.copied_bytes,
        skipped_files: counters.skipped,
        links: counters.links,
        failed: counters.failed,
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

#[derive(Default)]
struct Counters {
    planned_files: usize,
    planned_bytes: u64,
    processed: usize,
    copied: usize,
    copied_bytes: u64,
    bytes_done: u64,
    skipped: usize,
    links: usize,
    failed: usize,
    last_emit: Option<Instant>,
}

impl Counters {
    fn new() -> Self {
        Self {
            last_emit: Some(Instant::now()),
            ..Default::default()
        }
    }
}

fn copy_tree(
    src: &Path,
    dst: &Path,
    overwrite: bool,
    hardlink: bool,
    counters: &mut Counters,
    sink: &dyn ImportSink,
    cancel: &CancelToken,
) -> Result<()> {
    std::fs::create_dir_all(dst).at(dst)?;
    let mut stack = vec![(src.to_path_buf(), dst.to_path_buf())];
    while let Some((sdir, ddir)) = stack.pop() {
        check_cancel(cancel)?;
        let entries = match std::fs::read_dir(&sdir) {
            Ok(entries) => entries,
            Err(e) => {
                counters.failed += 1;
                sink.emit(ImportProgress::Log {
                    level: LogLevel::Warn,
                    message: format!("读不了 {}：{e}", sdir.display()),
                });
                continue;
            }
        };
        for entry in entries.flatten() {
            check_cancel(cancel)?;
            let spath = entry.path();
            let dpath = ddir.join(entry.file_name());
            let Ok(ft) = entry.file_type() else {
                counters.skipped += 1;
                continue;
            };
            if ft.is_symlink() {
                // 不跟符号链接：既避免环，也避免把源目录外面的东西拖进来。
                counters.skipped += 1;
                continue;
            }
            if ft.is_dir() {
                if let Err(e) = std::fs::create_dir_all(&dpath) {
                    counters.failed += 1;
                    sink.emit(ImportProgress::Log {
                        level: LogLevel::Warn,
                        message: format!("建不了 {}：{e}", dpath.display()),
                    });
                    continue;
                }
                stack.push((spath, dpath));
                continue;
            }
            if !ft.is_file() {
                counters.skipped += 1;
                continue;
            }

            counters.processed += 1;
            if dpath.exists() && !overwrite {
                counters.skipped += 1;
            } else {
                match copy_one(&spath, &dpath, hardlink) {
                    Ok((bytes, linked)) => {
                        counters.copied += 1;
                        counters.copied_bytes += bytes;
                        counters.bytes_done += bytes;
                        if linked {
                            counters.links += 1;
                        }
                    }
                    Err(e) => {
                        counters.failed += 1;
                        sink.emit(ImportProgress::Log {
                            level: LogLevel::Warn,
                            message: format!("复制失败 {}：{e}", spath.display()),
                        });
                    }
                }
            }
            maybe_emit(counters, sink, &dpath);
        }
    }
    Ok(())
}

/// 复制一个文件。返回（字节数，是否走了硬链）。
fn copy_one(src: &Path, dst: &Path, hardlink: bool) -> std::io::Result<(u64, bool)> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // 跨卷、文件系统不支持硬链：退回复制，不算失败。
    if hardlink && std::fs::hard_link(src, dst).is_ok() {
        let bytes = std::fs::metadata(dst).map(|m| m.len()).unwrap_or(0);
        return Ok((bytes, true));
    }
    let bytes = std::fs::copy(src, dst)?;
    Ok((bytes, false))
}

fn maybe_emit(counters: &mut Counters, sink: &dyn ImportSink, current: &Path) {
    let now = Instant::now();
    let due = counters.processed % 24 == 0
        || counters
            .last_emit
            .is_none_or(|at| now.duration_since(at) >= Duration::from_millis(80))
        || counters.processed >= counters.planned_files;
    if !due {
        return;
    }
    counters.last_emit = Some(now);
    sink.emit(ImportProgress::Files {
        done: counters.processed,
        total: counters.planned_files,
        bytes_done: counters.bytes_done,
        bytes_total: counters.planned_bytes,
        current: current.to_string_lossy().into_owned(),
    });
}

/// 源目录 → 真正的游戏目录。Prism / MultiMC 实例外面套了一层 `minecraft/`。
pub fn resolve_game_dir(path: &Path) -> PathBuf {
    let inner = path.join("minecraft");
    if inner.is_dir()
        && (path.join("instance.cfg").is_file()
            || path.join("mmc-pack.json").is_file()
            || !path.join("versions").is_dir())
    {
        return inner;
    }
    path.to_path_buf()
}

/// 版本名只能是 `versions/` 下的一个目录名。
fn sanitize_version_id(raw: &str) -> Result<String> {
    let id = raw.trim();
    if id.is_empty() || id.contains(['/', '\\']) || id == "." || id == ".." {
        return Err(Error::other(format!("版本名不合法：{raw}")));
    }
    Ok(id.to_string())
}

// ---------------------------------------------------------------- 小工具

fn read_small_text(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    // 配置文件都很小；遇到大文件直接跳过，别把内存吃光。
    if !meta.is_file() || meta.len() > 4 * 1024 * 1024 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let text: &str = &text;
    Some(text.strip_prefix('\u{feff}').unwrap_or(text).to_string())
}

fn read_json(path: &Path) -> Option<serde_json::Value> {
    let text = read_small_text(path)?;
    serde_json::from_str(&text).ok()
}

fn dir_has_entries(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|mut it| it.next().is_some())
        .unwrap_or(false)
}

fn modified_secs(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn push_unique<T: PartialEq>(list: &mut Vec<T>, value: T) {
    if !list.iter().any(|v| v == &value) {
        list.push(value);
    }
}

/// 路径去重用的 key：统一分隔符、去掉末尾分隔符，Windows 下不分大小写。
fn norm_key(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('\\', "/");
    let trimmed = raw.trim_end_matches('/');
    let trimmed = if trimmed.is_empty() { "/" } else { trimmed };
    if cfg!(windows) {
        trimmed.to_ascii_lowercase()
    } else {
        trimmed.to_string()
    }
}

/// 目录里前 `limit` 个文本文件（只看一层，BakaXL 的配置就一层）。
fn text_files_in(dir: &Path, limit: usize) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .map(|x| {
                        let x = x.to_string_lossy().to_ascii_lowercase();
                        matches!(
                            x.as_str(),
                            "json" | "config" | "ini" | "xml" | "cfg" | "txt"
                        )
                    })
                    .unwrap_or(false)
        })
        .take(limit)
        .collect()
}

// ---------------------------------------------------------------- 测试

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // ---------- 临时目录 ----------

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            use std::sync::atomic::AtomicUsize;
            static SEQ: AtomicUsize = AtomicUsize::new(0);
            let n = SEQ.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("mcdl-import-{tag}-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("建临时目录");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("建父目录");
        }
        std::fs::write(path, content).expect("写文件");
    }

    fn mkdir(path: &Path) {
        std::fs::create_dir_all(path).expect("建目录");
    }

    /// 造一套「本机环境」：appdata 和 data_home 同源（Windows 就是这样）。
    fn env_at(root: &Path) -> ScanEnv {
        ScanEnv {
            app_data: Some(root.join("appdata")),
            data_home: Some(root.join("appdata")),
            local_data: Some(root.join("localappdata")),
            home: Some(root.join("home")),
        }
    }

    /// 造一个能认出来的版本目录。
    fn fake_version(game_dir: &Path, id: &str, main_class: &str) {
        write(
            &game_dir
                .join("versions")
                .join(id)
                .join(format!("{id}.json")),
            &format!(r#"{{"id":"{id}","type":"release","mainClass":"{main_class}"}}"#),
        );
    }

    // ---------- ini ----------

    #[test]
    fn ini_parses_sections_comments_and_colons() {
        let ini = Ini::parse(
            "\u{feff}; 注释\r\n[Setup]\r\nLaunchFolderSelect:0=C:\\mc\\.minecraft\r\nLaunchFolderSelect:1 = \"D:\\other\\.minecraft\"\r\n# 另一行注释\r\nName=PCL\r\n",
        );
        assert_eq!(
            ini.path_values(),
            vec![
                "C:\\mc\\.minecraft".to_string(),
                "D:\\other\\.minecraft".to_string()
            ]
        );
        assert_eq!(ini.get_any("name").as_deref(), Some("PCL"));
        assert_eq!(ini.get_any("nope"), None);
        assert_eq!(ini.get_in("Setup", "Name").as_deref(), Some("PCL"));
        assert_eq!(ini.get_in("Other", "Name"), None);
    }

    #[test]
    fn ini_ignores_lines_without_equals() {
        let ini = Ini::parse("garbage line\nkey=value\n[sec]\nother=1");
        assert_eq!(ini.get_any("key").as_deref(), Some("value"));
        assert_eq!(ini.get_any("other").as_deref(), Some("1"));
    }

    // ---------- 路径识别 ----------

    #[test]
    fn recognises_absolute_paths() {
        assert!(looks_like_absolute_path("C:\\mc\\.minecraft"));
        assert!(looks_like_absolute_path("D:/mc/.minecraft"));
        assert!(looks_like_absolute_path("/home/u/.minecraft"));
        assert!(looks_like_absolute_path("\\\\server\\share\\mc"));
        assert!(!looks_like_absolute_path(".minecraft"));
        assert!(!looks_like_absolute_path("1.21.1"));
        assert!(!looks_like_absolute_path(""));
        assert!(!looks_like_absolute_path("C:"));
    }

    #[test]
    fn harvest_keeps_only_real_game_dirs() {
        let tmp = TempDir::new("harvest");
        let mc = tmp.path().join("appdata/.minecraft");
        mkdir(&mc.join("versions"));
        let other = tmp.path().join("not-mc");
        mkdir(&other);

        let text = format!(
            r#"{{"config":{{"gameDir":"{}"}},"junk":"C:\\nope\\.minecraft","dir":"{}"}}"#,
            mc.to_string_lossy(),
            other.to_string_lossy()
        );
        let found = harvest_paths(&text);
        assert_eq!(found.len(), 1, "只该留下真的游戏目录：{found:?}");
        assert_eq!(norm_key(&found[0]), norm_key(&mc));
    }

    #[test]
    fn harvest_ignores_junk_tokens() {
        assert!(harvest_paths("hello world, 1.21.1, \"C:\\nope\" not a path").is_empty());
        assert!(harvest_paths("").is_empty());
    }

    // ---------- PCL ----------

    #[test]
    fn pcl_setup_ini_lists_other_dirs() {
        let tmp = TempDir::new("pcl");
        let env = env_at(tmp.path());
        let app = env.app_data.clone().unwrap();

        // 默认游戏目录 + PCL 自己的目录
        let mc = app.join(".minecraft");
        mkdir(&mc.join("versions"));
        let other = tmp.path().join("D_drive/.minecraft");
        mkdir(&other.join("versions"));
        write(
            &mc.join("PCL").join("Setup.ini"),
            &format!(
                "[Setup]\nLaunchFolderSelect:0={}\nLaunchFolderSelect:1={}\n",
                mc.to_string_lossy(),
                other.to_string_lossy()
            ),
        );
        fake_version(&mc, "1.21.1", "net.minecraft.client.main.Main");
        fake_version(
            &other,
            "1.20.1",
            "net.fabricmc.loader.impl.launch.knot.KnotClient",
        );

        let result = scan(&env);
        let paths: Vec<String> = result.items.iter().map(|i| norm_key(&i.path)).collect();
        assert!(paths.contains(&norm_key(&mc)), "默认目录要在：{paths:?}");
        assert!(
            paths.contains(&norm_key(&other)),
            "ini 里的目录要在：{paths:?}"
        );

        let item = result
            .items
            .iter()
            .find(|i| norm_key(&i.path) == norm_key(&other))
            .unwrap();
        assert_eq!(item.main_launcher(), Launcher::Pcl);
        assert_eq!(item.versions.len(), 1);
        assert_eq!(item.versions[0].loader.as_deref(), Some("Fabric"));

        let probe = result
            .probes
            .iter()
            .find(|p| p.path == mc.join("PCL").join("Setup.ini"))
            .expect("要留下 Setup.ini 的探针");
        assert!(probe.found);
        assert!(probe.note.contains("2 个目录"), "{}", probe.note);
    }

    #[test]
    fn pcl_ce_marker_switches_launcher() {
        let tmp = TempDir::new("pclce");
        let env = env_at(tmp.path());
        let mc = env.app_data.clone().unwrap().join(".minecraft");
        mkdir(&mc.join("versions"));
        write(
            &mc.join("PCL").join("Setup.ini"),
            "[Setup]\nEdition=PCLCE\nLaunchFolderSelect:0=\n",
        );
        let result = scan(&env);
        let item = result
            .items
            .iter()
            .find(|i| i.kind == ItemKind::GameDir)
            .unwrap();
        assert!(
            item.launchers.contains(&Launcher::PclCe),
            "{:?}",
            item.launchers
        );
        assert_eq!(Launcher::PclCe.label(), "PCL CE");
    }

    #[test]
    fn pcl_config_dir_marks_default_game_dir() {
        let tmp = TempDir::new("pclcfg");
        let env = env_at(tmp.path());
        let app = env.app_data.clone().unwrap();
        mkdir(&app.join("PCL"));
        write(&app.join("PCL").join("config.json"), r#"{"JavaList":[]}"#);
        let mc = app.join(".minecraft");
        mkdir(&mc.join("versions"));

        let result = scan(&env);
        let item = result
            .items
            .iter()
            .find(|i| norm_key(&i.path) == norm_key(&mc))
            .expect("PCL 配置目录存在时，默认 .minecraft 也要列出来");
        assert_eq!(item.main_launcher(), Launcher::Pcl);
        assert!(item.versions.is_empty());
        assert!(item.evidence.iter().any(|e| e.contains("PCL")));
    }

    // ---------- HMCL ----------

    #[test]
    fn hmcl_json_game_dir_and_multi_dirs() {
        let tmp = TempDir::new("hmcl");
        let env = env_at(tmp.path());
        let app = env.app_data.clone().unwrap();
        let hmcl = app.join(".hmcl");
        let mc_a = tmp.path().join("A/.minecraft");
        let mc_b = tmp.path().join("B/.minecraft");
        for mc in [&mc_a, &mc_b] {
            mkdir(&mc.join("versions"));
        }
        fake_version(&mc_a, "1.19.2", "net.minecraft.client.main.Main");

        // 字段名各版本不同：这里混着放，靠「值像游戏目录」来认
        write(
            &hmcl.join("hmcl.json"),
            &format!(
                r#"{{"config":{{"gameDir":"{}","gameDirs":[{{"dir":"{}"}}]}}}}"#,
                mc_a.to_string_lossy(),
                mc_b.to_string_lossy()
            ),
        );

        let result = scan(&env);
        let keys: Vec<String> = result.items.iter().map(|i| norm_key(&i.path)).collect();
        assert!(keys.contains(&norm_key(&mc_a)), "{keys:?}");
        assert!(keys.contains(&norm_key(&mc_b)), "{keys:?}");
        let item = result
            .items
            .iter()
            .find(|i| norm_key(&i.path) == norm_key(&mc_a))
            .unwrap();
        assert_eq!(item.main_launcher(), Launcher::Hmcl);
        assert_eq!(item.versions[0].id, "1.19.2");
    }

    #[test]
    fn hmcl_missing_file_leaves_probe() {
        let tmp = TempDir::new("hmcl-miss");
        let env = env_at(tmp.path());
        let result = scan(&env);
        assert!(result.items.is_empty());
        let probe = result
            .probes
            .iter()
            .find(|p| p.launcher == Launcher::Hmcl)
            .expect("没找到也要留探针");
        assert!(!probe.found);
    }

    // ---------- BakaXL ----------

    #[test]
    fn bakaxl_config_is_harvested() {
        let tmp = TempDir::new("baka");
        let env = env_at(tmp.path());
        let app = env.app_data.clone().unwrap();
        let baka = app.join("BakaXL");
        let mc = tmp.path().join("Baka/.minecraft");
        mkdir(&mc.join("versions"));
        fake_version(&mc, "1.16.5", "net.minecraft.client.main.Main");
        write(
            &baka.join("Core.config"),
            &format!("MinecraftFolder={}\nOther=1\n", mc.to_string_lossy()),
        );

        let result = scan(&env);
        let item = result
            .items
            .iter()
            .find(|i| norm_key(&i.path) == norm_key(&mc))
            .expect("BakaXL 配置里的目录要能扫到");
        assert_eq!(item.main_launcher(), Launcher::BakaXl);
        assert_eq!(item.versions[0].id, "1.16.5");
    }

    // ---------- 官方启动器 ----------

    #[test]
    fn non_empty_profiles_means_vanilla() {
        let tmp = TempDir::new("vanilla");
        let env = env_at(tmp.path());
        let mc = env.app_data.clone().unwrap().join(".minecraft");
        mkdir(&mc.join("versions"));
        write(
            &mc.join("launcher_profiles.json"),
            r#"{"profiles":{"1.21.1":{"name":"1.21.1"}}}"#,
        );
        let result = scan(&env);
        let item = result
            .items
            .iter()
            .find(|i| norm_key(&i.path) == norm_key(&mc))
            .unwrap();
        assert_eq!(item.main_launcher(), Launcher::Vanilla);
    }

    #[test]
    fn empty_profiles_is_not_vanilla() {
        // MCDL 自己装 Forge 时会写 {"profiles":{}}，不能把它当成官方启动器装过。
        let tmp = TempDir::new("vanilla-empty");
        let env = env_at(tmp.path());
        let mc = env.app_data.clone().unwrap().join(".minecraft");
        mkdir(&mc.join("versions"));
        write(&mc.join("launcher_profiles.json"), r#"{"profiles":{}}"#);
        let result = scan(&env);
        assert!(
            result.items.is_empty(),
            "空 profiles + 空 versions 不该列出来：{:?}",
            result.items.iter().map(|i| &i.path).collect::<Vec<_>>()
        );
    }

    // ---------- Prism / MultiMC ----------

    #[test]
    fn prism_instance_reports_version_and_loader() {
        let tmp = TempDir::new("prism");
        let env = env_at(tmp.path());
        let instances = env
            .app_data
            .clone()
            .unwrap()
            .join("PrismLauncher")
            .join("instances");
        let inst = instances.join("1.20.1 Fabric");
        write(
            &inst.join("instance.cfg"),
            "name=我的整合包\nInstanceType=OneSix\n",
        );
        write(
            &inst.join("mmc-pack.json"),
            r#"{"formatVersion":1,"components":[
                 {"uid":"net.minecraft","version":"1.20.1","important":true},
                 {"uid":"net.fabricmc.fabric-loader","version":"0.15.11"},
                 {"uid":"org.lwjgl3","version":"3.3.1"}
               ]}"#,
        );
        write(&inst.join("minecraft").join("mods").join("sodium.jar"), "x");

        let result = scan(&env);
        let item = result
            .items
            .iter()
            .find(|i| i.kind == ItemKind::PrismInstance)
            .expect("Prism 实例要扫到");
        assert_eq!(item.name, "我的整合包");
        assert_eq!(item.main_launcher(), Launcher::Prism);
        assert!(item.has_mods);
        assert_eq!(item.versions.len(), 1);
        assert_eq!(item.versions[0].id, "1.20.1");
        assert_eq!(item.versions[0].loader.as_deref(), Some("Fabric 0.15.11"));
        assert!(!item.versions[0].importable, "实例没有独立版本目录");
        assert_eq!(item.game_dir, inst.join("minecraft"));
        assert!(result
            .probes
            .iter()
            .any(|p| p.launcher == Launcher::Prism && p.found));
    }

    #[test]
    fn multimc_instance_with_real_versions_dir() {
        let tmp = TempDir::new("multimc");
        let env = env_at(tmp.path());
        let inst = env
            .app_data
            .clone()
            .unwrap()
            .join("MultiMC")
            .join("instances")
            .join("Vanilla");
        mkdir(&inst.join("minecraft/versions"));
        write(&inst.join("instance.cfg"), "name=原版实例\n");
        write(
            &inst.join("mmc-pack.json"),
            r#"{"components":[{"uid":"net.minecraft","version":"1.8.9"}]}"#,
        );
        fake_version(
            &inst.join("minecraft"),
            "1.8.9",
            "net.minecraft.client.main.Main",
        );

        let result = scan(&env);
        let item = result
            .items
            .iter()
            .find(|i| i.kind == ItemKind::MultiMcInstance)
            .unwrap();
        assert_eq!(item.versions.len(), 1);
        assert!(item.versions[0].importable);
        assert_eq!(item.versions[0].id, "1.8.9");
        assert_eq!(item.loaders(), Vec::<String>::new());
    }

    // ---------- 别的平台 ----------

    #[test]
    fn linux_and_macos_layouts_are_probed() {
        let tmp = TempDir::new("posix");
        // 只给 home，模拟 Linux / macOS：官方目录在 home 下面
        let env = ScanEnv {
            app_data: None,
            data_home: None,
            local_data: None,
            home: Some(tmp.path().join("home")),
        };
        let linux = tmp.path().join("home/.minecraft");
        let macos = tmp
            .path()
            .join("home/Library/Application Support/minecraft");
        for mc in [&linux, &macos] {
            mkdir(&mc.join("versions"));
        }
        fake_version(&linux, "1.21.4", "net.minecraft.client.main.Main");
        fake_version(&macos, "1.20.6", "net.minecraft.client.main.Main");

        let result = scan(&env);
        let keys: Vec<String> = result.items.iter().map(|i| norm_key(&i.path)).collect();
        assert!(keys.contains(&norm_key(&linux)), "{keys:?}");
        assert!(keys.contains(&norm_key(&macos)), "{keys:?}");
    }

    #[test]
    fn nothing_found_is_empty_not_error() {
        let tmp = TempDir::new("empty");
        let result = scan(&env_at(tmp.path()));
        assert!(result.items.is_empty());
        assert!(result.version_count() == 0);
        assert!(!result.probes.is_empty(), "探针要说明扫过哪");
        assert!(result.probes.iter().all(|p| !p.found));
    }

    #[test]
    fn unknown_launcher_dir_with_versions_is_listed() {
        let tmp = TempDir::new("unknown");
        let env = env_at(tmp.path());
        let mc = tmp.path().join("weird/.minecraft");
        mkdir(&mc.join("versions"));
        fake_version(&mc, "1.7.10", "net.minecraft.client.main.Main");
        // 没人记着它，只有一条 BakaXL 风格的配置把它指出来
        write(
            &env.app_data.clone().unwrap().join("BakaXL").join("c.json"),
            &format!(r#"{{"dir":"{}"}}"#, mc.to_string_lossy()),
        );
        let result = scan(&env);
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].versions[0].id, "1.7.10");
    }

    // ---------- 体积 ----------

    #[test]
    fn measure_item_counts_versions_mods_config() {
        let tmp = TempDir::new("measure");
        let env = env_at(tmp.path());
        let mc = env.app_data.clone().unwrap().join(".minecraft");
        fake_version(&mc, "1.21.1", "net.minecraft.client.main.Main");
        write(&mc.join("versions/1.21.1/1.21.1.jar"), "0123456789");
        write(&mc.join("mods/a.jar"), "12345");
        write(&mc.join("config/x.toml"), "12");
        // libraries 不算：共享资源不参与导入
        write(&mc.join("libraries/big.jar"), &"x".repeat(10_000));

        let result = scan(&env);
        let item = result.items.iter().find(|i| i.game_dir == mc).unwrap();
        let size = measure_item(item);
        // versions/1.21.1 里是版本 JSON + jar；mods 1 个、config 1 个
        assert_eq!(size.files, 4, "只数版本目录 + mods + config");
        assert!(size.bytes >= 10 + 5 + 2, "bytes={}", size.bytes);
        assert!(size.bytes < 10_000, "libraries 不该算进来：{}", size.bytes);
    }

    #[test]
    fn measure_of_missing_dir_is_zero() {
        let tmp = TempDir::new("measure-miss");
        assert_eq!(measure(&tmp.path().join("nope")), DirSize::default());
    }

    // ---------- 导入 ----------

    /// 造一个源目录：1 个版本 + mods + config + saves。
    fn fake_source(root: &Path) -> PathBuf {
        let src = root.join("src/.minecraft");
        fake_version(&src, "1.21.1", "net.minecraft.client.main.Main");
        write(&src.join("versions/1.21.1/1.21.1.jar"), "jar-bytes");
        write(&src.join("mods/sodium.jar"), "mod");
        write(&src.join("mods/lib/extra.jar"), "extra");
        write(&src.join("config/sodium.toml"), "cfg");
        write(&src.join("saves/world/level.dat"), "save");
        src
    }

    #[test]
    fn import_copies_versions_mods_config_and_leaves_source_alone() {
        let tmp = TempDir::new("import");
        let src = fake_source(tmp.path());
        let dst = tmp.path().join("dst/.minecraft");

        let mut req = ImportRequest::new(&src, &dst);
        req.version_ids = vec!["1.21.1".to_string()];
        req.isolated = false; // 共享目录模式：模组进根 mods/
        let report = import(&req, &NullSink, &cancel_token()).expect("导入该成功");

        assert_eq!(report.versions, vec!["1.21.1".to_string()]);
        assert!(report.files >= 4, "files={}", report.files);
        assert!(report.bytes > 0);
        assert_eq!(report.skipped_files, 0);
        assert!(dst.join("versions/1.21.1/1.21.1.json").is_file());
        assert!(dst.join("versions/1.21.1/1.21.1.jar").is_file());
        assert!(dst.join("mods/sodium.jar").is_file());
        assert!(dst.join("mods/lib/extra.jar").is_file());
        assert!(dst.join("config/sodium.toml").is_file());
        assert!(
            !dst.join("saves/world/level.dat").exists(),
            "没勾 saves 就不该搬"
        );

        // 源目录原封不动
        assert_eq!(
            std::fs::read_to_string(src.join("mods/sodium.jar")).unwrap(),
            "mod"
        );
        assert!(src.join("versions/1.21.1/1.21.1.jar").is_file());
    }

    /// 隔离开着（默认）：源 mods 跟着导入的版本进 `versions/<id>/mods/`，
    /// 根目录不该冒出 mods/；config 仍按共享目录走。
    #[test]
    fn import_puts_mods_into_the_version_dir_when_isolated() {
        let tmp = TempDir::new("import-iso");
        let src = fake_source(tmp.path());
        let dst = tmp.path().join("dst/.minecraft");

        let mut req = ImportRequest::new(&src, &dst);
        req.version_ids = vec!["1.21.1".to_string()];
        assert!(req.isolated, "导入默认就是隔离的");
        import(&req, &NullSink, &cancel_token()).expect("导入该成功");

        assert!(dst.join("versions/1.21.1/mods/sodium.jar").is_file());
        assert!(dst.join("versions/1.21.1/mods/lib/extra.jar").is_file());
        assert!(!dst.join("mods").exists(), "隔离开着不该在根目录建 mods/");
        assert!(
            dst.join("config/sodium.toml").is_file(),
            "config 仍走共享目录"
        );
    }

    /// 没有可导入的版本时，隔离开着也只能放共享目录——但不能静默这么做，
    /// 日志里必须有一条说明。
    #[test]
    fn import_without_versions_falls_back_to_shared_mods_and_says_so() {
        struct Collect(Mutex<Vec<String>>);
        impl ImportSink for Collect {
            fn emit(&self, progress: ImportProgress) {
                if let ImportProgress::Log { message, .. } = progress {
                    self.0.lock().unwrap().push(message);
                }
            }
        }

        let tmp = TempDir::new("import-iso-fallback");
        let src = fake_source(tmp.path());
        let dst = tmp.path().join("dst/.minecraft");
        let sink = Collect(Mutex::new(Vec::new()));

        let mut req = ImportRequest::new(&src, &dst);
        req.version_ids.clear();
        req.include_config = false;
        import(&req, &sink, &cancel_token()).expect("导入该成功");

        assert!(dst.join("mods/sodium.jar").is_file());
        let logs = sink.0.lock().unwrap();
        assert!(
            logs.iter().any(|m| m.contains("没有可导入的版本")),
            "缺一条回退说明：{logs:?}"
        );
    }

    #[test]
    fn import_skips_existing_version_instead_of_overwriting() {
        let tmp = TempDir::new("import-skip");
        let src = fake_source(tmp.path());
        let dst = tmp.path().join("dst/.minecraft");
        // 目标里已经有同名版本，内容不一样
        write(&dst.join("versions/1.21.1/1.21.1.json"), "keep-me");

        let mut req = ImportRequest::new(&src, &dst);
        req.version_ids = vec!["1.21.1".to_string()];
        req.include_mods = false;
        req.include_config = false;
        let report = import(&req, &NullSink, &cancel_token()).unwrap();

        assert!(report.versions.is_empty());
        assert_eq!(report.skipped_versions, vec!["1.21.1".to_string()]);
        assert_eq!(
            std::fs::read_to_string(dst.join("versions/1.21.1/1.21.1.json")).unwrap(),
            "keep-me",
            "已存在的不覆盖"
        );
    }

    #[test]
    fn import_overwrite_replaces_when_asked() {
        let tmp = TempDir::new("import-over");
        let src = fake_source(tmp.path());
        let dst = tmp.path().join("dst/.minecraft");
        write(&dst.join("versions/1.21.1/1.21.1.json"), "old");

        let mut req = ImportRequest::new(&src, &dst);
        req.version_ids = vec!["1.21.1".to_string()];
        req.include_mods = false;
        req.include_config = false;
        req.overwrite = true;
        let report = import(&req, &NullSink, &cancel_token()).unwrap();
        assert_eq!(report.versions, vec!["1.21.1".to_string()]);
        assert!(
            std::fs::read_to_string(dst.join("versions/1.21.1/1.21.1.json"))
                .unwrap()
                .contains("mainClass")
        );
    }

    #[test]
    fn import_reports_progress_to_the_sink() {
        struct Collect(Mutex<Vec<ImportProgress>>);
        impl ImportSink for Collect {
            fn emit(&self, progress: ImportProgress) {
                self.0.lock().unwrap().push(progress);
            }
        }

        let tmp = TempDir::new("import-progress");
        let src = fake_source(tmp.path());
        let dst = tmp.path().join("dst/.minecraft");
        let sink = Collect(Mutex::new(Vec::new()));
        let mut req = ImportRequest::new(&src, &dst);
        req.version_ids = vec!["1.21.1".to_string()];
        import(&req, &sink, &cancel_token()).unwrap();

        let events = sink.0.lock().unwrap();
        let stages = events
            .iter()
            .filter(|e| matches!(e, ImportProgress::Stage { .. }))
            .count();
        assert_eq!(stages, 3, "版本 + mods + config");
        let last_files = events
            .iter()
            .filter_map(|e| match e {
                ImportProgress::Files { done, total, .. } => Some((*done, *total)),
                _ => None,
            })
            .next_back()
            .expect("要有文件进度");
        assert_eq!(last_files.0, last_files.1, "进度要走到头");
        assert!(last_files.1 >= 4);
    }

    #[test]
    fn import_can_be_cancelled() {
        let tmp = TempDir::new("import-cancel");
        let src = fake_source(tmp.path());
        let dst = tmp.path().join("dst/.minecraft");
        let token = cancel_token();
        token.store(true, Ordering::Relaxed);
        let mut req = ImportRequest::new(&src, &dst);
        req.version_ids = vec!["1.21.1".to_string()];
        let err = import(&req, &NullSink, &token).unwrap_err();
        assert!(matches!(err, Error::Cancelled), "{err:?}");
    }

    #[test]
    fn import_rejects_same_or_nested_dirs() {
        let tmp = TempDir::new("import-same");
        let src = fake_source(tmp.path());
        let mut req = ImportRequest::new(&src, &src);
        req.version_ids = vec!["1.21.1".to_string()];
        assert!(import(&req, &NullSink, &cancel_token()).is_err());

        // 目标在源里面
        let mut req = ImportRequest::new(&src, src.join("inner"));
        req.version_ids = vec!["1.21.1".to_string()];
        assert!(import(&req, &NullSink, &cancel_token()).is_err());
    }

    #[test]
    fn import_rejects_path_traversal_version_id() {
        let tmp = TempDir::new("import-traversal");
        let src = fake_source(tmp.path());
        let dst = tmp.path().join("dst/.minecraft");
        for bad in ["../etc", "a/b", "a\\b", "..", "."] {
            let mut req = ImportRequest::new(&src, &dst);
            req.version_ids = vec![bad.to_string()];
            assert!(import(&req, &NullSink, &cancel_token()).is_err(), "{bad}");
        }
    }

    #[test]
    fn import_from_prism_instance_walks_into_minecraft() {
        let tmp = TempDir::new("import-prism");
        let inst = tmp.path().join("instances/Pack");
        write(&inst.join("instance.cfg"), "name=Pack\n");
        write(
            &inst.join("mmc-pack.json"),
            r#"{"components":[{"uid":"net.minecraft","version":"1.20.1"}]}"#,
        );
        write(&inst.join("minecraft/mods/x.jar"), "mod");
        write(&inst.join("minecraft/config/y.toml"), "cfg");
        let dst = tmp.path().join("dst/.minecraft");

        // 实例没有独立版本目录：只搬 mods/config
        let req = ImportRequest::new(&inst, &dst);
        let report = import(&req, &NullSink, &cancel_token()).unwrap();
        assert!(report.versions.is_empty());
        assert!(dst.join("mods/x.jar").is_file());
        assert!(dst.join("config/y.toml").is_file());
        assert!(report.files >= 2);
    }

    #[test]
    fn import_hardlink_works_or_falls_back() {
        let tmp = TempDir::new("import-link");
        let src = fake_source(tmp.path());
        let dst = tmp.path().join("dst/.minecraft");
        let mut req = ImportRequest::new(&src, &dst);
        req.version_ids = vec!["1.21.1".to_string()];
        req.include_config = false;
        req.hardlink = true;
        let report = import(&req, &NullSink, &cancel_token()).unwrap();
        assert!(report.files >= 3);
        // 同卷时走硬链，跨卷时退回复制——两种都算成功
        assert!(report.links + report.files >= 3);
        // 默认隔离：模组跟着版本进 versions/<id>/mods/
        assert!(dst.join("versions/1.21.1/mods/sodium.jar").is_file());
    }

    #[test]
    fn import_with_nothing_to_do_is_ok() {
        let tmp = TempDir::new("import-none");
        let src = fake_source(tmp.path());
        let dst = tmp.path().join("dst/.minecraft");
        let mut req = ImportRequest::new(&src, &dst);
        req.include_mods = false;
        req.include_config = false;
        let report = import(&req, &NullSink, &cancel_token()).unwrap();
        assert!(report.is_empty());
    }

    // ---------- 其它 ----------

    #[test]
    fn norm_key_normalises_separators() {
        let a = norm_key(Path::new("C:\\mc\\.minecraft\\"));
        let b = norm_key(Path::new("c:/mc/.minecraft"));
        assert_eq!(a, b);
    }

    #[test]
    fn resolve_game_dir_detects_instance() {
        let tmp = TempDir::new("resolve");
        let inst = tmp.path().join("inst");
        mkdir(&inst.join("minecraft"));
        write(&inst.join("instance.cfg"), "name=x\n");
        assert_eq!(resolve_game_dir(&inst), inst.join("minecraft"));

        let plain = tmp.path().join("plain");
        mkdir(&plain.join("versions"));
        assert_eq!(resolve_game_dir(&plain), plain);
    }

    #[test]
    fn launcher_labels_are_chinese() {
        assert_eq!(Launcher::Pcl.label(), "PCL");
        assert_eq!(Launcher::Hmcl.label(), "HMCL");
        assert_eq!(Launcher::Vanilla.label(), "官方启动器");
        assert_eq!(ItemKind::PrismInstance.label(), "Prism 实例");
        assert!(ItemKind::PrismInstance.is_instance());
        assert!(!Launcher::Unknown.is_known());
    }
}
