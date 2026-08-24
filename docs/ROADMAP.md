# Roadmap

Future capabilities live here, not in code comments. Code uses terse `planned: <x>`
markers that point back to this file.

## done: heap-snapshot create/restore primitives

Creating and restoring a V8 heap-snapshot blob works and is tested end to end:

- `v8::snapshot::create_snapshot_from_nano(nano)` serializes a `snapshot_creator`
  NanoIsolate's heap into a loadable blob (`into_inner` releases the EPT sentinel,
  then `create_blob` walks the heap).
- `create_isolate_from_snapshot(&blob)` restores a bare isolate;
  `NanoIsolate::from_v8_isolate` re-wraps it (EPT sentinel + VFS).
- Tests: `create_snapshot_from_nano_produces_loadable_blob`,
  `nano_isolate_round_trips_through_snapshot`, `v150_snapshot_creator_produces_blob`.

## planned: bake the initialized runtime into slivers

The valuable optimization is baking the *initialized runtime* (all WinterTC/native
API bindings) into the snapshot so cold start skips `RuntimeAPIs::bind_all`. This is
blocked by a hard V8 constraint, verified empirically (not assumed):

- Snapshotting an isolate after `bind_all` makes V8 **abort the process**
  (`V8_Fatal` in `CreateSnapshotDataBlobInternal`, `handle not serialized`). V8's
  startup serializer refuses native-callback objects unless every native function
  pointer is registered in an **external-reference table**, supplied identically at
  snapshot-create and restore time.
- **Remaining work (a dedicated subsystem, not a small task):** build an
  `EXTERNAL_REFERENCES` registry covering all ~24 native binders, restructure isolate
  init into a bake-then-rebind flow, embed the blob + a V8 version tag in the sliver
  (mirroring bytecode's `v8_cache_version` gate), and restore in the worker pool with
  a VFS fallback on version mismatch. A missing or mis-ordered reference aborts the
  process, so this needs exhaustive coverage before it touches the serving path.
- **Why not shipped in 2.7.0:** rushing an abort-prone serving-path change would
  contradict a stable release. Cold start today uses the VFS + bytecode-cache path
  (below), which needs no snapshot and carries no abort risk.

## done: build-time bytecode caching

*(Not a future item — documented here because it's the working half of the "faster
cold start" story and is often confused with heap snapshots.)*

Using nano-rs as a **CI/CD / build step** already produces bytecode:

- `nano sliver create --from-dir <dir>` (`sliver::packager::create_sliver_from_directory`) compiles
  the entrypoint JS to V8 bytecode via `UnboundScript::create_code_cache()`, embeds it
  as `bytecode.v8bc`, and records the V8 cache-version tag. `--source-only` skips it.
- At serve time the worker pool checks `bytecode_matches_v8()` (version tag vs the
  running V8) and, when it matches, compiles with
  `ScriptCompiler::Source::new_with_cached_data(...)` — **skipping parse+compile**.

This is compile-time work done off the serving path, exactly the CI/CD boot-time win.
It does **not** need heap snapshots and is not blocked by any v8 150 limitation.
Tested by `compile_js_to_bytecode*`, `bytecode_matches_v8*`, `unpack_with_bytecode`.
