use regex::Regex;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const DEFAULT_PATTERNS_JSON: &str = include_str!("../../../idle_patterns.json");
const COOLDOWN_SECS: u64 = 300; // 5 minutes per session

#[derive(Debug)]
pub struct IdlePattern {
    pub name: String,
    pub regex: Regex,
    pub slack_emoji: String,
}

/// Load idle patterns from the embedded JSON file.
/// Returns an empty vec (with a warning) if parsing fails — never crashes.
pub fn load_patterns() -> Vec<IdlePattern> {
    #[derive(serde::Deserialize)]
    struct RawPattern {
        name: String,
        regex: String,
        #[serde(default)]
        slack_emoji: String,
        #[serde(default = "default_true")]
        enabled: bool,
    }
    fn default_true() -> bool {
        true
    }

    fn compile_patterns(raw: Vec<RawPattern>) -> Vec<IdlePattern> {
        raw.into_iter()
            .filter(|p| p.enabled)
            .filter_map(|p| {
                match Regex::new(&p.regex) {
                    Ok(re) => Some(IdlePattern {
                        name: p.name,
                        regex: re,
                        slack_emoji: p.slack_emoji,
                    }),
                    Err(e) => {
                        crate::log_safe!("[idle] Bad regex for pattern '{}': {}", p.name, e);
                        None
                    }
                }
            })
            .collect()
    }

    // Check for runtime override in $NCC_DATA_DIR/idle_patterns.json
    if let Ok(data_dir) = std::env::var("NCC_DATA_DIR") {
        let path = PathBuf::from(&data_dir).join("idle_patterns.json");
        if path.is_file() {
            match std::fs::read_to_string(&path) {
                Ok(contents) => {
                    match serde_json::from_str::<Vec<RawPattern>>(&contents) {
                        Ok(raw) => {
                            crate::log_safe!("[idle] Loaded runtime idle patterns from {}", path.display());
                            let patterns = compile_patterns(raw);
                            crate::log_safe!("[idle] Loaded {} idle detection patterns", patterns.len());
                            return patterns;
                        }
                        Err(e) => {
                            crate::log_safe!("[idle] Runtime idle_patterns.json at {} is invalid ({}), falling back to compiled defaults", path.display(), e);
                        }
                    }
                }
                Err(e) => {
                    crate::log_safe!("[idle] Failed to read runtime idle_patterns.json at {} ({}), falling back to compiled defaults", path.display(), e);
                }
            }
        }
    }

    // Fall through: parse compiled defaults
    let raw: Vec<RawPattern> = match serde_json::from_str(DEFAULT_PATTERNS_JSON) {
        Ok(v) => v,
        Err(e) => {
            crate::log_safe!("[idle] Failed to parse idle_patterns.json: {}", e);
            return Vec::new();
        }
    };

    let patterns = compile_patterns(raw);
    crate::log_safe!("[idle] Loaded {} idle detection patterns", patterns.len());
    patterns
}

/// Per-session idle state machine. Tracks active→idle transitions.
pub struct IdleDetector {
    is_idle: bool,
    last_notified: Option<Instant>,
}

impl IdleDetector {
    pub fn new() -> Self {
        Self {
            is_idle: false,
            last_notified: None,
        }
    }

    /// Process a flushed batch of ANSI-stripped text.
    /// Returns Some(pattern_name, emoji) on an active→idle transition that passes cooldown.
    pub fn check<'a>(&mut self, stripped: &str, patterns: &'a [IdlePattern]) -> Option<(&'a str, &'a str)> {
        // Check last non-empty line against patterns
        let last_line = stripped.lines().rev().find(|l| !l.trim().is_empty());

        let matched = last_line.and_then(|line| {
            patterns.iter().find(|p| p.regex.is_match(line.trim()))
        });

        if let Some(pattern) = matched {
            if !self.is_idle {
                self.is_idle = true;
                // Cooldown check
                let past_cooldown = self
                    .last_notified
                    .map(|t| t.elapsed() > Duration::from_secs(COOLDOWN_SECS))
                    .unwrap_or(true);
                if past_cooldown {
                    self.last_notified = Some(Instant::now());
                    return Some((&pattern.name, &pattern.slack_emoji));
                }
            }
        } else if !stripped.trim().is_empty() {
            // Non-idle output received — reset to active
            self.is_idle = false;
        }

        None
    }

    pub fn is_idle(&self) -> bool {
        self.is_idle
    }
}
