use std::path::PathBuf;

// ── Data directories ──

/// Returns the platform-appropriate app data directory.
/// - macOS: ~/Library/Application Support/NexusControlPlane
/// - Windows: %APPDATA%\NexusControlPlane  (via dirs::data_dir → {FOLDERID_RoamingAppData})
/// - Linux: ~/.local/share/NexusControlPlane (via $XDG_DATA_HOME or default)
pub fn app_data_dir() -> Result<PathBuf, String> {
    dirs::data_dir()
        .map(|d| d.join("NexusControlPlane"))
        .ok_or_else(|| "Could not determine app data directory".to_string())
}

/// Returns NCC's resolved data directory.
/// NCC_DATA_DIR takes precedence; otherwise use the platform app data directory.
pub fn ncc_data_dir() -> Result<PathBuf, String> {
    if let Ok(data_dir) = std::env::var("NCC_DATA_DIR") {
        return Ok(PathBuf::from(data_dir));
    }
    app_data_dir()
}

/// Returns the status sideband directory within the app data dir.
pub fn status_dir() -> Result<PathBuf, String> {
    app_data_dir().map(|d| d.join("status"))
}

// ── Shell spawn ──

/// Returns the default shell for the current platform.
/// - macOS/Linux: $SHELL env var, or /bin/bash as fallback
/// - Windows: powershell.exe (COMSPEC typically points to cmd.exe, but PowerShell is preferred)
pub fn default_shell() -> String {
    #[cfg(target_os = "windows")]
    {
        // Prefer PowerShell; COMSPEC is usually cmd.exe
        "powershell.exe".to_string()
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
    }
}

/// Returns true if the current platform uses login shell flags (-l).
/// Windows shells (powershell, cmd) do not support -l.
pub fn use_login_shell() -> bool {
    cfg!(not(target_os = "windows"))
}

/// Returns true if TERM env var should be set for this platform.
/// Windows ConPTY does not use TERM and some programs misbehave if it's set.
pub fn should_set_term() -> bool {
    cfg!(not(target_os = "windows"))
}
