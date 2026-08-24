# NANO Configuration Reference

**Version:** 2.3.0  
**Last Updated:** 2026-08-08

---

## Overview

NANO uses JSON configuration files for multi-app hosting and runtime settings.

**Configuration sources (in order of precedence):**
1. CLI flags (`--port`, `--host`, etc.)
2. Environment variables (`NANO_CONFIG`)
3. Config file (`--config` or default path)
4. Built-in defaults

---

## Quick Start

### Single App

```json
{
  "server": {
    "port": 3000
  },
  "apps": [
    {
      "hostname": "localhost",
      "entrypoint": "./src/index.js",
      "limits": {
        "workers": 2,
        "memory_mb": 64
      }
    }
  ]
}
```

**Run:**
```bash
nano-rs run --config dev.json
```

---

## Schema

### Full Schema

```json
{
  "server": {
    "host": "127.0.0.1",
    "port": 8080,
    "admin_port": 8889,
    "admin_api_key": "your-secret-key-min-32-chars",
    "admin_unix_socket": "/tmp/nano-admin.sock"
  },
  "logging": {
    "level": "info",
    "format": "json",
    "output": "stdout"
  },
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
      "env_vars": {}
    }
  ],
  "vfs": {
    "backend": "memory",
    "disk_path": null,
    "s3_bucket": null,
    "max_file_size_mb": 10,
    "max_total_size_mb": 100
  }
}
```

---

## Server Section

### host
- **Type:** string
- **Default:** `"127.0.0.1"` (changed from `"0.0.0.0"` in v2.3.0)
- **Description:** HTTP server bind address
- **Examples:**
  - `"127.0.0.1"` — localhost only (default; safe for proxied deployments)
  - `"0.0.0.0"` — all interfaces (set explicitly for direct public access)
  - `"192.168.1.100"` — specific interface
- **CLI override:** `--host`

### port
- **Type:** number
- **Default:** `8080`
- **Description:** HTTP server port
- **Examples:** `80`, `443`, `3000`, `8080`
- **CLI override:** `--port`

### admin_port
- **Type:** number
- **Default:** `8889`
- **Description:** Admin API server port
- **Examples:** `8889`, `9090`

### admin_api_key
- **Type:** string
- **Default:** none (required)
- **Description:** API key for admin HTTP endpoints
- **Important:** If not set, all admin API requests are denied with 401 (changed in v2.3.0 — previously allowed all requests)
- **Security:** Use strong random key; minimum 32 characters recommended (`openssl rand -hex 32`)
- **Example:** `"sk_live_abc123xyz789"`

### admin_unix_socket
- **Type:** string | null
- **Default:** `"/tmp/nano-admin.sock"`
- **Description:** Unix domain socket path for admin API (Unix only)
- **Note:** Unix socket access bypasses API key authentication
- **Disable:** Set to `null` to disable

---

## Logging Section

### level
- **Type:** string
- **Default:** `"info"`
- **Options:** `"trace"`, `"debug"`, `"info"`, `"warn"`, `"error"`
- **Description:** Logging verbosity level
- **Recommendation:**
  - Development: `"debug"`
  - Production: `"info"` or `"warn"`

### format
- **Type:** string
- **Default:** `"json"`
- **Options:** `"json"`, `"text"`
- **Description:** Log output format
- **Recommendation:**
  - Use `"json"` for production (structured logging, easier parsing)
  - Use `"text"` for development (human-readable)

**JSON format:**
```json
{"level":"info","message":"Server started","timestamp":"2026-05-02T10:30:00Z","port":8080}
```

**Text format:**
```
[2026-05-02T10:30:00Z] INFO: Server started on port 8080
```

### output
- **Type:** string
- **Default:** `"stdout"`
- **Options:** `"stdout"`, `"stderr"`, `"/path/to/file"`
- **Description:** Log destination

---

## Apps Array

Each app object configures one hosted application.

### hostname (required)
- **Type:** string
- **Description:** Virtual host for routing (matched against Host header)
- **Examples:**
  - `"api.example.com"`
  - `"*.example.com"` (wildcard)
  - `"localhost"`
- **Routing:** Incoming requests with matching Host header are routed to this app

