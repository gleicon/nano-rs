# Sliver Format Specification

## Overview

A sliver is a **tar-based archive** that bundles a JavaScript edge app — its files
plus a bit of metadata — into a single, portable deployment artifact. The app runs
from the bundled files (VFS); there is no heap snapshot.

## Design Goals

- **Simplicity**: Standard tar format, inspectable with standard tools
- **Portability**: No host-specific paths or identifiers
- **Evolvability**: Format supports future extensions (bytecode, snapshots, compression)

## Archive Structure

```
app.sliver (tar archive)
├── meta.json          # Required: JSON metadata
├── bytecode.v8bc      # Optional: pre-compiled V8 UnboundScript bytes
└── vfs/               # Required: app files (JS source, assets, modules, WASM)
    ├── index.js
    └── assets/
        └── logo.png
```

> Earlier drafts of this spec required a `heap.bin` V8 heap snapshot. Slivers carry
> no heap snapshot; a sliver runs from its VFS source.

## File Specifications

### meta.json (Required)

JSON metadata describing the snapshot:

```json
{
  "hostname": "app.example.com",
  "created_at": "2026-04-20T12:34:56.789Z",
  "format_version": "1.0",
  "nano_version": "1.1.0",
  "description": "Production deployment v2.3.1",
  "custom": {
    "deployment": "production",
    "git_sha": "abc123"
  }
}
```

**Fields:**
- `hostname` (string, required): Virtual hostname for the app
- `created_at` (string, required): ISO 8601 timestamp
- `format_version` (string, required): Sliver format version (currently "1.0")
- `nano_version` (string, required): NANO runtime version that created the snapshot
- `description` (string, optional): Human-readable description
- `custom` (object, optional): Application-specific key-value pairs

### bytecode.v8bc (Optional)

Opaque binary blob of pre-compiled V8 `UnboundScript` bytecode. Generated at pack
time by `nano sliver create` (via `UnboundScript::create_code_cache`) and tagged with
the V8 cache-version; the runtime consumes it at load (`Source::new_with_cached_data`)
to skip parse+compile when the version matches. Omitted with `--source-only`; a
sliver runs from its VFS source when absent.

**Important:** The bytecode is version-specific to the V8 build. NANO treats it as an opaque blob and validates the version tag before use, falling back to source compilation on mismatch.

**Size:** Variable (typically 100KB - 10MB depending on isolate state)

**Format:** V8-specific binary format, not documented here.

### vfs/ (Optional)

Virtual filesystem contents stored under the `vfs/` prefix. Each file becomes a tar entry with the full path preserved.

**Path Format:**
- All paths use forward slashes (`/`) regardless of platform
- Paths are relative to the VFS root
- Directory structure is preserved

**Example Entry:**
```
Name: vfs/data/config.json
Size: 1234 bytes
Mode: 0644 (rw-r--r--)
MTime: [file modification time or snapshot time]
```

**Content:** Files are stored as-is (binary-safe). No encoding or compression is applied at the file level.

### manifest.txt (Generated)

Human-readable listing of all archive entries. Generated automatically during packing.

```
# Sliver Archive Manifest
# =========================

meta.json
bytecode.v8bc
vfs/data/config.json
vfs/assets/logo.png
```

This file is informational only and not used during loading.

## Tar Format Details

The archive uses standard tar format (ustar or GNU tar):

- **Format**: POSIX.1-2001 (pax) or GNU tar format
- **Compression**: None (uncompressed tar)
- **Encoding**: UTF-8 for pathnames
- **Checksum**: Standard tar checksum in header

### Entry Headers

Each entry has a tar header with:
- Name: Entry path (e.g., "meta.json", "vfs/data/config.json")
- Size: File size in bytes
- Mode: File permissions (0644 for files, 0755 for directories)
- MTime: Modification time (Unix timestamp)
- Checksum: Header checksum

### Directory Entries

Directories can be represented as:
1. Explicit directory entries (typeflag = '5')
2. Implicit via file paths (parent directories inferred)

NANO supports both approaches during unpacking.

## Format Versioning

### Current Version: 1.0

Version 1.0 defines the basic structure:
- Required: meta.json, vfs/ (app files)
- Optional: vfs/* entries
- No compression
- No delta support

### Version Compatibility

- **Reading**: NANO only supports reading version 1.0 archives
- **Writing**: NANO always writes version 1.0 archives
- **Forward compatibility**: Unknown entries are skipped during reading

## Tooling

### Viewing Contents

```bash
# List archive contents
tar -tf app-v1.sliver

# Extract specific file
tar -xf app-v1.sliver meta.json

# View metadata
tar -xf app-v1.sliver -O meta.json | jq .
```

### Creating Archives

While NANO provides `nano-rs snapshot create`, manual creation is possible:

```bash
# Create archive manually
tar -cf app-v1.sliver meta.json bytecode.v8bc vfs/
```

### Validating Archives

```bash
# Verify tar structure
tar -tf app-v1.sliver > /dev/null && echo "Valid tar"

# Check required files exist
tar -tf app-v1.sliver | grep -q "^meta.json$" && echo "Has metadata"
tar -tf app-v1.sliver | grep -q "^bytecode.v8bc$" && echo "Has bytecode"
```

## Security Considerations

### Path Traversal

- All paths in the archive are treated as relative to VFS root
- Path traversal attempts ("../") in archive entries are rejected
- Absolute paths (starting with "/") are normalized to relative

### File Permissions

- Permissions from the archive are informational only
- Access control is enforced by NANO's VFS layer, not tar mode bits
- Executable bits are preserved but not directly used

### Size Limits

- Maximum file size: Enforced by VFS ResourceLimits
- Maximum archive size: Enforced during streaming read
- Memory limits: Enforced during unpacking

### Validation

Before loading, NANO validates:
1. Tar structure is valid
2. Required files (meta.json) exist
3. Format version is supported
4. Metadata JSON is valid
5. Heap blob is non-empty

## Examples

### Minimal Sliver

```
test.sliver
├── meta.json (96 bytes)
└── bytecode.v8bc (1024 bytes)
```

### Full Sliver with VFS

```
production-v1.sliver (245 KB)
├── meta.json (156 bytes)
├── bytecode.v8bc (241,234 bytes)
├── manifest.txt (89 bytes)
└── vfs/
    ├── data/
    │   └── session-store.json (1,234 bytes)
    └── cache/
        └── precomputed.html (2,456 bytes)
```

### Metadata Example

```json
{
  "hostname": "api.example.com",
  "created_at": "2026-04-20T14:30:00.000Z",
  "format_version": "1.0",
  "nano_version": "1.1.0",
  "description": "API server deployment with session state",
  "custom": {
    "deployment_id": "deploy-2026-04-20-001",
    "git_sha": "a1b2c3d4",
    "environment": "production"
  }
}
```

## References

- [Tar Format (POSIX)](https://pubs.opengroup.org/onlinepubs/9699919799/utilities/pax.html)
- [V8 SnapshotCreator API](https://v8.github.io/api/head/classv8_1_1SnapshotCreator.html)
- [NANO VFS Documentation](../vfs/)

---

*Specification Version: 1.0*  
*Last Updated: 2026-04-20*
