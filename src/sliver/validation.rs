//! Sliver Validation and Integrity Checking
//!
//! Provides validation for sliver files including corruption detection,
//! version compatibility checking, and integrity verification.

use crate::sliver::error::{SliverError, SliverResult};
use crate::sliver::format::METADATA_FILENAME;
use crate::sliver::metadata::SliverMetadata;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Types of corruption that can be detected
#[derive(Debug, Clone)]
pub enum CorruptionType {
    /// Invalid tar archive structure
    InvalidTar { reason: String },
    /// Missing metadata file
    MissingMetadata,
    /// Sliver has no payload (no bytecode, VFS entries, or legacy heap blob)
    MissingPayload,
    /// Invalid metadata JSON
    InvalidMetadata { error: String },
    /// Truncated file
    TruncatedFile {
        expected: u64,
        found: u64,
        entry: String,
    },
    /// Empty required file
    EmptyFile { entry: String },
    /// Wrong file extension
    InvalidExtension { found: String },
}

impl std::fmt::Display for CorruptionType {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            CorruptionType::InvalidTar { reason } => {
                write!(f, "Invalid tar archive: {}", reason)
            }
            CorruptionType::MissingMetadata => {
                write!(f, "Missing {} (required)", METADATA_FILENAME)
            }
            CorruptionType::MissingPayload => {
                write!(
                    f,
                    "Sliver has no payload (no bytecode, VFS entries, or heap blob)"
                )
            }
            CorruptionType::InvalidMetadata { error } => {
                write!(f, "Corrupted {}: {}", METADATA_FILENAME, error)
            }
            CorruptionType::TruncatedFile {
                expected,
                found,
                entry,
            } => {
                write!(
                    f,
                    "Truncated file {}: expected {} bytes, found {}",
                    entry, expected, found
                )
            }
            CorruptionType::EmptyFile { entry } => {
                write!(f, "Empty required file: {}", entry)
            }
            CorruptionType::InvalidExtension { found } => {
                write!(f, "Invalid file extension: {} (expected .sliver)", found)
            }
        }
    }
}

/// Validate a sliver file's integrity
pub fn validate_sliver_integrity(path: &Path) -> SliverResult<()> {
    // Check file exists
    if !path.exists() {
        return Err(SliverError::IoError(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Sliver file not found: {}", path.display()),
        )));
    }

    // Check extension
    let ext = path.extension().and_then(|e| e.to_str());
    if ext != Some("sliver") {
        return Err(SliverError::CorruptedArchive {
            reason: format!(
                "Invalid extension: {} (expected .sliver)",
                ext.unwrap_or("none")
            ),
        });
    }

    // Open and validate tar structure
    let file = std::fs::File::open(path)?;

    let mut archive = tar::Archive::new(file);

    let mut found_metadata = false;
    let mut found_payload = false; // heap.bin (v1) or bytecode.v8bc (v2) or vfs/* entries
    let mut errors = Vec::new();

    match archive.entries() {
        Ok(entries) => {
            for entry_result in entries {
                match entry_result {
                    Ok(mut entry) => {
                        let path_result = entry.path();
                        let size = entry.size();

                        match path_result {
                            Ok(path) => {
                                let path_str = path.to_string_lossy();

                                match path_str.as_ref() {
                                    METADATA_FILENAME => {
                                        found_metadata = true;
                                        if size == 0 {
                                            errors.push(CorruptionType::EmptyFile {
                                                entry: METADATA_FILENAME.to_string(),
                                            });
                                        } else {
                                            // Try to parse as JSON
                                            let mut content = String::new();
                                            if let Err(e) = entry.read_to_string(&mut content) {
                                                errors.push(CorruptionType::InvalidMetadata {
                                                    error: e.to_string(),
                                                });
                                            } else if let Err(e) =
                                                serde_json::from_str::<SliverMetadata>(&content)
                                            {
                                                errors.push(CorruptionType::InvalidMetadata {
                                                    error: e.to_string(),
                                                });
                                            }
                                        }
                                    }
                                    "heap.bin" | "bytecode.v8bc" => {
                                        found_payload = true;
                                        if size == 0 {
                                            errors.push(CorruptionType::EmptyFile {
                                                entry: path_str.to_string(),
                                            });
                                        }
                                    }
                                    p if p.starts_with("vfs/") => {
                                        found_payload = true;
                                    }
                                    _ => {}
                                }
                            }
                            Err(e) => {
                                errors.push(CorruptionType::InvalidTar {
                                    reason: format!("Invalid entry path: {}", e),
                                });
                            }
                        }
                    }
                    Err(e) => {
                        errors.push(CorruptionType::InvalidTar {
                            reason: e.to_string(),
                        });
                    }
                }
            }
        }
        Err(e) => {
            return Err(SliverError::CorruptedArchive {
                reason: format!("Failed to read archive: {}", e),
            });
        }
    }

    // Check required files
    if !found_metadata {
        errors.push(CorruptionType::MissingMetadata);
    }
    if !found_payload {
        errors.push(CorruptionType::MissingPayload);
    }

    // Return first error if any
    if let Some(first_error) = errors.into_iter().next() {
        return Err(SliverError::CorruptedArchive {
            reason: first_error.to_string(),
        });
    }

    Ok(())
}

