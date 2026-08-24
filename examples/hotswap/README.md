# Zero-downtime sliver hot-swap

Deploy a new version of a sliver app **on the same subdomain, with no downtime**.
On `SIGHUP`, NANO re-reads the sliver file and does a blue-green pool swap: it builds
a fresh, fully-warm worker pool from the new bundle, atomically repoints traffic to
it, and drains the old pool (in-flight requests finish, then its workers exit).

This directory has two versions of a trivial app:

- `v1/index.js` — responds `{"version":"v1"}`
- `v2/index.js` — responds `{"version":"v2"}`

## Walkthrough

Build a binary first: `cargo build --release`. Then, from the repo root:

```bash
BIN=./target/release/nano-rs

# 1. Bundle v1 into the sliver file the server will watch.
$BIN sliver create --from-dir examples/hotswap/v1 --name demo --output app.sliver

# 2. Run it. Sliver mode binds 127.0.0.1:8080 (override the host with NANO_HOST).
#    The hostname comes from the bundle; override it with --hostname.
$BIN run --sliver app.sliver --hostname demo.local --workers 2 &
SERVER=$!
sleep 2

# 3. It serves v1.
curl -s -H 'Host: demo.local' http://127.0.0.1:8080/
#   → {"version":"v1","message":"hello from v1"}

# 4. Build v2 to a temp file and mv it over the watched path (atomic), then SIGHUP.
$BIN sliver create --from-dir examples/hotswap/v2 --name demo --output app-v2.sliver
mv app-v2.sliver app.sliver
kill -HUP $SERVER
sleep 2

# 5. Same host, same port, same process — now serving v2. No dropped requests.
curl -s -H 'Host: demo.local' http://127.0.0.1:8080/
#   → {"version":"v2","message":"hello from v2"}

kill $SERVER
```

> The handlers use the classic `function fetch(request) { return { status, body } }`
> form, which the sliver packer compiles to bytecode. `sliver create --output`
> refuses to overwrite an existing file, so step 4 writes a temp file and `mv`s it
> into place — which is also the safe, atomic way to deploy.

## Notes

- The hostname and port never change — clients and DNS are untouched.
- If the new sliver fails to read or unpack, the current version keeps serving and
  the error is logged; a bad deploy never takes the app down.
- In-flight requests complete on the old pool; only new requests go to v2.
- SIGHUP is a Unix signal. On other platforms, drive the swap programmatically via
  `SliverPoolSlot::hotswap` / `hotswap_and_drain` in `nano::worker`.
