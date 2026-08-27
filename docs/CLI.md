# NANO CLI Reference


---

## Overview

```
nano-rs [OPTIONS] <COMMAND>
```

NANO provides a command-line interface for running JavaScript applications, managing sliver snapshots, and operational tasks.

---

## Global Options

| Option | Short | Description | Default |
|--------|-------|-------------|---------|
| `--verbose` | `-v` | Enable verbose logging | false |
| `--quiet` | `-q` | Suppress non-error output | false |
| `--no-color` | | Disable colored output | false |
| `--help` | `-h` | Print help | |
| `--version` | `-V` | Print version | |

---

## Commands

### nano-rs run

Run JavaScript applications or sliver snapshots.

#### Usage

```bash
# Run with entrypoint
nano-rs run --entrypoint app.js --port 8080

# Run with config file
nano-rs run --config config.json

# Run with sliver snapshot
nano-rs run --sliver app.sliver

# Run with auto-detected entrypoint in directory
nano-rs run ./my-app/
```

#### Options

| Option | Short | Description | Default |
|--------|-------|-------------|---------|
| `--entrypoint` | `-e` | JavaScript entrypoint file | |
| `--port` | `-p` | HTTP server port | 8080 |
| `--host` | `-H` | HTTP server host | 0.0.0.0 |
| `--config` | `-c` | Configuration file path | |
| `--sliver` | `-s` | Sliver snapshot file | |
| `--admin-port` | | Admin API port | 8889 |

#### Examples

**Development server:**
```bash
nano-rs run --entrypoint ./src/index.js --port 3000
```

**Production with config:**
```bash
nano-rs run --config /etc/nano/production.json
```

**From sliver (fast cold start):**
```bash
nano-rs run --sliver ./app.sliver --port 8080
```

**Static site hosting:**
```bash
nano-rs run ./dist/
# Automatically detects index.html and serves static files
```

---

### nano-rs sliver create

Create a sliver snapshot from a running app or directory.

#### Usage

```bash
# Create from running app (requires app to be running)
nano-rs sliver create myapp.example.com --output app.sliver

# Create from directory (recommended for CI/CD)
nano-rs sliver create ./my-app/ --output app.sliver

# Create with verbose output
nano-rs sliver create ./my-app/ --output app.sliver --verbose
```

#### Options

| Option | Short | Description | Required |
|--------|-------|-------------|----------|
| `--output` | `-o` | Output sliver file path | Yes |
| `--verbose` | `-v` | Show detailed progress | No |

#### Examples

**Create from app directory (CI/CD workflow):**
```bash
# Build your app
npm run build

# Create sliver from dist/
nano-rs sliver create ./dist --output myapp.sliver

# Deploy sliver to production
scp myapp.sliver prod-server:/var/nano/
```

**Create with custom name:**
```bash
nano-rs sliver create ./src --output v1.0.0.sliver
```

#### What Gets Packed

