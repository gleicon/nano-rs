# NANO Performance & Tuning

**Version:** 2.7.0
**Last Updated:** 2026-08-24

This guide describes the performance-relevant mechanisms and the knobs you can
tune. It deliberately does **not** publish benchmark figures — none are measured in
this repo, and invented numbers help no one. Measure on your own hardware with the
admin metrics endpoints below.

---

## What actually affects performance

- **Bytecode caching.** A sliver built with `nano sliver create` embeds precompiled
  V8 bytecode (`bytecode.v8bc`) and a V8 cache-version tag. When the tag matches the
  running V8, the first request on a new isolate skips JavaScript parse + compile.
  `--source-only` opts out (portable across V8 versions, but no compile skip).
- **Isolate model.** One V8 isolate per worker thread; a fresh context per request
  for isolation without recreating the isolate. Isolates recycle after a fixed
  number of requests.
- **Heap snapshots.** Restoring an isolate from a baked heap blob (which would also
  skip API binding) is **not** wired — see [ROADMAP](ROADMAP.md). Cold start today is
  the bytecode path above.

---

## Tuning knobs

All per-app, under `limits` in the config:

| Knob | Effect |
|------|--------|
| `workers` | Concurrency per app. More workers = more parallel requests and more resident isolates (more memory). A reasonable start is one per core you want to give the app. |
| `memory_mb` | Hard V8 heap cap per isolate (enforced via `CreateParams::heap_limits`). Lower fits more isolates; too low causes eviction/OOM responses. |
| `cpu_time_ms` | Per-request CPU budget; the request is terminated when exceeded. Default is Cloudflare-style tight; raise for CPU-heavy handlers. |

Example:

```json
{
  "apps": [
    {
      "hostname": "api.example.com",
      "sliver": "./app.sliver",
      "limits": { "workers": 4, "memory_mb": 128, "cpu_time_ms": 50 }
    }
  ]
}
```

For lowest cold-start latency, deploy a sliver with bytecode (don't use
`--source-only`) and pre-warm pools by sending a little traffic after start.

---

## Measuring on your hardware

The admin plane exposes live, real metrics (enable it with `NANO_ADMIN_API_KEY`):

```bash
# Prometheus metrics, including request duration histograms
curl -s -H "X-API-Key: $KEY" http://localhost:8889/metrics

# Live per-isolate used-heap and request counts (real measurements from the
# serving path, not estimates)
curl -s -H "X-API-Key: $KEY" http://localhost:8889/isolates \
  | jq '.isolates[] | {hostname, worker_id, memory_bytes, request_count}'
```

Load-test with any HTTP benchmarking tool (e.g. `wrk`, `oha`, `hey`) against a
representative handler, and read the histograms above for latency distribution.

---

## Troubleshooting

**Rising latency over time.** Often global-state accumulation in the handler or GC
under memory pressure. Keep handler global state small; raise `memory_mb` or add
`workers`.

**Frequent cold starts under load.** Worker pool too small or aggressive eviction.
Increase `workers` and/or `memory_mb`; pre-warm after deploy.

**OOM responses / 503s.** Too many high-`memory_mb` apps for the host, or a handler
leak. Lower `memory_mb`/`workers` per app, or scale horizontally.

---

## References

- [Cold Start Terminology](COLD_START.md)
- [Configuration Reference](CONFIG.md)
- [Slivers](SLIVER_WORKFLOW.md)
- [Roadmap](ROADMAP.md) — heap-snapshot serving and other planned work
