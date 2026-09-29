//! Profile（配置组）= profiles 下的一个子文件夹。
//!
//! 文件夹里的音频文件会被自动扫描进列表；每个音频的快捷键 / 单独音量 / 顺序
//! 记在同目录的 `_schemes.json` 里（以文件名为键），移动/改名音频不丢绑定。
//!
//! 同一批音频可以有好几套「方案」：每套方案有自己的键位、音量、顺序，
//! 外加设备和播放设置（[`SchemeSettings`]），切一下就整套换掉。
//! 老版本的 `_bindings.json` 第一次加载时会被当成一套方案迁移进来（原文件不动）。

use crate::config::{SchemeSettings, SortMode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 支持的音频扩展名（小写）。
const AUDIO_EXTS: &[&str] = &["mp3", "wav", "ogg", "flac", "m4a", "aac"];

/// 一个音效条目。
#[derive(Debug, Clone)]
pub struct Sound {
    /// 展示名（文件名去扩展名）。
    pub name: String,
    /// 音频文件绝对路径。
    pub path: PathBuf,
    /// 绑定的快捷键，如 "Ctrl+Alt+S"；None = 未绑定。
    pub hotkey: Option<String>,
    /// 单独音量倍率（默认 1.0）。
    pub volume: f32,
    /// 文件修改时间（按时间排序用）。取不到时是 UNIX 纪元。
    pub modified: std::time::SystemTime,
}

/// 持久化到 `_bindings.json` 的单条绑定信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Binding {
    #[serde(default)]
    hotkey: Option<String>,
    #[serde(default = "default_volume")]
    volume: f32,
    /// 手动排序位置。None = 还没排过（新拖进来的文件），按名字排在最后。
    #[serde(default)]
    order: Option<u32>,
}

fn default_volume() -> f32 {
    1.0
}

/// 一套方案。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Scheme {
    /// 文件名 -> 绑定。
    #[serde(default)]
    bindings: BTreeMap<String, Binding>,
    /// None = 从老的 `_bindings.json` 迁移过来、还没记过设置，由界面用当前设置补上。
    #[serde(default)]
    settings: Option<SchemeSettings>,
}

/// `_schemes.json` 的内容。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SchemeFile {
    #[serde(default)]
    active: String,
    #[serde(default)]
    schemes: BTreeMap<String, Scheme>,
}

/// 一个 profile。
#[derive(Debug, Clone)]
pub struct Profile {
    pub name: String,
    pub dir: PathBuf,
    pub sounds: Vec<Sound>,
    /// 当前方案名。
    pub scheme: String,
    /// 全部方案。**当前方案**的键位 / 音量 / 顺序以 `sounds` 为准，存盘时才写回这里。
    schemes: BTreeMap<String, Scheme>,
}

fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// 列出所有 profile 名（profiles 下的子文件夹）。若一个都没有，创建「默认」。
pub fn list_profiles(default_name: &str) -> Vec<String> {
    let dir = crate::config::profiles_dir();
    let _ = std::fs::create_dir_all(&dir);
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    if names.is_empty() {
        let _ = std::fs::create_dir_all(dir.join(default_name));
        names.push(default_name.to_string());
    }
    names.sort();
    names
}

/// 新建一个 profile 文件夹。
pub fn create_profile(name: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(crate::config::profiles_dir().join(name))
}

impl Profile {
    fn schemes_file(dir: &Path) -> PathBuf {
        dir.join("_schemes.json")
    }

    /// 老版本只有一套绑定，存在这里。
    fn legacy_bindings_file(dir: &Path) -> PathBuf {
        dir.join("_bindings.json")
    }

