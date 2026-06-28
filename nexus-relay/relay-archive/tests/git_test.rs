use relay_archive::git::{GitRepo, git_add_all, git_commit, git_pull};
use std::process::Command;
use tempfile::TempDir;

/// Initialize a temporary git repo with user config so commits work.
fn init_temp_repo() -> (TempDir, GitRepo) {
    let dir = TempDir::new().expect("failed to create temp dir");
    let path = dir.path().to_str().unwrap().to_string();

    Command::new("git")
        .args(["init"])
        .current_dir(&path)
        .output()
        .expect("git init failed");

    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(&path)
        .output()
        .expect("git config email failed");

    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(&path)
        .output()
        .expect("git config name failed");

    let repo = GitRepo { path: path.clone() };
    (dir, repo)
}

#[tokio::test]
async fn git_commit_empty_repo_no_changes_returns_false() {
    let (_dir, repo) = init_temp_repo();

    let result = git_commit(&repo, "empty commit attempt").await;
    assert!(result.is_ok(), "expected Ok, got {:?}", result);
    assert!(!result.unwrap(), "expected false — nothing to commit");
}

#[tokio::test]
async fn git_commit_with_staged_changes_returns_true() {
    let (dir, repo) = init_temp_repo();

    // Write a file and stage it
    let file_path = dir.path().join("test.txt");
    std::fs::write(&file_path, "hello").expect("failed to write file");

    Command::new("git")
        .args(["add", "test.txt"])
        .current_dir(dir.path())
        .output()
        .expect("git add failed");

    let result = git_commit(&repo, "add test file").await;
    assert!(result.is_ok(), "expected Ok, got {:?}", result);
    assert!(result.unwrap(), "expected true — commit should succeed");
}

#[tokio::test]
async fn git_pull_no_remote_succeeds() {
    let (_dir, repo) = init_temp_repo();

    // git pull on a repo with no remote configured should succeed gracefully
    let result = git_pull(&repo).await;
    assert!(result.is_ok(), "expected Ok, got {:?}", result);
}

#[tokio::test]
async fn git_add_all_stages_files() {
    let (dir, repo) = init_temp_repo();

    // Create a couple of files
    std::fs::write(dir.path().join("a.txt"), "aaa").expect("write a.txt");
    std::fs::write(dir.path().join("b.txt"), "bbb").expect("write b.txt");

    git_add_all(&repo).await.expect("git_add_all failed");

    // Verify files are staged by checking `git diff --cached --name-only`
    let output = Command::new("git")
        .args(["diff", "--cached", "--name-only"])
        .current_dir(dir.path())
        .output()
        .expect("git diff --cached failed");

    let staged = String::from_utf8_lossy(&output.stdout);
    assert!(
        staged.contains("a.txt"),
        "a.txt should be staged, got: {staged}"
    );
    assert!(
        staged.contains("b.txt"),
        "b.txt should be staged, got: {staged}"
    );
}
