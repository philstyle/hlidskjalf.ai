use std::path::PathBuf;

/// Card name → filesystem slug.
///
/// Rules: non-alphanumeric → hyphen, collapse consecutive hyphens,
/// trim leading/trailing hyphens, max 50 chars (truncate at char boundary).
pub fn slugify(name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();

    // Collapse consecutive hyphens
    let mut collapsed = String::with_capacity(slug.len());
    let mut prev_hyphen = false;
    for c in slug.chars() {
        if c == '-' {
            if !prev_hyphen {
                collapsed.push('-');
            }
            prev_hyphen = true;
        } else {
            collapsed.push(c);
            prev_hyphen = false;
        }
    }

    // Trim leading/trailing hyphens
    let trimmed = collapsed.trim_matches('-');

    // Truncate at 50 chars on a char boundary
    if trimmed.len() <= 50 {
        trimmed.to_string()
    } else {
        let mut end = 50;
        while !trimmed.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        trimmed[..end].trim_end_matches('-').to_string()
    }
}

/// Resolve workspace path with conflict suffix.
///
/// Returns `{workspace_root}/{slug}`. If that path exists on disk,
/// appends `-2`, `-3`, ..., up to `-100`, then errors.
pub fn resolve_workspace_path(workspace_root: &str, slug: &str) -> Result<PathBuf, String> {
    let base = PathBuf::from(workspace_root).join(slug);
    if !base.exists() {
        return Ok(base);
    }

    for i in 2..=100 {
        let candidate = PathBuf::from(workspace_root).join(format!("{}-{}", slug, i));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(format!(
        "Could not find available workspace path for '{}' after 100 attempts",
        slug
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify_basic() {
        assert_eq!(slugify("Hello World"), "hello-world");
    }

    #[test]
    fn test_slugify_special_chars() {
        assert_eq!(slugify("Ledger · Auth Refactor"), "ledger-auth-refactor");
    }

    #[test]
    fn test_slugify_exclamations() {
        assert_eq!(slugify("Hello   World!!!"), "hello-world");
    }

    #[test]
    fn test_slugify_max_length() {
        let long_name = "a".repeat(60);
        let slug = slugify(&long_name);
        assert!(slug.len() <= 50);
    }

    #[test]
    fn test_slugify_leading_trailing() {
        assert_eq!(slugify("---test---"), "test");
    }

    #[test]
    fn test_resolve_no_conflict() {
        let tmp = std::env::temp_dir().join("nx-test-ws-no-conflict");
        // Ensure dir doesn't exist
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let result = resolve_workspace_path(tmp.to_str().unwrap(), "my-project");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), tmp.join("my-project"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_resolve_with_conflict() {
        let tmp = std::env::temp_dir().join("nx-test-ws-conflict");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("my-project")).unwrap();

        let result = resolve_workspace_path(tmp.to_str().unwrap(), "my-project");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), tmp.join("my-project-2"));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
