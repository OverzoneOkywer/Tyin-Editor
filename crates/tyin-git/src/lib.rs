use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitStatus {
    pub branch: String,
    pub changed_files: Vec<ChangedFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub path: PathBuf,
    pub index_status: char,
    pub worktree_status: char,
}

pub fn status(root: &Path) -> Result<GitStatus> {
    let output = run_git(
        root,
        &[
            "status",
            "--porcelain=v1",
            "--branch",
            "-z",
            "--untracked-files=normal",
        ],
    )?;
    let stdout =
        String::from_utf8(output.stdout).context("git returned non-UTF-8 status output")?;
    parse_status_z(&stdout)
}

pub fn diff(root: &Path, path: &Path) -> Result<String> {
    let relative = relative_path(root, path)?;
    let mut diff = String::new();
    for args in [
        vec!["diff", "--no-ext-diff", "--", relative.as_str()],
        vec!["diff", "--cached", "--no-ext-diff", "--", relative.as_str()],
    ] {
        let output = run_git(root, &args)?;
        diff.push_str(
            &String::from_utf8(output.stdout).context("git returned non-UTF-8 diff output")?,
        );
    }
    if diff.is_empty() {
        let is_untracked = status(root)?
            .changed_files
            .iter()
            .any(|file| file.path == PathBuf::from(&relative) && file.index_status == '?');
        if is_untracked {
            let null_device = if cfg!(windows) { "NUL" } else { "/dev/null" };
            let output = Command::new("git")
                .arg("-C")
                .arg(root)
                .args(["diff", "--no-index", "--no-ext-diff", "--", null_device])
                .arg(&relative)
                .output()
                .with_context(|| format!("failed to create diff in {}", root.display()))?;
            if !output.status.success() && output.status.code() != Some(1) {
                bail!(
                    "git diff failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            diff =
                String::from_utf8(output.stdout).context("git returned non-UTF-8 diff output")?;
        }
        if diff.is_empty() {
            diff.push_str("(No unstaged or staged diff for this file.)\n");
        }
    }
    Ok(diff)
}

pub fn staged_diff(root: &Path) -> Result<String> {
    let output = run_git(root, &["diff", "--cached", "--no-ext-diff"])?;
    let diff = String::from_utf8(output.stdout).context("git returned non-UTF-8 diff output")?;
    Ok(if diff.is_empty() {
        "(No staged changes.)\n".to_string()
    } else {
        diff
    })
}

pub fn stage(root: &Path, path: &Path) -> Result<()> {
    let relative = relative_path(root, path)?;
    run_git(root, &["add", "--", relative.as_str()])?;
    Ok(())
}

pub fn unstage(root: &Path, path: &Path) -> Result<()> {
    let relative = relative_path(root, path)?;
    let status = status(root)?;
    let is_new_file = status
        .changed_files
        .iter()
        .any(|file| file.path == PathBuf::from(&relative) && file.index_status == 'A');
    if is_new_file {
        run_git(root, &["rm", "--cached", "-q", "--", relative.as_str()])?;
    } else {
        run_git(root, &["reset", "-q", "HEAD", "--", relative.as_str()])?;
    }
    Ok(())
}

pub fn commit(root: &Path, message: &str) -> Result<()> {
    let message = message.trim();
    if message.is_empty() {
        bail!("commit message cannot be empty");
    }
    if staged_diff(root)?.starts_with("(No staged changes.)") {
        bail!("there are no staged changes to commit");
    }
    run_git(
        root,
        &["-c", "core.quotepath=false", "commit", "-m", message],
    )?;
    Ok(())
}

fn relative_path<'a>(root: &Path, path: &'a Path) -> Result<String> {
    let root = root
        .canonicalize()
        .with_context(|| format!("cannot resolve Git workspace {}", root.display()))?;
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let resolved = match absolute.canonicalize() {
        Ok(path) => path,
        Err(_) => {
            let parent = absolute
                .parent()
                .context("Git path does not have a parent directory")?
                .canonicalize()
                .with_context(|| format!("cannot resolve Git path {}", absolute.display()))?;
            parent.join(
                absolute
                    .file_name()
                    .context("Git path does not have a file name")?,
            )
        }
    };
    let relative = resolved
        .strip_prefix(&root)
        .with_context(|| format!("path is outside Git workspace: {}", path.display()))?;
    relative
        .to_str()
        .map(str::to_owned)
        .context("Git path is not valid UTF-8")
}

fn run_git(root: &Path, args: &[&str]) -> Result<std::process::Output> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .with_context(|| format!("failed to run git in {}", root.display()))?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.first().copied().unwrap_or("command"),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output)
}

