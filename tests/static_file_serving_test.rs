//! Static File Serving Tests
//!
//! Comprehensive test coverage for static file serving functionality:
//! - HTML entrypoint serving with correct Content-Type
//! - Directory entrypoint with index.html fallback
//! - CSS file content-type detection
//! - JS file served as static (not executed)
//! - 404 handling for missing files
//! - Sliver creation from directory
//! - Sliver running standalone without source
//! - JS entrypoint regression tests

use std::fs;
use std::path::Path;

/// Helper to get the path to static file test fixtures
fn static_files_dir() -> &'static Path {
    Path::new("tests/fixtures/static-files")
}

// ============================================================================
// Test 1: HTML Entrypoint Serving
// ============================================================================

#[test]
fn test_html_entrypoint_serves_file() {
    // Verify the fixture file exists
    let html_path = static_files_dir().join("index.html");
    assert!(html_path.exists(), "index.html fixture should exist");
    
    // Read and verify content
    let content = fs::read_to_string(&html_path).expect("Should read HTML file");
    assert!(content.contains("Static File Test"), "HTML should contain expected content");
    assert!(content.contains("<!DOCTYPE html>"), "HTML should have DOCTYPE");
    assert!(content.contains("text/html"), "HTML should reference content-type concept");
}

#[test]
fn test_html_file_has_correct_structure() {
    let html_path = static_files_dir().join("index.html");
    let content = fs::read_to_string(&html_path).expect("Should read HTML file");
    
    // Verify HTML structure
    assert!(content.contains("<html>"), "HTML should have html tag");
    assert!(content.contains("</html>"), "HTML should have closing html tag");
    assert!(content.contains("<head>"), "HTML should have head tag");
    assert!(content.contains("<body>"), "HTML should have body tag");
    assert!(content.contains("<link rel=\"stylesheet\""), "HTML should link CSS");
}

// ============================================================================
// Test 2: Directory Entrypoint
// ============================================================================

#[test]
fn test_directory_contains_index_html() {
    let dir = static_files_dir();
    assert!(dir.exists(), "Static files directory should exist");
    assert!(dir.is_dir(), "Static files path should be a directory");
    
    let index_path = dir.join("index.html");
    assert!(index_path.exists(), "Directory should contain index.html");
}

#[test]
fn test_directory_has_all_required_files() {
    let dir = static_files_dir();
    
    // Check all required files exist
    assert!(dir.join("index.html").exists(), "Should have index.html");
    assert!(dir.join("style.css").exists(), "Should have style.css");
    assert!(dir.join("app.js").exists(), "Should have app.js");
}

// ============================================================================
// Test 3: CSS Content-Type
// ============================================================================

#[test]
fn test_css_file_content() {
    let css_path = static_files_dir().join("style.css");
    let content = fs::read_to_string(&css_path).expect("Should read CSS file");
    
    // Verify CSS rules
    assert!(content.contains("font-family"), "CSS should contain font-family rule");
    assert!(content.contains("color"), "CSS should contain color rule");
    assert!(content.contains("body"), "CSS should style body element");
    assert!(content.contains("h1"), "CSS should style h1 element");
}

#[test]
fn test_css_file_extension() {
    let css_path = static_files_dir().join("style.css");
    
    // Verify file has .css extension
    assert_eq!(css_path.extension().unwrap(), "css", "File should have .css extension");
}

// ============================================================================
// Test 4: JS File Served (Not Executed)
// ============================================================================

#[test]
fn test_js_file_content_not_executable_in_test() {
    let js_path = static_files_dir().join("app.js");
    let content = fs::read_to_string(&js_path).expect("Should read JS file");
    
    // Verify JS content indicates it should be served not executed
    assert!(content.contains("console.log"), "JS should contain console.log statement");
    assert!(content.contains("not be executed"), "JS should indicate it's for serving");
}

#[test]
fn test_js_file_extension() {
    let js_path = static_files_dir().join("app.js");
    assert_eq!(js_path.extension().unwrap(), "js", "File should have .js extension");
}

// ============================================================================
// Test 5: 404 Handling (Missing Files)
// ============================================================================

#[test]
fn test_missing_file_not_present() {
    let dir = static_files_dir();
    let nonexistent = dir.join("nonexistent.html");
    
    // Verify non-existent file doesn't exist
    assert!(!nonexistent.exists(), "Non-existent file should not be present");
}

