use serde::Serialize;
use std::path::PathBuf;

#[derive(Serialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub hidden: bool,
}

fn validate_path(path: &str) -> Result<(), String> {
    let p = std::path::Path::new(path);
    for component in p.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err("Path traversal (..) not allowed".to_string());
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn read_file(path: String) -> Result<String, String> {
    validate_path(&path)?;
    std::fs::read_to_string(&path).map_err(|e| format!("Failed to read {}: {}", path, e))
}

#[tauri::command]
pub async fn write_file(path: String, content: String) -> Result<(), String> {
    validate_path(&path)?;
    std::fs::write(&path, &content).map_err(|e| format!("Failed to write {}: {}", path, e))
}

#[tauri::command]
pub async fn list_directory(path: String) -> Result<Vec<FileEntry>, String> {
    validate_path(&path)?;
    let dir = PathBuf::from(&path);
    if !dir.is_dir() {
        return Err(format!("Not a directory: {}", path));
    }

    let mut entries = Vec::new();
    let read_dir = std::fs::read_dir(&dir)
        .map_err(|e| format!("Failed to read directory {}: {}", path, e))?;

    for entry in read_dir {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = entry.file_name().to_string_lossy().to_string();
        let file_path = entry.path();
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };

        entries.push(FileEntry {
            hidden: {
                #[cfg(target_os = "windows")]
                {
                    use std::os::windows::fs::MetadataExt;
                    let attrs = metadata.file_attributes();
                    (attrs & 0x2) != 0 || name.starts_with('.') // FILE_ATTRIBUTE_HIDDEN or dot-prefix
                }
                #[cfg(not(target_os = "windows"))]
                {
                    name.starts_with('.')
                }
            },
            name,
            path: file_path.to_string_lossy().to_string(),
            is_dir: metadata.is_dir(),
            size: metadata.len(),
        });
    }

    // Sort: directories first, then alphabetical (case-insensitive)
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(entries)
}

#[tauri::command]
pub async fn copy_files(sources: Vec<String>, destination: String) -> Result<u32, String> {
    let dest = PathBuf::from(&destination);
    if !dest.is_dir() {
        return Err(format!("Destination is not a directory: {}", destination));
    }

    let mut copied = 0u32;
    for source in &sources {
        let src = PathBuf::from(source);
        let file_name = src
            .file_name()
            .ok_or_else(|| format!("Invalid source path: {}", source))?;
        let target = dest.join(file_name);

        if src.is_dir() {
            copy_dir_recursive(&src, &target)
                .map_err(|e| format!("Failed to copy directory {}: {}", source, e))?;
        } else {
            std::fs::copy(&src, &target)
                .map_err(|e| format!("Failed to copy {}: {}", source, e))?;
        }
        copied += 1;
    }

    Ok(copied)
}

#[tauri::command]
pub async fn get_home_dir() -> Result<String, String> {
    dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .ok_or_else(|| "Could not determine home directory".to_string())
}

fn copy_dir_recursive(src: &PathBuf, dst: &PathBuf) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue; // Skip symlinks to avoid copying unexpected trees
        }
        let target = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}
