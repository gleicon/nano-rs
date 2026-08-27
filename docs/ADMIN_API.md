# NANO Admin API Reference

**Base URL:** `http://localhost:8889` (default)  
**Authentication:** API Key (X-API-Key header)

---

## Overview

The Admin API provides operational visibility and control for running NANO instances. Use it for:
- Monitoring health and metrics
- Viewing running isolates and apps
- Getting diagnostic information
- Prometheus metrics export

**Default endpoints:**
- HTTP: `http://localhost:8889`
- Unix Socket: `/tmp/nano-admin.sock` (Unix only)

---

## Authentication

All HTTP endpoints require an API key:

```bash
curl -H "X-API-Key: your-api-key" http://localhost:8889/health
```

API keys are configured in the config file:
```json
{
  "server": {
    "admin_api_key": "your-secret-key"
  }
}
```

**Unix sockets bypass authentication** for local emergency access.

---

## Endpoints

### GET /health

Liveness probe — always returns 200 while the process is running. For
shutdown-aware draining use `GET /admin/ready` instead.

**Request:**
```bash
curl http://localhost:8889/admin/health
```

**Response (200 OK):**
```json
{
  "status": "healthy",
  "version": "2.6.0",
  "service": "nano-admin"
}
```

---

### GET /apps

List all configured applications.

**Request:**
```bash
curl -H "X-API-Key: secret" http://localhost:8889/apps
```

**Response:**
```json
{
  "apps": [
    {
      "hostname": "api.example.com",
      "entrypoint": "./api.js",
      "sliver": null,
      "limits": {
        "workers": 4,
        "memory_mb": 128,
        "timeout_secs": 30,
        "cpu_time_ms": 50
      },
      "requests_total": 15234,
      "errors_total": 12
    },
    {
      "hostname": "blog.example.com",
      "entrypoint": null,
      "sliver": "./blog.sliver",
      "limits": {
        "workers": 2
      },
      "requests_total": 8901,
      "errors_total": 0
    }
  ]
}
```

**Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `hostname` | string | Virtual host for routing |
| `entrypoint` | string \| null | JS entrypoint path |
| `sliver` | string \| null | Sliver file path |
| `limits.workers` | number | Number of worker threads |
| `limits.memory_mb` | number | Per-isolate memory limit (MB) |
| `limits.timeout_secs` | number | Request timeout (seconds) |
| `limits.cpu_time_ms` | number | CPU time limit (milliseconds) |
| `requests_total` | number | Total requests served |
| `errors_total` | number | Total errors (4xx, 5xx, timeouts) |

---

### GET /apps/{hostname}

Get details for a specific app.

**Request:**
```bash
curl -H "X-API-Key: secret" http://localhost:8889/apps/api.example.com
```

**Response:** the app's configured details (from the registry).
```json
{
  "hostname": "api.example.com",
  "entrypoint": "./api.js",
  "env_vars": { "API_KEY": "…" },
  "limits": {
    "workers": 4,
    "memory_mb": 128,
    "timeout_secs": 30,
    "cpu_time_ms": 50
  },
  "status": "active",
  "created_at": "unknown",
  "is_active": true
}
```

For live per-isolate runtime stats (request counts, memory, busy) use
`GET /isolates`.

---

### GET /isolates

List all live V8 isolates across all apps, with real per-isolate telemetry
published by the worker threads: request count, busy state, used-heap bytes,
creation time, hostname, worker id, and env-var keys.

> **Note:** isolates are created lazily on the first request, so an app that
> has received no traffic reports zero isolates. `memory_bytes` is `null` until
> that isolate has served at least one request. For aggregate request/latency
> counters use `GET /admin/metrics`.

**Request:**
```bash
curl -H "X-API-Key: secret" http://localhost:8889/isolates
```

