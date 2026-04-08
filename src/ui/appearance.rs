use anyhow::Result;
use lucide_icons::Icon;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::acp::storage;

const APPEARANCE_FILE: &str = "appearance.json";
const DEFAULT_ICON_THEME_ID: &str = "orbitshell-dark";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppearanceSettings {
    pub icon_theme: String,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            icon_theme: DEFAULT_ICON_THEME_ID.to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IconThemeOption {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub author: &'static str,
    pub accent: u32,
    pub folder_color: u32,
    pub file_color: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct IconThemeVisuals {
    pub folder_closed: Icon,
    pub folder_open: Icon,
    pub file: Icon,
    pub folder_color: u32,
    pub file_color: u32,
}

#[derive(Clone, Copy, Debug)]
struct ThemePalette {
    folder_default: u32,
    file_default: u32,
    code: u32,
    docs: u32,
    config: u32,
    git: u32,
    image: u32,
    archive: u32,
    test: u32,
    package: u32,
}

const ICON_THEME_OPTIONS: [IconThemeOption; 4] = [
    IconThemeOption {
        id: "orbitshell-dark",
        name: "OrbitShell Dark",
        description: "Default OrbitShell icons with subdued contrast.",
        author: "OrbitShell",
        accent: 0x6b9eff,
        folder_color: 0xe0a458,
        file_color: 0x8f9bb3,
    },
    IconThemeOption {
        id: "catppuccin-mocha",
        name: "Catppuccin Mocha",
        description: "Soft pastel folders and files inspired by Catppuccin.",
        author: "Community preset",
        accent: 0x89b4fa,
        folder_color: 0xf9e2af,
        file_color: 0xcdd6f4,
    },
    IconThemeOption {
        id: "material-ocean",
        name: "Material Ocean",
        description: "Material-style glyph colors with stronger category contrast.",
        author: "Community preset",
        accent: 0x80cbc4,
        folder_color: 0xffcb6b,
        file_color: 0x82aaff,
    },
    IconThemeOption {
        id: "mono-graphite",
        name: "Mono Graphite",
        description: "Minimal monochrome icons for a flatter visual language.",
        author: "OrbitShell Labs",
        accent: 0xb0b0b0,
        folder_color: 0xb0b0b0,
        file_color: 0x8a8a8a,
    },
];

impl AppearanceSettings {
    pub fn load() -> Self {
        Self::load_result().unwrap_or_default()
    }

    pub fn load_result() -> Result<Self> {
        let Some(path) = Self::settings_path() else {
            return Ok(Self::default());
        };
        Ok(storage::load_optional_json_file(&path)?.unwrap_or_default())
    }

    pub fn save(&self) -> Result<()> {
        let Some(path) = Self::settings_path() else {
            return Ok(());
        };
        storage::save_json_file(&path, self)
    }

    pub fn selected_icon_theme(&self) -> IconThemeOption {
        icon_theme_by_id(&self.icon_theme).unwrap_or(ICON_THEME_OPTIONS[0])
    }

    fn settings_path() -> Option<PathBuf> {
        storage::app_root()
            .ok()
            .map(|root| root.join(APPEARANCE_FILE))
    }
}

pub fn icon_theme_options() -> &'static [IconThemeOption] {
    &ICON_THEME_OPTIONS
}

pub fn icon_theme_by_id(id: &str) -> Option<IconThemeOption> {
    ICON_THEME_OPTIONS
        .iter()
        .copied()
        .find(|theme| theme.id == id)
}

pub fn icon_theme_visuals(id: &str) -> IconThemeVisuals {
    match id {
        "catppuccin-mocha" => IconThemeVisuals {
            folder_closed: Icon::Folder,
            folder_open: Icon::FolderOpen,
            file: Icon::File,
            folder_color: 0xf9e2af,
            file_color: 0xcdd6f4,
        },
        "material-ocean" => IconThemeVisuals {
            folder_closed: Icon::Folder,
            folder_open: Icon::FolderOpen,
            file: Icon::FileText,
            folder_color: 0xffcb6b,
            file_color: 0x82aaff,
        },
        "mono-graphite" => IconThemeVisuals {
            folder_closed: Icon::Folder,
            folder_open: Icon::FolderOpen,
            file: Icon::FileText,
            folder_color: 0xb0b0b0,
            file_color: 0x8a8a8a,
        },
        _ => IconThemeVisuals {
            folder_closed: Icon::Folder,
            folder_open: Icon::FolderOpen,
            file: Icon::File,
            folder_color: 0xe0a458,
            file_color: 0x8f9bb3,
        },
    }
}

fn theme_palette(id: &str) -> ThemePalette {
    match id {
        "catppuccin-mocha" => ThemePalette {
            folder_default: 0xf9e2af,
            file_default: 0xcdd6f4,
            code: 0x89b4fa,
            docs: 0x94e2d5,
            config: 0xf5c2e7,
            git: 0xf38ba8,
            image: 0xf9e2af,
            archive: 0xfab387,
            test: 0xa6e3a1,
            package: 0xcba6f7,
        },
        "material-ocean" => ThemePalette {
            folder_default: 0xffcb6b,
            file_default: 0x82aaff,
            code: 0x89ddff,
            docs: 0xc3e88d,
            config: 0xc792ea,
            git: 0xf78c6c,
            image: 0xffcb6b,
            archive: 0xf78c6c,
            test: 0xc3e88d,
            package: 0x82aaff,
        },
        "mono-graphite" => ThemePalette {
            folder_default: 0xb0b0b0,
            file_default: 0x8a8a8a,
            code: 0xd0d0d0,
            docs: 0xbfbfbf,
            config: 0x9f9f9f,
            git: 0xc9c9c9,
            image: 0xaeaeae,
            archive: 0x989898,
            test: 0xd7d7d7,
            package: 0xb5b5b5,
        },
        _ => ThemePalette {
            folder_default: 0xe0a458,
            file_default: 0x8f9bb3,
            code: 0x4ea1ff,
            docs: 0x66c2a5,
            config: 0xf0b44c,
            git: 0xf97316,
            image: 0x38bdf8,
            archive: 0x94a3b8,
            test: 0x7bd88f,
            package: 0xa78bfa,
        },
    }
}

pub fn resolve_themed_icon(
    theme_id: &str,
    path: &Path,
    is_dir: bool,
    is_open: bool,
) -> (Icon, u32) {
    if is_dir {
        return resolve_themed_folder_icon(theme_id, path, is_open);
    }
    resolve_themed_file_icon(theme_id, path)
}

fn resolve_themed_folder_icon(theme_id: &str, path: &Path, is_open: bool) -> (Icon, u32) {
    let palette = theme_palette(theme_id);
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    let closed = match name.as_str() {
        "src" | "infra" => Icon::FolderCode,
        "tests" | "__tests__" => Icon::FolderSearch,
        "docs" => Icon::FolderOpenDot,
        ".gitlab" | ".github" | ".husky" => Icon::FolderGit2,
        "packages" | "node_modules" => Icon::FolderTree,
        "target" | "dist" | "build" | "installer" => Icon::FolderArchive,
        _ => Icon::Folder,
    };

    let open = match closed {
        Icon::FolderGit2 => Icon::FolderOpenDot,
        Icon::FolderCode => Icon::FolderOpen,
        Icon::FolderSearch => Icon::FolderOpenDot,
        Icon::FolderArchive => Icon::FolderOpen,
        _ => Icon::FolderOpen,
    };

    let color = match name.as_str() {
        "src" | "infra" => palette.code,
        "tests" | "__tests__" => palette.test,
        "docs" => palette.docs,
        ".gitlab" | ".github" | ".husky" => palette.git,
        "packages" | "node_modules" => palette.package,
        "target" | "dist" | "build" | "installer" => palette.archive,
        _ => palette.folder_default,
    };

    (if is_open { open } else { closed }, color)
}

fn resolve_themed_file_icon(theme_id: &str, path: &Path) -> (Icon, u32) {
    let palette = theme_palette(theme_id);
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let ext = path
        .extension()
        .map(|ext| ext.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    match name.as_str() {
        "cargo.toml" | "cargo.lock" | "build.rs" => return (Icon::FileCode, palette.code),
        "readme.md" | "plan.md" => return (Icon::BookText, palette.docs),
        ".gitignore" | ".gitattributes" => return (Icon::GitBranch, palette.git),
        ".gitlab-ci.yml" => return (Icon::Gitlab, palette.git),
        "package.json" | "pnpm-lock.yaml" | "yarn.lock" => return (Icon::Package, palette.package),
        "agents.json" | "repo.config.json" | "registry-sample.json" | "orbitshell_rules.json" => {
            return (Icon::FileCog, palette.config);
        }
        _ => {}
    }

    match ext.as_str() {
        "rs" | "js" | "ts" | "tsx" | "jsx" | "py" | "go" | "java" | "c" | "cpp" | "h" | "hpp" => {
            (Icon::FileCode, palette.code)
        }
        "md" | "txt" | "rst" => (Icon::FileText, palette.docs),
        "json" | "toml" | "yaml" | "yml" | "ini" | "lock" => (Icon::FileCog, palette.config),
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "ico" => (Icon::FileImage, palette.image),
        "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" => (Icon::FileArchive, palette.archive),
        "csv" | "tsv" => (Icon::FileSpreadsheet, palette.docs),
        "sh" | "bash" | "zsh" | "fish" => (Icon::FileTerminal, palette.code),
        _ => (icon_theme_visuals(theme_id).file, palette.file_default),
    }
}

#[cfg(test)]
mod tests {
    use super::{AppearanceSettings, DEFAULT_ICON_THEME_ID, icon_theme_by_id};

    #[test]
    fn defaults_to_builtin_icon_theme() {
        let settings = AppearanceSettings::default();
        assert_eq!(settings.icon_theme, DEFAULT_ICON_THEME_ID);
        assert_eq!(settings.selected_icon_theme().id, DEFAULT_ICON_THEME_ID);
    }

    #[test]
    fn falls_back_when_theme_id_is_unknown() {
        let settings = AppearanceSettings {
            icon_theme: "unknown-theme".into(),
        };
        assert_eq!(settings.selected_icon_theme().id, DEFAULT_ICON_THEME_ID);
        assert!(icon_theme_by_id("unknown-theme").is_none());
    }
}