    /// 读 `_schemes.json`；没有就把老的 `_bindings.json` 迁成一套名为 `default_scheme` 的方案。
    fn read_schemes(dir: &Path, default_scheme: &str) -> SchemeFile {
        let mut file: SchemeFile = std::fs::read_to_string(Self::schemes_file(dir))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| {
                let bindings = std::fs::read_to_string(Self::legacy_bindings_file(dir))
                    .ok()
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default();
                SchemeFile {
                    active: default_scheme.to_string(),
                    schemes: BTreeMap::from([(
                        default_scheme.to_string(),
                        Scheme { bindings, settings: None },
                    )]),
                }
            });
        if file.schemes.is_empty() {
            file.schemes.insert(default_scheme.to_string(), Scheme::default());
        }
        if !file.schemes.contains_key(&file.active) {
            file.active = file.schemes.keys().next().cloned().unwrap_or_default();
        }
        file
    }

    /// 扫描文件夹、合并当前方案的绑定，加载出 profile。
    /// `dir` 可以是 profiles 下的子文件夹，也可以是用户选的任意外部文件夹。
    pub fn load(name: &str, dir: &Path, default_scheme: &str) -> Self {
        let dir = dir.to_path_buf();
        let _ = std::fs::create_dir_all(&dir);
        let file = Self::read_schemes(&dir, default_scheme);

        // 扫描音频文件
        let mut sounds: Vec<Sound> = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| p.is_file() && is_audio(p))
                    .map(|p| {
                        let name = p
                            .file_stem()
                            .and_then(|n| n.to_str())
                            .unwrap_or_default()
                            .to_string();
                        let modified = p
                            .metadata()
                            .and_then(|m| m.modified())
                            .unwrap_or(std::time::UNIX_EPOCH);
                        Sound { name, path: p, hotkey: None, volume: 1.0, modified }
                    })
                    .collect()
            })
            .unwrap_or_default();
        if let Some(scheme) = file.schemes.get(&file.active) {
            Self::apply_bindings(&mut sounds, &scheme.bindings);
        }

        Profile {
            name: name.to_string(),
            dir,
            sounds,
            scheme: file.active,
            schemes: file.schemes,
        }
    }

    /// 把一套绑定套到音效列表上：键位、音量，以及按存下的手动顺序重排。
    fn apply_bindings(sounds: &mut [Sound], bindings: &BTreeMap<String, Binding>) {
        let file_name = |s: &Sound| {
            s.path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string()
        };
        for s in sounds.iter_mut() {
            let b = bindings.get(&file_name(s));
            s.hotkey = b.and_then(|b| b.hotkey.clone());
            s.volume = b.map(|b| b.volume).unwrap_or(1.0);
        }
        // 先按手动顺序，这套方案里没排过的（新拖进来的文件）按名字接在最后。
        let order = |s: &Sound| bindings.get(&file_name(s)).and_then(|b| b.order).unwrap_or(u32::MAX);
        sounds.sort_by(|a, b| {
            order(a)
                .cmp(&order(b))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
    }

    /// 把 `sounds` 里的键位 / 音量 / 当前先后顺序记回当前方案（不落盘）。
    fn store_current(&mut self) {
        let mut map: BTreeMap<String, Binding> = BTreeMap::new();
        for (i, s) in self.sounds.iter().enumerate() {
            if let Some(fname) = s.path.file_name().and_then(|n| n.to_str()) {
                map.insert(
                    fname.to_string(),
                    Binding {
                        hotkey: s.hotkey.clone(),
                        volume: s.volume,
                        order: Some(i as u32),
                    },
                );
            }
        }
        self.schemes.entry(self.scheme.clone()).or_default().bindings = map;
    }

    fn write(&self) {
        let file = SchemeFile { active: self.scheme.clone(), schemes: self.schemes.clone() };
        match serde_json::to_string_pretty(&file) {
            Ok(json) => {
                if let Err(e) = std::fs::write(Self::schemes_file(&self.dir), json) {
                    log::error!("写 _schemes.json 失败：{e}");
                }
            }
            Err(e) => log::error!("序列化方案失败：{e}"),
        }
    }

    /// 把当前绑定写回 `_schemes.json`。列表当前的先后顺序会作为 `order` 存下来，
    /// 所以手动拖动排序是持久的。
    pub fn save_bindings(&mut self) {
        self.store_current();
        self.write();
    }

    /// 全部方案名（按名字排好）。
    pub fn scheme_names(&self) -> Vec<String> {
        self.schemes.keys().cloned().collect()
    }

    /// 当前方案记下的设置。None = 还没记过（老数据迁移来的）。
    pub fn settings(&self) -> Option<&SchemeSettings> {
        self.schemes.get(&self.scheme).and_then(|s| s.settings.as_ref())
    }

    /// 把当前设置记进当前方案；和已存的一样就什么都不做（每帧都会调）。
    pub fn sync_settings(&mut self, settings: SchemeSettings) {
        if self.settings() == Some(&settings) {
            return;
        }
        self.schemes.entry(self.scheme.clone()).or_default().settings = Some(settings);
        self.save_bindings();
    }

    /// 切到另一套方案。名字不存在或就是当前方案时返回 false。
    pub fn switch_scheme(&mut self, name: &str) -> bool {
        if name == self.scheme || !self.schemes.contains_key(name) {
            return false;
        }
        self.store_current();
        self.scheme = name.to_string();
        if let Some(scheme) = self.schemes.get(name) {
            Self::apply_bindings(&mut self.sounds, &scheme.bindings);
        }
        self.write();
        true
    }

    /// 新建一套方案并切过去。`copy` = 另存为（带上当前全部键位 / 音量 / 顺序）；
    /// 否则是一套空白键位，设备和播放设置沿用当前的。名字为空或已存在时返回 false。
    pub fn add_scheme(&mut self, name: &str, copy: bool) -> bool {
        let name = name.trim();
        if name.is_empty() || self.schemes.contains_key(name) {
            return false;
        }
        self.store_current();
        let cur = self.schemes.get(&self.scheme).cloned().unwrap_or_default();
        let new = if copy {
            cur
        } else {
            Scheme { bindings: BTreeMap::new(), settings: cur.settings }
        };
        self.schemes.insert(name.to_string(), new);
        self.switch_scheme(name)
    }

    /// 重命名当前方案。名字为空或和别的方案重名时返回 false。
    pub fn rename_scheme(&mut self, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() || self.schemes.contains_key(name) {
            return false;
        }
        self.store_current();
        if let Some(s) = self.schemes.remove(&self.scheme) {
            self.schemes.insert(name.to_string(), s);
        }
        self.scheme = name.to_string();
        self.write();
        true
    }

    /// 删掉当前方案并切到剩下的第一套。只剩一套时不让删，返回 false。
    pub fn delete_scheme(&mut self) -> bool {
        if self.schemes.len() <= 1 {
            return false;
        }
        self.schemes.remove(&self.scheme);
        self.scheme = self.schemes.keys().next().cloned().unwrap_or_default();
        if let Some(scheme) = self.schemes.get(&self.scheme) {
            Self::apply_bindings(&mut self.sounds, &scheme.bindings);
        }
        self.write();
        true
    }

    /// 把第 `from` 项移动到第 `to` 项的位置（拖动排序用）。
    pub fn move_sound(&mut self, from: usize, to: usize) {
        if from == to || from >= self.sounds.len() || to >= self.sounds.len() {
            return;
        }
        let s = self.sounds.remove(from);
        self.sounds.insert(to, s);
    }

    /// 按指定方式重排。`Custom` 保持文件里存的手动顺序不动。
    pub fn apply_sort(&mut self, mode: SortMode) {
        match mode {
            SortMode::Custom => {}
            SortMode::NameAsc => self
                .sounds
                .sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
            SortMode::NameDesc => self
                .sounds
                .sort_by(|a, b| b.name.to_lowercase().cmp(&a.name.to_lowercase())),
            // 时间相同的（同一批拷进来的文件很常见）再按名字兜底，顺序才稳定。
            SortMode::TimeAsc => self.sounds.sort_by(|a, b| {
                a.modified
                    .cmp(&b.modified)
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            }),
            SortMode::TimeDesc => self.sounds.sort_by(|a, b| {
                b.modified
                    .cmp(&a.modified)
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            }),
        }
    }

    /// 当前文件夹里的音频文件名集合（用于「有没有新文件」的轻量比对）。
    pub fn file_signature(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| p.is_file() && is_audio(p))
                    .filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }
}