### entrypoint
- **Type:** string | null
- **Description:** Path to JavaScript entrypoint file
- **Examples:**
  - `"./src/index.js"`
  - `"/apps/api/main.js"`
- **Note:** Either `entrypoint` or `sliver` must be specified (not both)

### sliver
- **Type:** string | null
- **Description:** Path to sliver snapshot file
- **Examples:**
  - `"./app.sliver"`
  - `"/snapshots/api-v1.sliver"`
- **Note:** Either `entrypoint` or `sliver` must be specified (not both)
- **Benefit:** first request skips JavaScript parse+compile via precompiled bytecode

### limits
- **Type:** object
- **Default:** See `limits` defaults below
- **Description:** Resource limits for this app

#### limits.workers
- **Type:** number
- **Default:** `4`
- **Description:** Number of worker threads for this app
- **Examples:** `1`, `4`, `16`

#### limits.memory_mb
- **Type:** number
- **Default:** `128`
- **Description:** Per-isolate memory limit in megabytes
- **Examples:** `64`, `128`, `256`, `512`

#### limits.timeout_secs
- **Type:** number
- **Default:** `30` (30 seconds)
- **Description:** Per-request timeout in seconds
- **Examples:** `5`, `30`, `60`

#### limits.cpu_time_ms
- **Type:** number
- **Default:** `50` (Cloudflare-style)
- **Description:** Per-request CPU time limit in milliseconds
- **Examples:** `50`, `100`, `500`

### env_vars
- **Type:** object
- **Default:** `{}`
- **Description:** Environment variables injected into the isolate
- **Note:** Accessible via global scope in JavaScript

---

## VFS Section

Virtual File System configuration.

### backend
- **Type:** string
- **Default:** `"memory"`
- **Options:** `"memory"`, `"disk"`, `"s3"`
- **Description:** VFS storage backend type

| Backend | Use Case | Persistence | Latency |
|---------|----------|-------------|---------|
| `memory` | Fast, ephemeral | No | <1µs |
| `disk` | Persistence | Yes | ~1ms |
| `s3` | Cloud, scalable | Yes | ~50ms |

### disk_path
- **Type:** string | null
- **Default:** `null`
- **Description:** Path for disk backend storage
- **Examples:**
  - `"/var/lib/nano/vfs"`
  - `"./data"`
- **Required when:** `backend: "disk"`

### s3_bucket
- **Type:** string | null
- **Default:** `null`
- **Description:** S3 bucket name for S3 backend
- **Required when:** `backend: "s3"`
- **Credentials:** Set `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`

### max_file_size_mb
- **Type:** number
- **Default:** `10`
- **Description:** Maximum single file size in megabytes
- **Examples:** `1`, `10`, `100`

### max_total_size_mb
- **Type:** number
- **Default:** `100`
- **Description:** Maximum total VFS size per isolate in megabytes
- **Examples:** `50`, `100`, `500`

---

## Examples

### Single App Development

```json
{
  "server": {
    "port": 3000
  },
  "apps": [
    {
      "hostname": "localhost",
      "entrypoint": "./src/index.js",
      "limits": {
        "workers": 2,
        "memory_mb": 64
      }
    }
  ]
}
```

**Run:**
```bash
nano-rs run --config dev.json
```

---

### Multi-App Production

```json
{
  "server": {
    "host": "0.0.0.0",
    "port": 80,
    "admin_port": 8889,
    "admin_api_key": "prod-secret-key-change-me"
  },
  "logging": {
    "level": "warn",
    "format": "json"
  },
  "apps": [
    {
      "hostname": "api.example.com",
      "entrypoint": "./api.js",
      "limits": {
        "workers": 8,
        "memory_mb": 256,
        "timeout_secs": 30,
        "cpu_time_ms": 50
      }
    },
    {
      "hostname": "blog.example.com",
      "sliver": "./blog.sliver",
      "limits": {
        "workers": 4,
        "memory_mb": 128
      }
    },
    {
      "hostname": "static.example.com",
      "entrypoint": "./static/index.html",
      "limits": {
        "workers": 2,
        "memory_mb": 64
      }
    }
  ],
  "vfs": {
    "backend": "memory",
    "max_file_size_mb": 50,
    "max_total_size_mb": 500
  }
}
```

---

### With Disk Persistence

