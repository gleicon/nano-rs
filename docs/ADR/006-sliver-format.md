# ADR-006: Sliver Snapshot Format

**Status:** Superseded (2026-08-24)  
**Date:** 2026-04-20  
**Deciders:** Core Team  
**Technical Story:** Design portable format for isolate snapshots

> **Superseded:** the shipped sliver format carries **no `heap.bin` heap snapshot**.
> A sliver is `meta.json` + `vfs/` + an optional precompiled `bytecode.v8bc`; the app
> runs from its VFS source. The ~267µs "snapshot restore" target below was never
> realized (heap-snapshot serving needs an external-reference table).
> Kept as a historical record; for the current format see
> [FORMAT.md](../../src/sliver/FORMAT.md).

---

## Context and Problem Statement

For edge deployment, we need:
1. **Fast isolate startup** — ~1ms target (currently ~267µs)
2. **Portable snapshots** — Deploy anywhere (dev → staging → prod)
3. **Full state capture** — Heap + filesystem + metadata
4. **Version-agnostic format** — Survive V8 upgrades

Challenge: V8 heap snapshots are version-specific. A snapshot created with V8 12.x won't work with V8 12.y if the heap layout changed. We need a format that:
- Captures all isolate state
- Works across V8 micro-versions
- Is inspectable/debuggable
- Is streamable (for large slivers)

---

## Decision Drivers

* **Portability** — Same sliver works on any NANO version
* **Speed** — Sub-millisecond restore time
* **Completeness** — Capture heap + VFS + metadata
* **Inspectability** — Can inspect without NANO (debugging)
* **Version safety** — Survive V8 upgrades

---

## Considered Options

### Option 1: Tar + Opaque Heap Blob

Standard tar with V8 heap as opaque bytes.

### Option 2: Pure V8 Snapshot

Fast, but version-specific.

### Option 3: Custom Binary Format

Efficient, but not inspectable.

### Option 4: JSON + base64

Human-readable, but huge and slow.

---

## Decision Outcome

**Chosen option: "Tar + Opaque Heap Blob"**

Sliver format structure:
```
sliver.tar
├── meta.json       # Metadata (host, created, version)
├── heap.bin        # Opaque V8 heap blob (version-specific)
├── manifest.txt    # File listing
└── vfs/            # Virtual filesystem
    ├── file1.js
    ├── file2.json
    └── ...
```

**Why tar?**
- **Human-inspectible** — `tar -tf app.sliver`
- **Standard tool support** — Works everywhere
- **Streaming friendly** — Can extract single files
- **Unix philosophy** — Simple, composable

**Why opaque heap?**
- **V8 handles details** — We don't parse heap structure
- **Just pass through** — To V8 on restore
- **Survives upgrades** — Recreated (not migrated) on V8 changes

**Rationale:**
- Balances portability, inspectability, and performance
- Tar works everywhere
- Opaque blob forces recreation on V8 changes (safe)
- ~267µs restore time (measured)

---

## Implementation Details

### Format Version

```json
{
  "format_version": "1.0",
  "created_at": "2026-04-20T14:32:11Z",
  "v8_version": "12.4.0",
  "hostname": "api.example.com",
  "snapshot_version": "v8-12.4.0-v1"
}
```

**Version string format:** `v8-{v8-major}-{snapshot-format}`

### Heap Blob

```rust
pub struct HeapBlob {
    // Opaque bytes from V8 SnapshotCreator
    // We don't parse this - just pass to V8
    data: Vec<u8>,
    version: String,  // V8 version that created it
}
```

**Key insight:** We store the V8 version that created the heap. On restore, we check compatibility. If mismatch, we recreate from source (slow but safe).

### VFS Directory

```
vfs/
├── src/
│   └── index.js
├── data/
│   └── config.json
└── static/
    └── style.css
```

All files from the VFS are stored uncompressed in the tar (for random access).

### Manifest

```
# manifest.txt
FORMAT_VERSION=1.0
V8_VERSION=12.4.0
HOSTNAME=api.example.com
FILE_COUNT=23
HEAP_SIZE=1048576
```

Quick metadata without parsing JSON.

---

## Performance

| Metric | Value |
|--------|-------|
| Sliver creation | ~50-100ms |
| Sliver restore | ~267µs |
| Size | ~10-100KB (typical) |
| Compression | gzip (optional) |
| Portability | Yes (opaque heap forces recreation on mismatch) |

### Size Breakdown

Typical sliver (Hono.js app):
```
meta.json        200 bytes
manifest.txt     150 bytes
heap.bin        ~500KB (V8 heap)
vfs/            ~50KB (JS files, assets)
────────────────────────────
Total           ~550KB
Gzipped         ~150KB
```

---

## Version Compatibility

### Detection

```rust
fn check_compatibility(sliver: &Sliver) -> Compatibility {
    let current_v8 = v8::V8::get_version();
    let sliver_v8 = &sliver.meta.v8_version;
    
    // Major.minor must match
    if current_v8.major() == sliver_v8.major()
        && current_v8.minor() == sliver_v8.minor() {
        Compatibility::Compatible
    } else {
        Compatibility::Incompatible
    }
}
```

### Recovery

On version mismatch:
1. Log warning: "Sliver V8 version mismatch, recreating from source"
2. Create fresh isolate from source (~50-100ms)
3. Option: Create new sliver in background
4. Future requests use restored sliver

---

## Positive Consequences

* **Portability** — Tar works everywhere
* **Inspectability** — Can debug with standard tools
* **Version safety** — Opaque blob forces recreation on mismatch
* **~267µs restore time** — Fast cold starts
* **Streaming** — Can extract files without full unpack
* **Compression optional** — Speed vs size trade-off

---

## Negative Consequences

* **Larger than pure binary format** — no archive-level compression
* **Heap blob not human-readable** — Opaque by design
* **Requires tar library** — Additional dependency
* **Version mismatch requires recreation** — ~50-100ms fallback
* **Not as fast as pure V8 snapshot** — Tar parsing adds overhead

---

## Alternatives Rejected

### Option 2: Pure V8 Snapshot — Rejected

**Why:** Not portable across V8 versions. Would require all production to use exact same V8 build.

### Option 3: Custom Binary Format — Rejected

**Why:** Not inspectable, requires custom tooling. Tar is standard, debuggable.

### Option 4: JSON + base64 — Rejected

**Why:** Huge size (base64 bloat), slow parsing, not streamable.

---

## CLI Integration

### Create Sliver

```bash
# From directory
nano-rs sliver create ./my-app --output app.sliver

# From running app
nano-rs sliver create api.example.com --output app.sliver
```

### Inspect Sliver

```bash
# List contents
tar -tf app.sliver

# Extract metadata
tar -xf app.sliver meta.json -O | jq

# View manifest
tar -xf app.sliver manifest.txt -O
```

### Restore (Automatic)

```bash
# NANO automatically restores on run
nano-rs run --sliver app.sliver
```

---

## Related Decisions

* [ADR-004: VFS Architecture](004-vfs-architecture.md) — VFS state captured in sliver
* `docs/SLIVER_WORKFLOW.md` — User-facing sliver documentation
* `src/sliver/FORMAT.md` — Format specification
* `src/sliver/` — Implementation

---

## Code References

- `src/sliver/create.rs` — Sliver creation
- `src/sliver/restore.rs` — Sliver restoration
- `src/sliver/format.rs` — Format definitions
- `src/sliver/validate.rs` — Version validation

---

*Last updated: 2026-04-20*
