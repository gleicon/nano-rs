# Sliver VFS Test Script

## Overview

The `test_sliver_vfs.sh` script demonstrates the complete sliver workflow with **real VFS files** - HTML, CSS, JavaScript, images, and configuration files.

## What It Tests

This script creates a **static website** and packages it into a sliver:

### Files Created

```
static-site/
├── index.html          # HTML page (864 bytes)
├── styles.css          # CSS styles (1175 bytes)
├── app.js              # JavaScript (313 bytes)
├── robots.txt          # SEO config (46 bytes)
├── data/
│   └── config.json     # Site config (254 bytes)
└── images/
    ├── logo.webp       # WebP image (41 bytes)
    └── banner.gif      # GIF image (43 bytes)
```

### Workflow

1. **Create Static Site** - Generates realistic web files
2. **Create Sliver** - `nano-rs sliver create` captures VFS files
3. **Verify VFS Contents** - Extracts and validates all files
4. **List Slivers** - Shows metadata with verbose output
5. **Demonstrate Run** - Shows how to serve from sliver

## Usage

### Basic Usage

```bash
./tests/test_sliver_vfs.sh
```

### With Custom Binary

```bash
./tests/test_sliver_vfs.sh /path/to/nano-rs
```

## Expected Output

### Successful Run

```
========================================
SLIVER VFS FUNCTIONAL TEST
========================================

✓ Found nano-rs binary

========================================
STEP 1: Creating Static Website
========================================
  📄 index.html (864 bytes, text/html)
  📄 styles.css (1175 bytes, text/css)
  📄 app.js (313 bytes, application/javascript)
  📄 images/logo.webp (41 bytes, image/webp)
  📄 images/banner.gif (43 bytes, image/gif)
  📄 data/config.json (254 bytes, application/json)
  📄 robots.txt (46 bytes, text/plain)
✓ Created static website with 7 files

========================================
STEP 2: Creating Sliver with VFS Files
========================================
Created sliver: static-site-v1-v1.0.sliver
  Hostname: site.example.com
  Name: static-site-v1
  Size: 467456 bytes
  Heap: 452783 bytes
✓ Sliver creation command completed

========================================
STEP 3: Verifying VFS Contents in Sliver
========================================
✓ Found VFS directory in sliver
  VFS contents:
    vfs/index.html (864 bytes)
    vfs/styles.css (1175 bytes)
    vfs/app.js (313 bytes)
    vfs/data/config.json (254 bytes)
    vfs/images/logo.webp (41 bytes)
    vfs/images/banner.gif (43 bytes)
  Files verified: 7 / 7
✓ index.html content verified
✓ styles.css content verified
✓ data/config.json content verified

========================================
ALL TESTS PASSED
========================================
```

## Key Features Tested

### ✓ File Types

- **HTML** - `index.html` with proper structure
- **CSS** - `styles.css` with variables and selectors
- **JavaScript** - `app.js` with global state
- **JSON** - `data/config.json` configuration
- **Images** - WebP and GIF format files
- **Text** - `robots.txt` plain text

### ✓ Directory Structure

Directory hierarchy is preserved:
- `vfs/` - Root VFS prefix
- `vfs/data/` - Configuration directory
- `vfs/images/` - Asset directory

### ✓ Content Integrity

Files are verified by:
- **Existence** - All expected files present
- **Size** - Byte count matches original
- **Content** - Specific strings verified:
  - `index.html`: "NANO Static Site"
  - `styles.css`: "primary-color"
  - `data/config.json`: "darkMode"

## Sliver Format with VFS

```
static-site-v1-v1.0.sliver (467 KB)
├── meta.json          # App metadata
├── bytecode.v8bc      # Precompiled V8 bytecode (optional)
├── manifest.txt       # Human-readable listing
└── vfs/               # Virtual filesystem
    ├── index.html
    ├── styles.css
    ├── app.js
    ├── robots.txt
    ├── data/
    │   └── config.json
    └── images/
        ├── logo.webp
        └── banner.gif
```

## Performance

| Metric | Value |
|--------|-------|
| Total sliver size | ~467 KB |
| Heap snapshot | ~452 KB |
| VFS overhead | ~15 KB |
| Files captured | 7 |
| Capture time | <100ms |

## Comparison with Placeholder Slivers

| Feature | Before (v137) | After (v139+) |
|---------|---------------|---------------|
| VFS files | ❌ Not captured | ✅ Full capture |
| Heap snapshot | 29 bytes (placeholder) | 452 KB (real) |
| Static assets | ❌ Not supported | ✅ Complete |
| Directory structure | ❌ Flat | ✅ Preserved |

## Troubleshooting

### Sliver Too Small

If sliver is <100KB, VFS files weren't captured:

```
Size: 4567 bytes
Heap: 29 bytes
```

**Fix:** Ensure you're using nano-rs with VFS capture enabled (v139+).

### Missing VFS Directory

If extraction shows no `vfs/` directory:

```
Extracted contents:
  ./bytecode.v8bc
  ./meta.json
```

**Cause:** Directory was empty or no files were readable.

**Fix:** Check file permissions and ensure files exist in source directory.

### Content Verification Failed

If content check fails:

```
✗ index.html content doesn't match
```

**Cause:** File was corrupted or modified during sliver creation.

**Fix:** Check sliver_create.log for errors.

## Running the Server

To actually serve the static site:

```bash
cd /path/to/static-site
nano-rs run --sliver ./static-site-v1-v1.0.sliver --workers 2
```

Then access:
- http://site.example.com/ - Serves `vfs/index.html`
- http://site.example.com/styles.css - Serves CSS
- http://site.example.com/images/logo.webp - Serves image

## CI Integration

### GitHub Actions

```yaml
- name: Test Sliver VFS Workflow
  run: |
    cargo build --release
    ./tests/test_sliver_vfs.sh ./target/release/nano-rs
```

## See Also

- [CLI Test Script](CLI_TEST_SCRIPT.md) - Basic CLI testing
- [Sliver Workflow](SLIVER_WORKFLOW.md) - Complete workflow guide
- [Rust Functional Tests](../tests/sliver_functional_test.rs) - API tests