**Response:**
```json
{
  "total_isolates": 2,
  "total_requests": 6046,
  "app_count": 2,
  "isolates": [
    {
      "hostname": "api.example.com",
      "worker_id": 0,
      "created_at": "2026-04-20T14:32:11Z",
      "uptime": "1h 13m",
      "request_count": 3812,
      "memory_bytes": 47185920,
      "busy": false,
      "env_keys": ["API_KEY"]
    },
    {
      "hostname": "blog.example.com",
      "worker_id": 0,
      "created_at": "2026-04-20T14:30:45Z",
      "uptime": "1h 15m",
      "request_count": 2234,
      "memory_bytes": 93323264,
      "busy": true,
      "env_keys": []
    }
  ],
  "apps": [
    { "hostname": "api.example.com", "worker_count": 1, "total_requests": 3812 }
  ],
  "timestamp": "2026-04-20T15:45:22Z"
}
```

---

### GET /metrics

Prometheus-compatible metrics export.

**Request:**
```bash
curl -H "X-API-Key: secret" http://localhost:8889/metrics
```

**Response (text/plain):**
```
# HELP nano_requests_total Total requests served
# TYPE nano_requests_total counter
nano_requests_total{hostname="api.example.com"} 15234
nano_requests_total{hostname="blog.example.com"} 8901

# HELP nano_request_duration_seconds Request duration
# TYPE nano_request_duration_seconds histogram
nano_request_duration_seconds_bucket{hostname="api.example.com",le="0.005"} 14500
nano_request_duration_seconds_bucket{hostname="api.example.com",le="0.01"} 14900
nano_request_duration_seconds_bucket{hostname="api.example.com",le="0.025"} 15200
nano_request_duration_seconds_bucket{hostname="api.example.com",le="+Inf"} 15234
nano_request_duration_seconds_sum{hostname="api.example.com"} 45.6
nano_request_duration_seconds_count{hostname="api.example.com"} 15234

# HELP nano_memory_usage_bytes Memory usage by isolate
# TYPE nano_memory_usage_bytes gauge
nano_memory_usage_bytes{isolate="iso-1",hostname="api.example.com"} 47185920
nano_memory_usage_bytes{isolate="iso-2",hostname="api.example.com"} 70254592

# HELP nano_cpu_time_seconds Total CPU time consumed
# TYPE nano_cpu_time_seconds counter
nano_cpu_time_seconds{hostname="api.example.com"} 12.34

# HELP nano_active_isolates Number of active isolates
# TYPE nano_active_isolates gauge
nano_active_isolates{hostname="api.example.com"} 4

# HELP nano_cpu_limit_violations_total Total CPU limit violations
# TYPE nano_cpu_limit_violations_total counter
nano_cpu_limit_violations_total{hostname="api.example.com"} 23

# HELP nano_memory_evictions_total Total memory evictions
# TYPE nano_memory_evictions_total counter
nano_memory_evictions_total{hostname="api.example.com"} 5
```

**Available Metrics:**

| Metric | Type | Description |
|--------|------|-------------|
| `nano_requests_total` | counter | Total requests |
| `nano_request_duration_seconds` | histogram | Request latency |
| `nano_memory_usage_bytes` | gauge | Per-isolate memory |
| `nano_cpu_time_seconds` | counter | Per-app CPU time |
| `nano_active_isolates` | gauge | Current isolates |
| `nano_errors_total` | counter | Error count |
| `nano_cpu_limit_violations_total` | counter | CPU violations |
| `nano_memory_evictions_total` | counter | Memory evictions |
| `nano_wasm_compilations_total` | counter | WASM compilations |

---

### GET /diagnostics

Get detailed diagnostics (ps-style output).

**Request:**
```bash
curl -H "X-API-Key: secret" http://localhost:8889/diagnostics
```

**Response (text/plain):**
```
ISOLATE   HOSTNAME            STATUS  MEM(MB)  CPU(MS)  REQ/S  UP
--------  ------------------  ------  -------  -------  -----  ----------
iso-1     api.example.com     idle    45.2     1205     1.2    1h23m
iso-2     api.example.com     busy    67.4     2341     3.5    1h23m
iso-3     api.example.com     idle    42.1     982      0.8    1h22m
iso-4     api.example.com     idle    38.7     876      0.7    1h22m
iso-5     blog.example.com    busy    89.3     5432     8.9    2h15m
iso-6     blog.example.com    idle    34.2     1234     2.1    2h14m
```

