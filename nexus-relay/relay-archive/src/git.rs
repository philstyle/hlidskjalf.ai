use thiserror::Error;
use tokio::process::Command;

#[derive(Debug, Error)]
pub enum ArchiveError {
    #[error("git error: {0}")]
    Git(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("db error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("serialize error: {0}")]
    Serialize(String),
}

#[derive(Clone)]
pub struct GitRepo {
    pub path: String,
}

async fn run_git(repo: &GitRepo, args: &[&str]) -> Result<String, ArchiveError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(&repo.path)
        .output()
        .await?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        Err(ArchiveError::Git(stderr))
    }
}

pub async fn git_pull(repo: &GitRepo) -> Result<(), ArchiveError> {
    // On empty repos or repos with no upstream, pull will fail — that's OK.
    // We only need pull to succeed when there's an existing remote branch.
    let result = run_git(repo, &["pull", "--rebase"]).await;
    match result {
        Ok(_) => Ok(()),
        Err(ArchiveError::Git(ref msg))
            if msg.contains("no such ref")
                || msg.contains("no tracking information")
                || msg.contains("Couldn't find remote ref")
                || msg.contains("There is no tracking information") =>
        {
            tracing::debug!("git pull skipped (no remote branch yet): {}", msg.trim());
            Ok(())
        }
        Err(e) => Err(e),
    }
}

pub async fn git_add_all(repo: &GitRepo) -> Result<(), ArchiveError> {
    run_git(repo, &["add", "-A"]).await?;
    Ok(())
}

/// Returns Ok(true) if a commit was made, Ok(false) if nothing to commit.
pub async fn git_commit(repo: &GitRepo, message: &str) -> Result<bool, ArchiveError> {
    let output = Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(&repo.path)
        .output()
        .await?;

    if output.status.success() {
        return Ok(true);
    }

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let combined = format!("{}{}", stdout, stderr);

    if combined.contains("nothing to commit") || combined.contains("nothing added to commit") {
        return Ok(false);
    }

    Err(ArchiveError::Git(stderr))
}

pub async fn git_push(repo: &GitRepo) -> Result<(), ArchiveError> {
    // Try normal push first; if it fails because no upstream is set, push with -u
    let result = run_git(repo, &["push"]).await;
    match result {
        Ok(_) => Ok(()),
        Err(ArchiveError::Git(ref msg))
            if msg.contains("no upstream branch")
                || msg.contains("has no upstream branch")
                || msg.contains("The current branch") =>
        {
            tracing::info!("setting upstream branch on first push");
            run_git(repo, &["push", "-u", "origin", "main"]).await?;
            Ok(())
        }
        Err(e) => Err(e),
    }
}

pub async fn git_add_file(repo: &GitRepo, path: &str) -> Result<(), ArchiveError> {
    run_git(repo, &["add", path]).await?;
    Ok(())
}