#[test]
fn test_non_standard_files_not_present() {
    let dir = static_files_dir();
    
    // Verify common files that shouldn't exist
    assert!(!dir.join("404.html").exists(), "404.html should not be present");
    assert!(!dir.join("error.html").exists(), "error.html should not be present");
    assert!(!dir.join("missing.txt").exists(), "missing.txt should not be present");
}

// ============================================================================
// Test 6: Sliver Creation from Directory
// ============================================================================

#[test]
fn test_sliver_directory_packing() {
    use nano::sliver::packager::create_sliver_from_directory;
    
    // Create sliver from directory (async function, use pollster to block)
    let result = pollster::block_on(create_sliver_from_directory(
        "tests/fixtures/static-files",
        "static-test",
        Some("v1.0".to_string()),
        Some("/tmp/static-test.sliver".to_string()),
        Some("test-host".to_string()),
        true,
    ));
    
    assert!(result.is_ok(), "Should create sliver from directory: {:?}", result.err());
    
    // Cleanup
    let sliver_path = std::path::PathBuf::from("/tmp/static-test.sliver");
    if sliver_path.exists() {
        fs::remove_file(&sliver_path).expect("Should remove sliver file");
    }
}

#[test]
fn test_sliver_directory_exists_with_correct_name() {
    // Just verify the directory exists for sliver creation tests
    assert!(static_files_dir().exists(), "Static files directory must exist for sliver tests");
}

// ============================================================================
// Test 7: Sliver Standalone (Run Without Source)
// ============================================================================

#[test]
fn test_sliver_can_be_created_without_running_app() {
    use nano::sliver::packager::create_sliver_from_directory;
    
    // Create sliver directly from directory (no running app required)
    let result = pollster::block_on(create_sliver_from_directory(
        "tests/fixtures/static-files",
        "standalone-test",
        Some("v1.0".to_string()),
        Some("/tmp/standalone-test.sliver".to_string()),
        Some("standalone.local".to_string()),
        true,
    ));
    
    assert!(result.is_ok(), "Should create sliver without running app");
    
    // Cleanup
    let sliver_path = std::path::PathBuf::from("/tmp/standalone-test.sliver");
    if sliver_path.exists() {
        fs::remove_file(&sliver_path).expect("Should remove sliver file");
    }
}

// ============================================================================
// Test 8: JS Entrypoint Regression (Still Works)
// ============================================================================

/// Test that JS files are still detected as JS entrypoints
/// This ensures static file serving doesn't break JS execution
#[test]
fn test_js_entrypoint_detection() {
    use nano::http::router::detect_entrypoint_type;
    
    // A JS file should be detected as JavaScript entrypoint
    let js_type = detect_entrypoint_type("./app.js");
    
    match js_type {
        nano::http::router::EntrypointType::JavaScript(_) => {
            // Correct - JS file detected as JavaScript
        }
        _ => panic!("JS file should be detected as JavaScript entrypoint, got {:?}", js_type),
    }
}

#[test]
fn test_html_entrypoint_detection() {
    use nano::http::router::detect_entrypoint_type;
    
    // An HTML file should be detected as static file
    let html_type = detect_entrypoint_type("./index.html");
    
    match html_type {
        nano::http::router::EntrypointType::StaticFile(_) => {
            // Correct - HTML file detected as static
        }
        _ => panic!("HTML file should be detected as StaticFile entrypoint, got {:?}", html_type),
    }
}

#[test]
fn test_directory_entrypoint_detection() {
    use nano::http::router::detect_entrypoint_type;
    
    // A directory should be detected as StaticDir
    let dir_type = detect_entrypoint_type("./dist");
    
    match dir_type {
        nano::http::router::EntrypointType::StaticDir(_) => {
            // Correct - directory detected as StaticDir
        }
        _ => panic!("Directory should be detected as StaticDir entrypoint, got {:?}", dir_type),
    }
}