/// Find sliver file with smart searching
pub fn find_sliver_file(name_or_path: &str) -> Option<PathBuf> {
    // Try direct path first
    let path = Path::new(name_or_path);
    if path.exists() && path.is_file() {
        return Some(path.to_path_buf());
    }

    // Search in common locations
    let home_dir = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();

    let search_paths = vec![
        format!("./{}.sliver", name_or_path),
        format!("./slivers/{}.sliver", name_or_path),
        format!("{}/.nano/slivers/{}.sliver", home_dir, name_or_path),
    ];

    for path_str in &search_paths {
        let path = Path::new(path_str);
        if path.exists() && path.is_file() {
            return Some(path.to_path_buf());
        }
    }

    None
}

/// Find similar sliver names for typo suggestions
pub fn find_similar_slivers(input: &str, search_dirs: &[PathBuf]) -> Option<String> {
    let mut candidates = Vec::new();

    for dir in search_dirs {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                if let Some(name) = entry.path().file_stem() {
                    if let Some(name_str) = name.to_str() {
                        candidates.push(name_str.to_string());
                    }
                }
            }
        }
    }

    find_similar_internal(input, &candidates, 3)
}

/// Internal Levenshtein distance implementation for typo suggestions
fn find_similar_internal(target: &str, candidates: &[String], threshold: usize) -> Option<String> {
    let mut best_match: Option<(String, usize)> = None;

    for candidate in candidates {
        let distance = levenshtein_distance(target, candidate);
        if distance <= threshold && distance < target.len() {
            if best_match.as_ref().map_or(true, |(_, d)| distance < *d) {
                best_match = Some((candidate.clone(), distance));
            }
        }
    }

    best_match.map(|(s, _)| s)
}

/// Calculate Levenshtein distance between two strings
fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_len = a.chars().count();
    let b_len = b.chars().count();

    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    let mut matrix = vec![vec![0; b_len + 1]; a_len + 1];

    for i in 0..=a_len {
        matrix[i][0] = i;
    }
    for j in 0..=b_len {
        matrix[0][j] = j;
    }

    for (i, a_char) in a.chars().enumerate() {
        for (j, b_char) in b.chars().enumerate() {
            let cost = if a_char == b_char { 0 } else { 1 };
            matrix[i + 1][j + 1] = (matrix[i][j + 1] + 1)
                .min(matrix[i + 1][j] + 1)
                .min(matrix[i][j] + cost);
        }
    }

    matrix[a_len][b_len]
}

/// Check NANO version compatibility
pub fn check_version_compatibility(
    metadata: &SliverMetadata,
    runtime_version: &str,
) -> SliverResult<()> {
    // Parse major versions from nano_version
    let sliver_major = metadata
        .nano_version
        .split('.')
        .next()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0);

    let runtime_major = runtime_version
        .split('.')
        .next()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0);

    // Major version differences may indicate incompatibility
    if runtime_major != sliver_major && runtime_major > 0 && sliver_major > 0 {
        // For now, just warn - in production might want to be stricter
        eprintln!(
            "Warning: NANO version mismatch: runtime={}, sliver={}. Some features may differ.",
            runtime_version, metadata.nano_version
        );
    }

    Ok(())
}

/// Get runtime V8 version
///
/// Returns the actual V8 engine version from the rusty_v8 crate.
/// This is used for sliver compatibility checking.
pub fn get_runtime_v8_version() -> String {
    v8::V8::get_version().to_string()
}

