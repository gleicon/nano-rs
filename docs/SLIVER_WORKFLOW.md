# Sliver Workflow


How to bundle, run, and hot-swap an application with slivers.

## What a sliver is

A sliver is a portable **tar bundle** of an application:

```
app.sliver (tar archive)
├── meta.json          # hostname, timestamps, V8 bytecode version tag
├── bytecode.v8bc      # precompiled V8 bytecode (optional; omitted for --source-only)
├── vfs/               # the app's files
│   ├── index.js
│   ├── data/…
│   └── assets/…
└── manifest.txt       # human-readable listing
```

A sliver carries **no heap snapshot**. The app runs from its VFS source; when
`bytecode.v8bc` is present and its version tag matches the running V8, the first
request on a new isolate skips JavaScript parse + compile. (Restoring an isolate
from a baked heap blob is not implemented.)

## Create a sliver

From a directory — the CI/CD path, no running server required:

```bash
# Bundle ./my-app into a sliver, compiling the entrypoint to bytecode.
nano-rs sliver create --from-dir ./my-app --output my-app.sliver

# Portable across V8 versions (skips bytecode; larger cold start):
nano-rs sliver create --from-dir ./my-app --output my-app.sliver --source-only
```

Manage slivers in the local store:

```bash
nano-rs sliver list
nano-rs sliver delete my-app
```

## Run from a sliver

```bash
# The hostname comes from meta.json; override with --hostname if needed.
nano-rs run --sliver ./my-app.sliver --workers 4
```

All requests route through the app's `fetch` handler (WinterTC style). Requests
with a non-matching `Host` header get 404 unless you pass `--static`.

## Hot-swap without changing the subdomain

A running sliver app can be replaced in place with **zero downtime** — the hostname
never changes. On `SIGHUP`, NANO re-reads the sliver file and performs a blue-green
pool swap: it builds a fresh, fully-warm worker pool from the new bundle, atomically
repoints traffic to it, and drains the old pool (in-flight requests finish on it,
then its workers exit).

```bash
# Deploy a new version over the same file the server was started with…
cp my-app-v2.sliver ./my-app.sliver

# …then signal the running process to swap it in.
kill -HUP <nano-rs pid>
```

New requests immediately hit the new version; requests already in flight complete on
the old pool. If the new sliver fails to read or unpack, the current pool keeps
serving and the failure is logged — a bad deploy never takes the app down.

Programmatically, the same mechanism is `SliverPoolSlot::hotswap` /
`SliverPoolSlot::hotswap_and_drain` in `nano::worker`.

## Bytecode compatibility

`meta.json` records the V8 cache-version tag at pack time. At serve time the runtime
compares it against the running V8 (`UnpackedSliver::bytecode_matches_v8`). On a
mismatch it ignores the embedded bytecode and compiles from source — always correct,
just without the compile-skip. Rebuild the sliver after a V8 upgrade to restore the
fast path.

## References

- [Cold Start Terminology](COLD_START.md)
- [Performance & Tuning](PERFORMANCE.md)
- [Sliver format](../src/sliver/FORMAT.md)
