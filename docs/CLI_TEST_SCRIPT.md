# CLI Test Script Documentation

## Overview

The `test_sliver_cli.sh` script provides comprehensive functional testing of the nano-rs sliver workflow using the command-line interface.

## What It Tests

The script validates the complete sliver lifecycle:

1. **Binary Check** - Verifies nano-rs binary exists and is executable
2. **Test App Creation** - Creates a sample app with:
   - JavaScript files (`index.js`, `package.json`)
   - Runtime data (`data/settings.json`, `data/cache.db`)
   - Asset files (`assets/image-*.png`)
3. **Sliver Creation** - Uses `nano-rs sliver create` to create snapshot
4. **Validation** - Checks tar archive format and required files
5. **Listing** - Tests `nano-rs sliver list` and `nano-rs sliver list --verbose`
6. **Inspection** - Extracts and examines sliver contents
7. **Run Demo** - Shows how to run from sliver
8. **Cleanup** - Removes test artifacts

## Usage

### Basic Usage

```bash
# Run with default binary location (./target/release/nano-rs)
./tests/test_sliver_cli.sh

# Run with custom binary path
./tests/test_sliver_cli.sh /path/to/nano-rs

# Run with debug binary
./tests/test_sliver_cli.sh ./target/debug/nano-rs
```

### With Output Capture

```bash
# Run and save full output
./tests/test_sliver_cli.sh 2>&1 | tee sliver_test.log

# Run with colorized output preserved
script -q /dev/null ./tests/test_sliver_cli.sh 2>&1 | tee sliver_test.log
```

## Expected Output

### Successful Run

```
========================================
SLIVER CLI FUNCTIONAL TEST
========================================

✓ Found nano-rs binary: ./target/release/nano-rs
  Version: nano-rs 0.1.0

========================================
STEP 1: Creating Test App
========================================
  ✓ Created index.js
  ✓ Created package.json
  ✓ Created data/user-settings.json
  ✓ Created data/cache.db
  ✓ Created 5 asset files in assets/

========================================
STEP 2: Creating Sliver with CLI
========================================
Created sliver: test-app-v1-v1.0.sliver
  Hostname: test.example.com
  Name: test-app-v1
  Tag: v1.0
  Size: 456704 bytes
  Heap: 452783 bytes

========================================
STEP 3: Validating Sliver
========================================
✓ Sliver is a valid tar archive
✓ Contains meta.json
✓ Contains bytecode.v8bc (when not --source-only)
✓ Heap snapshot is substantial (452783 bytes)

========================================
STEP 4: Listing Slivers
========================================
Slivers:
  test-app-v1-v1.0 (456704 bytes)
    Name: test-app-v1
    Hostname: test.example.com
    ...

========================================
ALL TESTS PASSED
========================================
```

## Test Details

### Step 1: Test App Creation

Creates a realistic app structure:

```
app/
├── index.js          # Sets global state: appState.counter = 42
├── package.json      # App metadata
├── data/
│   ├── user-settings.json  # Runtime settings
│   └── cache.db            # Runtime cache
└── assets/
    ├── image-0.png   # Binary assets
    ├── image-1.png
    └── ...
```

### Step 2: Sliver Creation

Executes:
```bash
nano-rs sliver create test.example.com \
  --name test-app-v1 \
  --tag v1.0
```

**Expected Results:**
- Sliver file created: `test-app-v1-v1.0.sliver`
- Heap snapshot: ~450KB (real V8 snapshot, not placeholder)
- Total size: ~445KB

### Step 3: Validation

Checks:
- ✓ Valid tar archive format
- ✓ Contains `meta.json` (hostname, version, timestamps)
- ✓ Contains `bytecode.v8bc` when packed with bytecode (omitted for --source-only)
- ✓ JSON metadata is valid

### Step 4: Listing

Tests both commands:
```bash
nano-rs sliver list              # Brief listing
nano-rs sliver list --verbose  # Detailed with metadata
```

### Step 5: Inspection

Extracts and displays:
- Archive type and size
- All file entries with sizes
- `meta.json` contents
- `manifest.txt` contents (if present)
- VFS directory structure (if present)

### Step 6: Run Demonstration

Shows the command to run from sliver:
```bash
nano-rs run --sliver ./test-app-v1-v1.0.sliver --workers 4
```

### Step 7: Cleanup

Removes:
- Test directory with all artifacts
- Sliver file
- Temporary files

## Troubleshooting

### Binary Not Found

```
✗ nano-rs binary not found at: ./target/release/nano-rs
```

**Fix:** Build the binary first:
```bash
cargo build --release
```

### Sliver Name Validation Error

```
Error: ✗ Invalid sliver name: 'test-app-v1.0'
Reason: Invalid characters: . (only letters, numbers, hyphens, underscores allowed)
```

**Fix:** The script now uses valid names. If customizing, ensure names match:
- `^[a-zA-Z0-9][a-zA-Z0-9_-]*$`

### V8 Platform Not Initialized

If you see a segfault, the V8 platform initialization fix is not applied.

**Fix:** Ensure `handle_sliver_command()` in `main.rs` includes:
```rust
if !nano::v8::platform::is_initialized() {
    nano::v8::platform::initialize_platform()?;
}
```

### Permission Denied

```
bash: ./tests/test_sliver_cli.sh: Permission denied
```

**Fix:** Make executable:
```bash
chmod +x ./tests/test_sliver_cli.sh
```

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | All tests passed |
| 1 | One or more tests failed |

## Integration with CI

### GitHub Actions Example

```yaml
- name: Run Sliver CLI Tests
  run: |
    cargo build --release
    ./tests/test_sliver_cli.sh ./target/release/nano-rs
```

### GitLab CI Example

```yaml
test:sliver-cli:
  script:
    - cargo build --release
    - ./tests/test_sliver_cli.sh ./target/release/nano-rs
```

## Manual Testing Scenarios

### Test Different Hostnames

```bash
# Edit the script or use environment variables
APP_HOSTNAME="api.example.com" ./tests/test_sliver_cli.sh
```

### Test Sliver Creation Variants

```bash
# Without tag
$NANO_BIN sliver create test.example.com --name my-app

# With custom output path
$NANO_BIN sliver create test.example.com \
  --output /tmp/custom-name.sliver

# With all options
$NANO_BIN sliver create test.example.com \
  --name my-app \
  --tag v2.0 \
  --output ./my-app-v2.sliver
```

## Performance Benchmarks

The script captures timing information:

```
Heap snapshot: 452,783 bytes (~442 KB)
Sliver archive: 456,704 bytes (~446 KB)
Overhead: ~4KB (tar headers + metadata)
```

This indicates efficient packing with minimal overhead.

## See Also

- [Sliver Workflow Documentation](SLIVER_WORKFLOW.md) - Complete workflow guide
- [Sliver Format](../src/sliver/format.rs) - Format specification
- [Functional Tests](../tests/sliver_functional_test.rs) - Rust API tests
