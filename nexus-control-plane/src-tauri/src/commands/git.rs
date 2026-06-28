use nexus_core::db::DbState;
use nexus_core::github::{BranchSummary, DispatchPr, GhAuthStatus, GithubService, OrgPriority, RepoSummary};
use nexus_core::services::settings as settings_svc;
use nexus_core::services::git as git_svc;
use std::path::PathBuf;
use std::sync::Arc;

#[tauri::command]
pub async fn check_gh_auth(gh_state: tauri::State<'_, Arc<GithubService>>) -> Result<GhAuthStatus, String> {
    let gh = gh_state.inner().clone();
    Ok(tokio::task::spawn_blocking(move || gh.check_auth())
        .await
        .map_err(|e| e.to_string())?)
}

#[tauri::command]
pub async fn list_org_repos(
    org: String,
    gh_state: tauri::State<'_, Arc<GithubService>>,
) -> Result<Vec<RepoSummary>, String> {
    let gh = gh_state.inner().clone();
    tokio::task::spawn_blocking(move || gh.list_org_repos(&org))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn list_branches(
    full_name: String,
    gh_state: tauri::State<'_, Arc<GithubService>>,
) -> Result<Vec<BranchSummary>, String> {
    gh_state.list_branches(&full_name)
}

#[tauri::command]
pub fn clone_repo(
    full_name: String,
    target_path: String,
    branch: Option<String>,
    gh_state: tauri::State<'_, Arc<GithubService>>,
) -> Result<(), String> {
    gh_state.clone_repo(
        &full_name,
        &PathBuf::from(&target_path),
        branch.as_deref(),
    )
}

#[tauri::command]
pub async fn compute_workspace_path(
    card_name: String,
    db_state: tauri::State<'_, DbState>,
) -> Result<String, String> {
    let db = db_state.inner().clone();
    tokio::task::spawn_blocking(move || git_svc::compute_workspace_path(&db, &card_name))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn list_dispatch_prs(
    db_state: tauri::State<'_, DbState>,
    gh_state: tauri::State<'_, Arc<GithubService>>,
) -> Result<Vec<DispatchPr>, String> {
    let gh = gh_state.inner().clone();
    let db = db_state.inner().clone();

    tokio::task::spawn_blocking(move || {
        // Read dispatch repo + base branch from settings, with defaults
        let repo = settings_svc::get_setting(&db, "dispatch_repo")
            .ok()
            .flatten()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "SkyNexus-AI/dispatch".to_string());
        let base = settings_svc::get_setting(&db, "dispatch_base_branch")
            .ok()
            .flatten()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "sdowney".to_string());

        gh.list_dispatch_prs(&repo, &base)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn list_dispatch_prs_sent(
    db_state: tauri::State<'_, DbState>,
    gh_state: tauri::State<'_, Arc<GithubService>>,
) -> Result<Vec<DispatchPr>, String> {
    let gh = gh_state.inner().clone();
    let db = db_state.inner().clone();

    tokio::task::spawn_blocking(move || {
        let repo = settings_svc::get_setting(&db, "dispatch_repo")
            .ok()
            .flatten()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "SkyNexus-AI/dispatch".to_string());

        gh.list_dispatch_prs_sent(&repo)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn fetch_org_priorities(
    gh_state: tauri::State<'_, Arc<GithubService>>,
) -> Result<Vec<OrgPriority>, String> {
    let gh = gh_state.inner().clone();
    tokio::task::spawn_blocking(move || {
        gh.fetch_org_priorities("SkyNexus-AI/priorities")
    })
    .await
    .map_err(|e| e.to_string())?
}