The sliver includes:
- All files from the source directory (the app's VFS)
- Optional precompiled V8 bytecode (`bytecode.v8bc`) to skip parse+compile
- Metadata (hostname, created timestamp)

**Exclusions:**
- `node_modules/` (must bundle dependencies)
- `*.log` files
- Hidden files (starting with `.`)

---

### nano-rs sliver list

List available sliver snapshots.

#### Usage

```bash
# List all slivers
nano-rs sliver list

# List with details
nano-rs sliver list --verbose
```

#### Options

| Option | Short | Description |
|--------|-------|-------------|
| `--verbose` | `-v` | Show detailed information (size, created date) |

#### Output

```
NAME          SIZE    CREATED
myapp.sliver  42KB    2026-04-20 14:32:11
api.sliver    156KB   2026-04-19 09:15:23
static.sliver 2.1MB   2026-04-18 22:45:00
```

---

### nano-rs sliver delete

Delete a sliver snapshot.

#### Usage

```bash
# Delete sliver
nano-rs sliver delete myapp.sliver

# Delete without confirmation
nano-rs sliver delete myapp.sliver --force
```

#### Options

| Option | Short | Description |
|--------|-------|-------------|
| `--force` | `-f` | Skip confirmation prompt |

---

### nano-rs sliver inspect

Inspect sliver contents (human-readable metadata).

#### Usage

```bash
nano-rs sliver inspect myapp.sliver
```

#### Output

```
Sliver: myapp.sliver
Created: 2026-04-20 14:32:11
Host: myapp.example.com
V8 Version: 12.4.0
VFS Files: 23
Size: 42KB
Format Version: 1.0
```

#### Low-level Inspection

Use standard tar tools to inspect:

```bash
# List contents
tar -tf myapp.sliver

# Extract metadata
tar -xf myapp.sliver meta.json -O | jq

# Extract specific file
tar -xf myapp.sliver vfs/src/index.js -O
```

---

## Configuration File

See [Configuration Reference](CONFIG.md) for full schema.

**Example `config.json`:**
```json
{
  "server": {
    "host": "0.0.0.0",
    "port": 8080,
    "admin_port": 8889
  },
  "apps": [
    {
      "hostname": "api.example.com",
      "entrypoint": "./api.js",
      "limits": {
        "workers": 4,
        "memory_mb": 128,
        "timeout_secs": 30
      }
    },
    {
      "hostname": "static.example.com",
      "sliver": "./static.sliver",
      "limits": {
        "workers": 2
      }
    }
  ]
}
```

**Run with config:**
```bash
nano-rs run --config production.json
```

---

## Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `NANO_LOG_LEVEL` | Logging level (trace, debug, info, warn, error) | info |
| `NANO_CONFIG` | Default config file path | — |
| `NO_COLOR` | Disable colored output | false |
| `RUST_LOG` | Rust logging filter (debug, trace) | — |
| `AWS_ACCESS_KEY_ID` | S3 backend access key | — |
| `AWS_SECRET_ACCESS_KEY` | S3 backend secret | — |

---

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | General error |
| 2 | Invalid arguments |
| 3 | Config error (invalid JSON, missing fields) |
| 4 | Sliver error (corrupt, incompatible version) |
| 5 | V8 error (initialization, snapshot failure) |
| 130 | Interrupted (Ctrl+C) |

---

## Examples

### Development Workflow

```bash
# 1. Start development server
nano-rs run --entrypoint ./src/index.js --port 3000

# 2. Test your app
curl http://localhost:3000/

# 3. Create sliver for production
nano-rs sliver create ./src --output myapp.sliver

# 4. Test sliver
nano-rs run --sliver myapp.sliver --port 3000

# 5. Deploy
scp myapp.sliver production-server:/var/nano/
```

### Multi-App Hosting

```bash
# Create config with multiple apps
cat > production.json << 'EOF'
{
  "server": { "port": 80 },
  "apps": [
    {
      "hostname": "api.example.com",
      "entrypoint": "./api.js",
      "limits": {
        "workers": 8
      }
    },
    {
      "hostname": "blog.example.com",
      "sliver": "./blog.sliver",
      "limits": {
        "workers": 4
      }
    }
  ]
}
EOF

# Run with config
nano-rs run --config production.json
```

### Static Site Deployment

```bash
# For Next.js static export
next export

# Run the exported site
nano-rs run ./out/

# Or create sliver for deployment
nano-rs sliver create ./out/ --output website.sliver
nano-rs run --sliver website.sliver --port 80
```

### CI/CD Pipeline

```bash
#!/bin/bash
set -e

# Build
npm ci
npm run build

# Test
cargo test

# Create sliver
nano-rs sliver create ./dist --output "app-${VERSION}.sliver"

# Upload to artifact store
aws s3 cp "app-${VERSION}.sliver" s3://nano-artifacts/

# Deploy to staging
ssh staging-server "aws s3 cp s3://nano-artifacts/app-${VERSION}.sliver /var/nano/"
ssh staging-server "systemctl restart nano"
```

---

## Troubleshooting

### Port Already in Use

```
Error: Address already in use (os error 98)
```

**Solution:**
```bash
# Find process using port 8080
lsof -i :8080

# Kill or use different port
nano-rs run --port 8081
```

### Sliver Version Mismatch

```
Warning: Sliver created with V8 12.3.0, running 12.4.0
Recreating isolate from source...
```

**Solution:** Recreate sliver with current V8 version:
```bash
nano-rs sliver create ./src --output app.sliver
```

### Config Validation Error

```
Error: Configuration validation failed
  - apps[0].hostname: required field missing
```

**Solution:** Check config.json against schema. See [CONFIG.md](CONFIG.md).

### High Memory Usage

```bash
# Check isolates via admin API
curl -H "X-API-Key: secret" http://localhost:8889/isolates

# Reduce workers or memory limits in config
```

### Slow Cold Starts

```bash
# Use sliver instead of entrypoint
nano-rs run --sliver app.sliver  # serves from a sliver bundle
# vs
nano-rs run --entrypoint app.js    # 50-100ms
```

---

## See Also

- [Configuration Reference](CONFIG.md) — Full configuration schema
- [Admin API Reference](ADMIN_API.md) — Monitoring endpoints
- [API Reference](API.md) — JavaScript globals available to apps

---

*Last updated: 2026-05-02*