#[cfg(test)]
fn parse_status(output: &str) -> Result<GitStatus> {
    let mut branch = "unknown".to_string();
    let mut changed_files = Vec::new();

    for line in output.lines() {
        if let Some(header) = line.strip_prefix("## ") {
            branch = header
                .split("...")
                .next()
                .unwrap_or(header)
                .split(" [")
                .next()
                .unwrap_or(header)
                .to_string();
            continue;
        }
        if line.len() < 4 {
            continue;
        }

        let status = &line[..2];
        let path = line[3..].split(" -> ").last().unwrap_or(&line[3..]);
        changed_files.push(ChangedFile {
            path: PathBuf::from(path),
            index_status: status.chars().next().unwrap_or(' '),
            worktree_status: status.chars().nth(1).unwrap_or(' '),
        });
    }

    Ok(GitStatus {
        branch,
        changed_files,
    })
}

fn parse_status_z(output: &str) -> Result<GitStatus> {
    let mut branch = "unknown".to_string();
    let mut changed_files = Vec::new();
    let mut records = output.split('\0').peekable();
    while let Some(record) = records.next() {
        if let Some(header) = record.strip_prefix("## ") {
            branch = header
                .split("...")
                .next()
                .unwrap_or(header)
                .split(" [")
                .next()
                .unwrap_or(header)
                .to_string();
            continue;
        }
        if record.len() < 4 {
            continue;
        }
        let status = &record[..2];
        let path = &record[3..];
        changed_files.push(ChangedFile {
            path: PathBuf::from(path),
            index_status: status.chars().next().unwrap_or(' '),
            worktree_status: status.chars().nth(1).unwrap_or(' '),
        });
        if status.contains('R') || status.contains('C') {
            records.next();
        }
    }

    Ok(GitStatus {
        branch,
        changed_files,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        commit, diff, parse_status, parse_status_z, stage, staged_diff, status, unstage,
        ChangedFile,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn repository() -> PathBuf {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tyin-git-test-{}-{id}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        assert!(Command::new("git")
            .args(["-C", root.to_str().unwrap(), "init", "-q"])
            .status()
            .unwrap()
            .success());
        root
    }

    fn git(root: &std::path::Path, args: &[&str]) {
        assert!(Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .unwrap()
            .success());
    }

    #[test]
    fn parses_branch_ahead_state_and_changed_paths() {
        let status =
            parse_status("## main...origin/main [ahead 1]\n M src/main.rs\n?? new file.txt\n")
                .unwrap();
        assert_eq!(status.branch, "main");
        assert_eq!(
            status.changed_files,
            vec![
                ChangedFile {
                    path: PathBuf::from("src/main.rs"),
                    index_status: ' ',
                    worktree_status: 'M',
                },
                ChangedFile {
                    path: PathBuf::from("new file.txt"),
                    index_status: '?',
                    worktree_status: '?',
                },
            ]
        );
    }

    #[test]
    fn parses_detached_head() {
        let status = parse_status("## HEAD (no branch)\n").unwrap();
        assert_eq!(status.branch, "HEAD (no branch)");
        assert!(status.changed_files.is_empty());
    }

    #[test]
    fn parses_null_delimited_paths_without_losing_spaces() {
        let status = parse_status_z("## main\0?? new file.txt\0").unwrap();
        assert_eq!(status.branch, "main");
        assert_eq!(status.changed_files[0].path, PathBuf::from("new file.txt"));
    }

    #[test]
    fn stages_reviews_unstages_and_commits_files_safely() {
        let root = repository();
        let file = root.join("new file.txt");
        fs::write(&file, "review me\n").unwrap();

        assert_eq!(
            status(&root).unwrap().changed_files[0].path,
            PathBuf::from("new file.txt")
        );
        let preview = diff(&root, &file).unwrap();
        assert!(preview.contains("+review me"), "{preview:?}");
        stage(&root, &file).unwrap();
        assert!(staged_diff(&root).unwrap().contains("+review me"));
        unstage(&root, &file).unwrap();
        assert_eq!(status(&root).unwrap().changed_files[0].index_status, '?');
        stage(&root, &file).unwrap();
        git(&root, &["config", "user.name", "Tyin Test"]);
        git(
            &root,
            &["config", "user.email", "tyin-test@example.invalid"],
        );
        commit(&root, "Add reviewed file").unwrap();
        assert!(status(&root).unwrap().changed_files.is_empty());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_empty_commit_messages_and_paths_outside_repository() {
        let root = repository();
        assert!(commit(&root, "  ").is_err());
        assert!(stage(&root, &root.parent().unwrap().join("outside")).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
