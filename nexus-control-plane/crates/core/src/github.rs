use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    let clean: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    let mut out = Vec::new();
    let table = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let lookup = |c: u8| -> Result<u8, String> {
        table.iter().position(|&b| b == c).map(|p| p as u8).ok_or_else(|| format!("invalid base64 char: {}", c as char))
    };
    let bytes = clean.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'=' { break; }
        let a = lookup(bytes[i])?;
        let b = if i + 1 < bytes.len() && bytes[i + 1] != b'=' { lookup(bytes[i + 1])? } else { 0 };
        let c = if i + 2 < bytes.len() && bytes[i + 2] != b'=' { lookup(bytes[i + 2])? } else { 0 };
        let d = if i + 3 < bytes.len() && bytes[i + 3] != b'=' { lookup(bytes[i + 3])? } else { 0 };
        out.push((a << 2) | (b >> 4));
        if i + 2 < bytes.len() && bytes[i + 2] != b'=' { out.push((b << 4) | (c >> 2)); }
        if i + 3 < bytes.len() && bytes[i + 3] != b'=' { out.push((c << 6) | d); }
        i += 4;
    }
    Ok(out)
}

fn map_to_priority(map: &std::collections::HashMap<String, String>) -> Option<OrgPriority> {
    let id = map.get("id")?.clone();
    let title = map.get("title").cloned().unwrap_or_default();
    Some(OrgPriority {
        id,
        title,
        description: map.get("description").cloned().unwrap_or_default(),
        status: map.get("status").cloned().unwrap_or_else(|| "active".to_string()),
        level: map.get("level").cloned().unwrap_or_else(|| "org".to_string()),
        owner: map.get("owner").cloned().unwrap_or_default(),
        ventures: map.get("ventures").map(|v| v.split(',').map(|s| s.trim().to_string()).collect()).unwrap_or_default(),
        repos: map.get("repos").map(|v| v.split(',').map(|s| s.trim().to_string()).collect()).unwrap_or_default(),
        timeframe: map.get("timeframe").cloned().unwrap_or_default(),
        tags: map.get("tags").map(|v| v.split(',').map(|s| s.trim().to_string()).collect()).unwrap_or_default(),
    })
}