/// Check if a sliver can be restored with fallback
pub fn can_restore_with_fallback(_metadata: &SliverMetadata) -> bool {
    // Can restore with fallback if we have an entrypoint
    // or if the V8 snapshot restoration fails but we can create fresh
    true // Simplified - actual logic depends on context
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    fn create_test_sliver(dir: &Path, name: &str, valid: bool) -> PathBuf {
        use tar::{Builder, Header};

        let path = dir.join(format!("{}.sliver", name));
        let mut builder = Builder::new(Vec::new());

        if valid {
            // Add metadata
            let metadata = r#"{"format_version":"1.0","hostname":"test.example.com","name":"test","created_at":"2026-04-20T00:00:00Z","nano_version":"1.1.0"}"#;
            let mut header = Header::new_gnu();
            header.set_path(METADATA_FILENAME).unwrap();
            header.set_size(metadata.len() as u64);
            header.set_cksum();
            builder.append(&header, metadata.as_bytes()).unwrap();

            // Add heap
            let heap = vec![0u8; 1024];
            let mut header = Header::new_gnu();
            header.set_path("heap.bin").unwrap();
            header.set_size(heap.len() as u64);
            header.set_cksum();
            builder.append(&header, heap.as_slice()).unwrap();
        } else {
            // Invalid - missing heap
            let metadata = r#"{"format_version":"1.0"}"#;
            let mut header = Header::new_gnu();
            header.set_path(METADATA_FILENAME).unwrap();
            header.set_size(metadata.len() as u64);
            header.set_cksum();
            builder.append(&header, metadata.as_bytes()).unwrap();
        }

        let data = builder.into_inner().unwrap();
        std::fs::write(&path, data).unwrap();
        path
    }

    #[test]
    fn test_validate_valid_sliver() {
        let temp_dir = TempDir::new().unwrap();
        let path = create_test_sliver(temp_dir.path(), "valid", true);

        let result = validate_sliver_integrity(&path);
        if let Err(ref e) = result {
            eprintln!("Validation error: {:?}", e);
        }
        assert!(result.is_ok(), "Validation failed: {:?}", result);
    }

    #[test]
    fn test_validate_missing_metadata() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("bad.sliver");

        // Create tar without metadata
        use tar::{Builder, Header};
        let mut builder = Builder::new(Vec::new());
        let heap = vec![0u8; 1024];
        let mut header = Header::new_gnu();
        header.set_path("heap.bin").unwrap();
        header.set_size(heap.len() as u64);
        header.set_cksum();
        builder.append(&header, heap.as_slice()).unwrap();

        let data = builder.into_inner().unwrap();
        std::fs::write(&path, data).unwrap();

        let result = validate_sliver_integrity(&path);
        assert!(result.is_err());
        let err_str = format!("{}", result.unwrap_err());
        assert!(
            err_str.contains("meta.json"),
            "Error should mention meta.json, got: {}",
            err_str
        );
    }

    #[test]
    fn test_find_sliver_file_direct() {
        let temp_dir = TempDir::new().unwrap();
        let path = create_test_sliver(temp_dir.path(), "test", true);

        let found = find_sliver_file(path.to_str().unwrap());
        assert_eq!(found, Some(path));
    }

    #[test]
    fn test_find_sliver_file_by_name() {
        let temp_dir = TempDir::new().unwrap();
        let _path = create_test_sliver(temp_dir.path(), "my-sliver", true);

        // Change to temp dir and search
        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        let found = find_sliver_file("my-sliver");
        std::env::set_current_dir(original).unwrap();

        assert!(found.is_some());
    }

    #[test]
    fn test_check_version_compatibility_same_version() {
        let metadata = SliverMetadata::new("test.example.com", "1.1.0");

        assert!(check_version_compatibility(&metadata, "1.1.0").is_ok());
    }

    #[test]
    fn test_find_similar_slivers() {
        let temp_dir = TempDir::new().unwrap();
        let _path1 = create_test_sliver(temp_dir.path(), "api-prod", true);
        let _path2 = create_test_sliver(temp_dir.path(), "api-staging", true);

        let similar = find_similar_slivers("api-prd", &[temp_dir.path().to_path_buf()]);
        assert_eq!(similar, Some("api-prod".to_string()));
    }
}
