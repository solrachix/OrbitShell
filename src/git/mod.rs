use git2::Repository;
use std::path::Path;

pub struct GitStatus {
    pub branch: String,
    pub files_changed: usize,
    pub added: usize,
    pub deleted: usize,
    pub modified: usize,
}

#[derive(Clone, Debug)]
pub struct GitChange {
    pub path: String,
    pub staged: bool,
    pub unstaged: bool,
    pub kind: String,
}

pub fn get_git_status(path: &Path) -> Option<GitStatus> {
    let repo = Repository::discover(path).ok()?;
    let head = repo.head().ok()?;
    let branch = head.shorthand()?.to_string();

    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true);
    let statuses = repo.statuses(Some(&mut opts)).ok()?;

    let mut added = 0usize;
    let mut deleted = 0usize;
    let mut modified = 0usize;

    for entry in statuses.iter() {
        let status = entry.status();
        if status.is_index_new() || status.is_wt_new() {
            added += 1;
        }
        if status.is_index_deleted() || status.is_wt_deleted() {
            deleted += 1;
        }
        if status.is_index_modified()
            || status.is_wt_modified()
            || status.is_index_renamed()
            || status.is_wt_renamed()
            || status.is_index_typechange()
            || status.is_wt_typechange()
        {
            modified += 1;
        }
    }

    Some(GitStatus {
        branch,
        files_changed: statuses.len(),
        added,
        deleted,
        modified,
    })
}

pub fn get_git_branches(path: &Path) -> Vec<String> {
    let repo = match Repository::discover(path) {
        Ok(repo) => repo,
        Err(_) => return Vec::new(),
    };
    let mut branches = Vec::new();
    let iter = match repo.branches(Some(git2::BranchType::Local)) {
        Ok(iter) => iter,
        Err(_) => return branches,
    };
    for branch in iter.flatten() {
        if let Ok(name) = branch.0.name() {
            if let Some(name) = name {
                branches.push(name.to_string());
            }
        }
    }
    branches.sort();
    branches
}

pub fn get_git_changes(path: &Path) -> Vec<GitChange> {
    let repo = match Repository::discover(path) {
        Ok(repo) => repo,
        Err(_) => return Vec::new(),
    };

    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true);
    let statuses = match repo.statuses(Some(&mut opts)) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let mut out = Vec::new();
    for entry in statuses.iter() {
        let status = entry.status();
        let path = entry.path().unwrap_or("").to_string();
        if path.is_empty() {
            continue;
        }

        let staged = status.is_index_new()
            || status.is_index_modified()
            || status.is_index_deleted()
            || status.is_index_renamed()
            || status.is_index_typechange();
        let unstaged = status.is_wt_new()
            || status.is_wt_modified()
            || status.is_wt_deleted()
            || status.is_wt_renamed()
            || status.is_wt_typechange();

        let kind = if status.is_index_new() || status.is_wt_new() {
            "A"
        } else if status.is_index_deleted() || status.is_wt_deleted() {
            "D"
        } else if status.is_index_modified()
            || status.is_wt_modified()
            || status.is_index_renamed()
            || status.is_wt_renamed()
            || status.is_index_typechange()
            || status.is_wt_typechange()
        {
            "M"
        } else {
            "?"
        };

        out.push(GitChange {
            path,
            staged,
            unstaged,
            kind: kind.to_string(),
        });
    }

    out
}