#[derive(Serialize, Clone)]
pub struct GhAuthStatus {
    pub authenticated: bool,
    pub username: Option<String>,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct RepoSummary {
    pub name: String,
    pub full_name: String,
    pub description: Option<String>,
    pub default_branch: String,
    pub updated_at: String,
    #[serde(rename = "private")]
    pub is_private: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct BranchSummary {
    pub name: String,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct DispatchPr {
    pub number: u32,
    pub title: String,
    pub state: String,
    pub author: DispatchPrAuthor,
    #[serde(default)]
    pub assignees: Vec<DispatchPrAssignee>,
    pub labels: Vec<DispatchPrLabel>,
    #[serde(rename = "createdAt", default)]
    pub created_at: Option<String>,
    #[serde(rename = "updatedAt", default)]
    pub updated_at: Option<String>,
    #[serde(rename = "headRefName", default)]
    pub head_ref_name: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub url: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct DispatchPrAuthor {
    pub login: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct DispatchPrAssignee {
    pub login: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct DispatchPrLabel {
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OrgPriority {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub status: String,
    pub level: String,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub ventures: Vec<String>,
    #[serde(default)]
    pub repos: Vec<String>,
    #[serde(default)]
    pub timeframe: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

pub struct GithubService {
    gh_path: Option<String>,
}

fn gh_install_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "GitHub CLI (gh) not found. Install with: brew install gh"
    } else if cfg!(target_os = "windows") {
        "GitHub CLI (gh) not found. Install with: winget install GitHub.cli"
    } else {
        "GitHub CLI (gh) not found. Install with: sudo apt install gh (or see https://github.com/cli/cli#installation)"
    }
}

impl GithubService {
    /// Resolve `gh` binary path at startup.
    ///
    /// 1. Try bare `gh --version`
    /// 2. Unix: `$SHELL -l -c 'which gh'`
    /// 3. Windows: `where.exe gh`
    /// 4. If all fail, store None
    pub fn new() -> Self {
        // Try bare gh first (all platforms)
        if Command::new("gh")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Self {
                gh_path: Some("gh".to_string()),
            };
        }

        // Unix: resolve via login shell
        #[cfg(not(target_os = "windows"))]
        if let Ok(shell) = std::env::var("SHELL") {
            if let Ok(output) = Command::new(&shell)
                .args(["-l", "-c", "which gh"])
                .output()
            {
                if output.status.success() {
                    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !path.is_empty() {
                        return Self {
                            gh_path: Some(path),
                        };
                    }
                }
            }
        }

        // Windows: try where.exe
        #[cfg(target_os = "windows")]
        if let Ok(output) = Command::new("where").arg("gh").output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !path.is_empty() {
                    return Self {
                        gh_path: Some(path),
                    };
                }
            }
        }

        Self { gh_path: None }
    }

    pub fn gh_path_clone(&self) -> Option<String> {
        self.gh_path.clone()
    }

    fn gh(&self) -> Result<Command, String> {
        match &self.gh_path {
            Some(path) => Ok(Command::new(path)),
            None => Err(gh_install_hint().to_string()),
        }
    }

    pub fn check_auth(&self) -> GhAuthStatus {
        let mut cmd = match self.gh() {
            Ok(cmd) => cmd,
            Err(e) => {
                return GhAuthStatus {
                    authenticated: false,
                    username: None,
                    error: Some(e),
                }
            }
        };

        let output = match cmd.args(["auth", "status"]).output() {
            Ok(o) => o,
            Err(e) => {
                return GhAuthStatus {
                    authenticated: false,
                    username: None,
                    error: Some(format!("Failed to run gh: {}", e)),
                }
            }
        };

        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        if output.status.success() {
            // Parse username from "Logged in to github.com account USERNAME"
            let username = combined
                .lines()
                .find_map(|line| {
                    if let Some(idx) = line.find("account ") {
                        let rest = &line[idx + 8..];
                        let name = rest.split_whitespace().next().unwrap_or("").to_string();
                        if name.is_empty() {
                            None
                        } else {
                            Some(name)
                        }
                    } else {
                        None
                    }
                });

            GhAuthStatus {
                authenticated: true,
                username,
                error: None,
            }
        } else {
            GhAuthStatus {
                authenticated: false,
                username: None,
                error: Some("Not authenticated. Run: gh auth login".to_string()),
            }
        }
    }

    /// List repositories for an owner — works for BOTH organizations and
    /// personal accounts (and includes private repos the authenticated user can
    /// access). Uses `gh repo list <owner>` rather than the `/orgs/{org}/repos`
    /// REST endpoint, which 404s for a username (a personal account isn't an
    /// org) and never returns a user's private repos. One path for "point it at
    /// a personal account or an org and have it work." `org` is the owner login
    /// (org name or username).
    pub fn list_org_repos(&self, org: &str) -> Result<Vec<RepoSummary>, String> {
        let mut cmd = self.gh()?;

        let output = cmd
            .args([
                "repo",
                "list",
                org,
                // gh's repo-list default cap is 30; raise it for power users
                // with many repos. 1000 is gh's documented per-call maximum.
                "--limit",
                "1000",
                "--json",
                "name,nameWithOwner,description,defaultBranchRef,updatedAt,isPrivate",
                // Reshape gh's field names into the RepoSummary NDJSON shape the
                // parser below already expects. defaultBranchRef is null for an
                // empty repo, so fall back to "".
                "--jq",
                ".[] | {name, full_name: .nameWithOwner, description, default_branch: (.defaultBranchRef.name // \"\"), updated_at: .updatedAt, private: .isPrivate}",
            ])
            .output()
            .map_err(|e| format!("Failed to run gh: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(format!(
                "Failed to load repos for '{}'. Check that your GitHub login (gh auth status) can access it. ({})",
                org, stderr
            ));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let repos: Vec<RepoSummary> = stdout
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| serde_json::from_str(l))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to parse repo list: {}", e))?;

        Ok(repos)
    }

    pub fn list_branches(&self, full_name: &str) -> Result<Vec<BranchSummary>, String> {
        let mut cmd = self.gh()?;

        let output = cmd
            .args([
                "api",
                &format!("/repos/{}/branches", full_name),
                "--paginate",
                "--jq",
                ".[] | {name}",
            ])
            .output()
            .map_err(|e| format!("Failed to run gh: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(format!("Failed to list branches: {}", stderr));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let branches: Vec<BranchSummary> = stdout
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| serde_json::from_str(l))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to parse branch list: {}", e))?;

        Ok(branches)
    }

    pub fn clone_repo(&self, full_name: &str, target_path: &Path, branch: Option<&str>) -> Result<(), String> {
        let mut cmd = self.gh()?;

        cmd.args([
            "repo",
            "clone",
            full_name,
            target_path.to_str().unwrap_or_default(),
        ]);

        if let Some(branch) = branch {
            cmd.args(["--", "--branch", branch]);
        }

        let output = cmd
            .output()
            .map_err(|e| format!("Failed to run gh: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(format!("Clone failed: {}", stderr));
        }

        Ok(())
    }

    pub fn list_dispatch_prs(&self, repo: &str, base_branch: &str) -> Result<Vec<DispatchPr>, String> {
        let mut cmd = self.gh()?;

        let output = cmd
            .args([
                "pr", "list",
                "--repo", repo,
                "--base", base_branch,
                "--state", "all", "--limit", "20",
                "--json", "number,title,state,author,assignees,labels,createdAt,updatedAt,headRefName,body,url",
            ])
            .output()
            .map_err(|e| format!("Failed to run gh: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(format!("Failed to list dispatch PRs: {}", stderr));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.trim().is_empty() {
            return Ok(vec![]);
        }

        let prs: Vec<DispatchPr> = serde_json::from_str(&stdout)
            .map_err(|e| format!("Failed to parse dispatch PRs: {}", e))?;

        Ok(prs)
    }

    pub fn fetch_org_priorities(&self, repo: &str) -> Result<Vec<OrgPriority>, String> {
        let mut cmd = self.gh()?;

        let output = cmd
            .args([
                "api",
                &format!("repos/{}/contents/priorities.yaml", repo),
                "--jq", ".content",
            ])
            .output()
            .map_err(|e| format!("Failed to run gh: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(format!("Failed to fetch priorities: {}", stderr));
        }

        let b64 = String::from_utf8_lossy(&output.stdout).trim().replace('\n', "");
        let decoded = base64_decode(&b64)
            .map_err(|e| format!("Failed to decode priorities: {}", e))?;
        let content = String::from_utf8(decoded)
            .map_err(|e| format!("Invalid UTF-8 in priorities: {}", e))?;

        // Parse YAML — serde_yaml or manual parse
        // Use serde_json via a simple YAML-to-JSON approach since we have serde_json
        // Actually, parse the YAML manually since we don't have serde_yaml
        let mut priorities = Vec::new();
        let mut current: Option<std::collections::HashMap<String, String>> = None;
        let mut current_list_key: Option<String> = None;
        let mut current_list: Vec<String> = Vec::new();

        for line in content.lines() {
            if line.starts_with("  - id:") {
                // Save previous priority
                if let Some(ref map) = current {
                    if let Some(p) = map_to_priority(map) {
                        priorities.push(p);
                    }
                }
                let mut map = std::collections::HashMap::new();
                map.insert("id".to_string(), line.trim_start_matches("  - id:").trim().to_string());
                current = Some(map);
                current_list_key = None;
            } else if let Some(ref mut map) = current {
                let trimmed = line.trim();
                if trimmed.starts_with("- ") && current_list_key.is_some() {
                    let val = trimmed.trim_start_matches("- ").trim().to_string();
                    current_list.push(val);
                    if let Some(ref key) = current_list_key {
                        map.insert(key.clone(), current_list.join(","));
                    }
                } else if let Some(colon_idx) = trimmed.find(':') {
                    let key = trimmed[..colon_idx].trim().to_string();
                    let val = trimmed[colon_idx + 1..].trim().to_string();
                    if val.is_empty() {
                        // Could be a list or multiline
                        current_list_key = Some(key);
                        current_list = Vec::new();
                    } else {
                        current_list_key = None;
                        let clean = val.trim_matches('"').trim_matches('\'').to_string();
                        // Handle inline arrays like [revenue, product]
                        if clean.starts_with('[') && clean.ends_with(']') {
                            let inner = clean[1..clean.len()-1].to_string();
                            map.insert(key, inner);
                        } else {
                            map.insert(key, clean);
                        }
                    }
                }
            }
        }
        // Save last
        if let Some(ref map) = current {
            if let Some(p) = map_to_priority(map) {
                priorities.push(p);
            }
        }

        Ok(priorities)
    }

    pub fn list_dispatch_prs_sent(&self, repo: &str) -> Result<Vec<DispatchPr>, String> {
        let mut cmd = self.gh()?;

        let output = cmd
            .args([
                "pr", "list",
                "--repo", repo,
                "--author", "@me",
                "--state", "all", "--limit", "20",
                "--json", "number,title,state,author,assignees,labels,createdAt,updatedAt,headRefName,body,url",
            ])
            .output()
            .map_err(|e| format!("Failed to run gh: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(format!("Failed to list sent dispatches: {}", stderr));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.trim().is_empty() {
            return Ok(vec![]);
        }

        let prs: Vec<DispatchPr> = serde_json::from_str(&stdout)
            .map_err(|e| format!("Failed to parse sent dispatches: {}", e))?;

        Ok(prs)
    }
}