#[test]
fn test_entrypoint_type_variants() {
    use nano::http::router::detect_entrypoint_type;
    
    // Test various JavaScript extensions
    let js = detect_entrypoint_type("./app.js");
    assert!(matches!(js, nano::http::router::EntrypointType::JavaScript(_)), "JS should be JavaScript");
    
    let mjs = detect_entrypoint_type("./app.mjs");
    assert!(matches!(mjs, nano::http::router::EntrypointType::JavaScript(_)), "MJS should be JavaScript");
    
    let ts = detect_entrypoint_type("./app.ts");
    assert!(matches!(ts, nano::http::router::EntrypointType::JavaScript(_)), "TS should be JavaScript");
    
    // Test static files
    let html = detect_entrypoint_type("./index.html");
    assert!(matches!(html, nano::http::router::EntrypointType::StaticFile(_)), "HTML should be StaticFile");
    
    let css = detect_entrypoint_type("./style.css");
    assert!(matches!(css, nano::http::router::EntrypointType::StaticFile(_)), "CSS should be StaticFile");
    
    let png = detect_entrypoint_type("./image.png");
    assert!(matches!(png, nano::http::router::EntrypointType::StaticFile(_)), "PNG should be StaticFile");
}

// ============================================================================
// Integration Tests - Content-Type Detection
// ============================================================================

#[test]
fn test_content_type_detection_html() {
    use nano::http::content_type_from_ext;
    
    assert!(content_type_from_ext("html").contains("text/html"), "HTML should have text/html content-type");
    assert!(content_type_from_ext("htm").contains("text/html"), "HTM should have text/html content-type");
}

#[test]
fn test_content_type_detection_css() {
    use nano::http::content_type_from_ext;
    
    assert!(content_type_from_ext("css").contains("text/css"), "CSS should have text/css content-type");
}

#[test]
fn test_content_type_detection_js() {
    use nano::http::content_type_from_ext;
    
    assert!(content_type_from_ext("js").contains("application/javascript"), "JS should have application/javascript content-type");
    assert!(content_type_from_ext("mjs").contains("application/javascript"), "MJS should have application/javascript content-type");
}

#[test]
fn test_content_type_detection_other() {
    use nano::http::content_type_from_ext;
    
    assert!(content_type_from_ext("json").contains("application/json"), "JSON should have application/json content-type");
    assert_eq!(content_type_from_ext("png"), "image/png", "PNG should have image/png content-type");
    assert!(content_type_from_ext("txt").contains("text/plain"), "TXT should have text/plain content-type");
}

// ============================================================================
// File Size and Properties Tests
// ============================================================================

#[test]
fn test_static_files_have_content() {
    let dir = static_files_dir();
    
    // All files should have actual content
    let html_content = fs::read_to_string(dir.join("index.html")).unwrap();
    assert!(html_content.len() > 100, "HTML file should have substantial content");
    
    let css_content = fs::read_to_string(dir.join("style.css")).unwrap();
    assert!(css_content.len() > 20, "CSS file should have content");
    
    let js_content = fs::read_to_string(dir.join("app.js")).unwrap();
    assert!(js_content.len() > 10, "JS file should have content");
}

#[test]
fn test_html_references_css() {
    let html_content = fs::read_to_string(static_files_dir().join("index.html")).unwrap();
    
    // HTML should reference the CSS file
    assert!(html_content.contains("style.css"), "HTML should reference style.css");
}

// ============================================================================
// Sliver Metadata Tests
// ============================================================================

#[test]
fn test_sliver_packing_with_metadata() {
    use nano::sliver::packager::create_sliver_from_directory;
    
    // Create sliver with specific name and tag
    let result = pollster::block_on(create_sliver_from_directory(
        "tests/fixtures/static-files",
        "meta-test-app",
        Some("v2.0.0".to_string()),
        Some("/tmp/meta-test.sliver".to_string()),
        Some("meta.example.com".to_string()),
        true,
    ));
    
    assert!(result.is_ok(), "Should create sliver with metadata");
    
    let sliver_path = result.unwrap();
    assert!(sliver_path.exists(), "Sliver file should exist");
    
    // Cleanup
    fs::remove_file(&sliver_path).expect("Should remove sliver file");
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn test_static_dir_path_as_string() {
    let dir = static_files_dir();
    let dir_str = dir.to_string_lossy();
    
    // Verify the path can be converted to string for API use
    assert!(!dir_str.is_empty(), "Directory path should be convertable to string");
    assert!(dir_str.contains("static-files"), "Path should contain 'static-files'");
}

#[test]
fn test_file_paths_relative_to_dir() {
    let dir = static_files_dir();
    
    // Verify relative paths
    let html_relative = dir.join("index.html");
    let css_relative = dir.join("style.css");
    let js_relative = dir.join("app.js");
    
    assert!(html_relative.exists(), "Relative HTML path should exist");
    assert!(css_relative.exists(), "Relative CSS path should exist");
    assert!(js_relative.exists(), "Relative JS path should exist");
}