---

## Error Responses

### 401 Unauthorized

```json
{
  "error": "Invalid or missing API key"
}
```

**Resolution:** Provide valid X-API-Key header.

### 404 Not Found

```json
{
  "error": "App or isolate not found"
}
```

### 503 Service Unavailable

```json
{
  "error": "Server is not ready"
}
```

**Resolution:** Wait for startup to complete or check logs.

---

## Examples

### Health Check with Retry

```bash
#!/bin/bash
until curl -sf -H "X-API-Key: secret" http://localhost:8889/health; do
  echo "Waiting for NANO to be ready..."
  sleep 1
done
echo "NANO is healthy!"
```

### Prometheus Scraping

```bash
# Direct scrape
curl -H "X-API-Key: secret" http://localhost:8889/metrics

# Save to file for debugging
curl -H "X-API-Key: secret" http://localhost:8889/metrics > nano-metrics.txt

# Parse specific metric
curl -s -H "X-API-Key: secret" http://localhost:8889/metrics | \
  grep "nano_requests_total" | \
  awk '{print $2}'
```

### List Apps and Their Status

```bash
# Pretty print with jq
curl -s -H "X-API-Key: secret" http://localhost:8889/apps | \
  jq '.apps[] | {hostname: .hostname, workers: .limits.workers, requests: .requests_total, errors: .errors_total}'
```

**Output:**
```json
{
  "hostname": "api.example.com",
  "workers": 4,
  "requests": 15234,
  "errors": 12
}
{
  "hostname": "blog.example.com",
  "workers": 2,
  "requests": 8901,
  "errors": 0
}
```

### Monitor Isolate Health

```bash
# Live per-isolate used-heap and request counts
watch -n 1 'curl -s -H "X-API-Key: secret" http://localhost:8889/isolates | jq ".isolates[] | {hostname, worker_id, busy, requests: .request_count, memory: .memory_bytes}"'

# Aggregate request/latency counters
curl -s http://localhost:8889/admin/metrics | grep nano_
```

### CPU Time Monitoring

```bash
# Check for CPU limit violations
curl -s -H "X-API-Key: secret" http://localhost:8889/metrics | \
  grep "nano_cpu_limit_violations_total"
```

---

## Unix Domain Socket

On Unix systems, the admin API is also available via Unix domain socket for local access. No API key is required — access is controlled entirely by filesystem permissions on the socket file (mode 0660, owner+group):

```bash
# Default socket path
curl --unix-socket /tmp/nano-admin.sock http://localhost/health

# Get metrics via socket
curl --unix-socket /tmp/nano-admin.sock http://localhost/metrics

# Diagnostics
curl --unix-socket /tmp/nano-admin.sock http://localhost/diagnostics
```

### Socket Path Configuration

```json
{
  "server": {
    "admin_unix_socket": "/var/run/nano/admin.sock"
  }
}
```

---

## Prometheus Integration

### Scraping Configuration

```yaml
# prometheus.yml
scrape_configs:
  - job_name: 'nano'
    static_configs:
      - targets: ['localhost:8889']
    metrics_path: /metrics
    bearer_token: 'your-api-key'
```

### Grafana Dashboard

Key panels for NANO monitoring:

1. **Request Rate** — `rate(nano_requests_total[5m])`
2. **Error Rate** — `rate(nano_errors_total[5m])`
3. **P95 Latency** — `histogram_quantile(0.95, nano_request_duration_seconds_bucket)`
4. **Memory Usage** — `nano_memory_usage_bytes`
5. **CPU Time** — `nano_cpu_time_seconds`
6. **Active Isolates** — `nano_active_isolates`
7. **CPU Violations** — `rate(nano_cpu_limit_violations_total[5m])`
8. **Memory Evictions** — `rate(nano_memory_evictions_total[5m])`

---

## See Also

- [Configuration Reference](CONFIG.md) — Admin API configuration options
- [CLI Reference](CLI.md) — Command-line interface
- [API Reference](API.md) — JavaScript APIs available to apps

---

*Last updated: 2026-05-02*
