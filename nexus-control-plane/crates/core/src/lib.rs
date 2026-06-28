#[macro_export]
macro_rules! log_safe {
    ($($arg:tt)*) => {{
        use std::io::Write;
        let _ = writeln!(std::io::stderr(), $($arg)*);
    }};
}

pub mod claude_session;
pub mod db;
pub mod events;
pub mod github;
pub mod jsonl_types;
pub mod idle;
pub mod nexuslink;
pub mod platform;
pub mod pty;
pub mod services;
pub mod settings;
pub mod slack;
pub mod status;
pub mod stuck;
pub mod relay;
pub mod substrate;
pub mod evidence;
pub mod winddown;
pub mod tailscale;
pub mod types;
pub mod workspace;