```json
{
  "server": {
    "port": 8080
  },
  "apps": [
    {
      "hostname": "app.example.com",
      "entrypoint": "./app.js",
      "limits": {
        "workers": 4
      }
    }
  ],
  "vfs": {
    "backend": "disk",
    "disk_path": "/var/lib/nano/data",
    "max_file_size_mb": 100,
    "max_total_size_mb": 1000
  }
}
```

**Setup:**
```bash
mkdir -p /var/lib/nano/data
nano-rs run --config production.json
```

---

### With S3 Backend (Production Multi-Instance)

```json
{
  "server": {
    "port": 8080
  },
  "vfs": {
    "backend": "s3",
    "s3_bucket": "nano-vfs-prod",
    "max_file_size_mb": 50,
    "max_total_size_mb": 1000
  }
}
```

**Environment:**
```bash
export AWS_ACCESS_KEY_ID="your-key"
export AWS_SECRET_ACCESS_KEY="your-secret"
nano-rs run --config production.json
```

---

### High-Traffic API

```json
{
  "server": {
    "port": 80,
    "admin_api_key": "prod-secret"
  },
  "logging": {
    "level": "warn",
    "format": "json"
  },
  "apps": [
    {
      "hostname": "api.example.com",
      "sliver": "./api.sliver",
      "limits": {
        "workers": 16,
        "memory_mb": 256,
        "timeout_secs": 5,
        "cpu_time_ms": 100
      }
    }
  ]
}
```

---

## Validation

NANO validates configuration on startup and reports errors:

```bash
$ nano-rs run --config invalid.json
Error: Configuration validation failed
  - apps[0].hostname: required field missing
  - server.port: invalid type (expected number, got string)
```

### Common Validation Errors

| Error | Cause | Fix |
|-------|-------|-----|
| `hostname: required` | Missing hostname field | Add `"hostname": "example.com"` |
| `entrypoint or sliver required` | Neither specified | Add one or the other |
| `both entrypoint and sliver` | Both specified | Remove one |
| `disk_path required` | Disk backend without path | Add `disk_path` |
| `port: invalid type` | Port as string | Use number: `8080` not `"8080"` |

---

## Hot Reload

Configuration changes are detected and applied without restart:

1. Edit `config.json`
2. Save file
3. NANO detects change (within 5 seconds)
4. Graceful drain: existing requests complete
5. New configuration applied
6. New requests use updated config

**Changes requiring full restart:**
- Server host/port changes
- VFS backend changes (memory → disk)
- Admin port changes
- TLS certificate changes (if implemented)

**Changes applied via hot reload:**
- App entrypoint/sliver
- Worker count
- Memory limits
- Timeout/CPU limits
- New apps added/removed

---

## Environment Variables

Override config with environment:

| Variable | Effect |
|----------|--------|
| `NANO_CONFIG` | Default config file path |
| `NANO_LOG_LEVEL` | Override logging.level |
| `NO_COLOR` | Disable colored output |

---

## CLI Precedence

CLI flags override config file:

```bash
# Config says port 8080, CLI overrides to 3000
nano-rs run --config production.json --port 3000
```

**Priority:**
1. CLI flags (highest)
2. Environment variables
3. Config file
4. Built-in defaults (lowest)

---

## Security Best Practices

1. **Admin API Key**
   ```json
   {
     "server": {
       "admin_api_key": "use-random-64-char-string"
     }
   }
   ```
   Generate: `openssl rand -base64 48`

2. **Unix Socket Permissions**
   ```bash
   chmod 600 /tmp/nano-admin.sock
   ```

3. **VFS Limits**
   ```json
   {
     "vfs": {
       "max_file_size_mb": 10,
       "max_total_size_mb": 100
     }
   }
   ```

4. **CPU/Memory Limits**
   ```json
   {
     "apps": [{
       "limits": {
         "memory_mb": 128,
         "cpu_time_ms": 50,
         "timeout_secs": 30
       }
     }]
   }
   ```

---

## See Also

- [CLI Reference](CLI.md) — Command-line options
- [Admin API](ADMIN_API.md) — Admin endpoints for monitoring
- [API Reference](API.md) — JavaScript APIs available to apps

---

*Last updated: 2026-05-02*
