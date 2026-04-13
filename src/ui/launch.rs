use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LaunchOptions {
    pub base_terminal_cwd: Option<PathBuf>,
}

impl LaunchOptions {
    pub fn from_args(args: impl IntoIterator<Item = OsString>) -> Self {
        let base_terminal_cwd = args
            .into_iter()
            .skip(1)
            .map(PathBuf::from)
            .find(|path| path.is_dir());

        Self { base_terminal_cwd }
    }
}

pub fn default_base_terminal_cwd() -> PathBuf {
    resolve_default_base_terminal_cwd(
        std::env::var_os("USERPROFILE"),
        std::env::var_os("HOME"),
        cfg!(windows),
    )
}

pub fn project_dir_name_from_prompt(prompt: &str) -> Option<String> {
    safe_dir_name(prompt)
}

pub fn clone_dir_name_from_url(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return None;
    }

    let repo = trimmed
        .rsplit(['/', '\\', ':'])
        .find(|segment| !segment.trim().is_empty())?
        .trim()
        .strip_suffix(".git")
        .unwrap_or_else(|| {
            trimmed
                .rsplit(['/', '\\', ':'])
                .find(|segment| !segment.trim().is_empty())
                .unwrap_or(trimmed)
                .trim()
        });

    safe_dir_name(repo)
}

fn resolve_default_base_terminal_cwd(
    user_profile: Option<OsString>,
    home: Option<OsString>,
    windows: bool,
) -> PathBuf {
    user_profile
        .filter(|path| !path.is_empty())
        .or_else(|| home.filter(|path| !path.is_empty()))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if windows {
                PathBuf::from(r"C:\")
            } else {
                PathBuf::from("/")
            }
        })
}

fn safe_dir_name(input: &str) -> Option<String> {
    let mut slug = String::new();
    let mut last_was_sep = false;

    for ch in input.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_sep = false;
        } else if !last_was_sep && !slug.is_empty() {
            slug.push('-');
            last_was_sep = true;
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }

    (!slug.is_empty()).then_some(slug)
}

#[cfg(test)]
mod tests {
    use super::resolve_default_base_terminal_cwd;
    use std::ffi::OsString;
    use std::path::PathBuf;

    #[test]
    fn default_base_terminal_cwd_prefers_user_profile_on_windows() {
        let cwd = resolve_default_base_terminal_cwd(
            Some(OsString::from(r"C:\Users\Carlos")),
            Some(OsString::from("/home/carlos")),
            true,
        );

        assert_eq!(cwd, PathBuf::from(r"C:\Users\Carlos"));
    }

    #[test]
    fn default_base_terminal_cwd_uses_home_when_user_profile_is_missing() {
        let cwd =
            resolve_default_base_terminal_cwd(None, Some(OsString::from("/home/carlos")), false);

        assert_eq!(cwd, PathBuf::from("/home/carlos"));
    }

    #[test]
    fn project_dir_name_from_prompt_builds_safe_slug() {
        assert_eq!(
            super::project_dir_name_from_prompt("Build a Minesweeper clone in React"),
            Some("build-a-minesweeper-clone-in-react".to_string())
        );
        assert_eq!(super::project_dir_name_from_prompt("   "), None);
    }

    #[test]
    fn clone_dir_name_from_url_uses_repository_name() {
        assert_eq!(
            super::clone_dir_name_from_url("git@github.com:owner/orbitshell.git"),
            Some("orbitshell".to_string())
        );
        assert_eq!(
            super::clone_dir_name_from_url("https://github.com/owner/my-app/"),
            Some("my-app".to_string())
        );
    }
}
