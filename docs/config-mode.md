# Config Mode

NANO supports running multiple applications from a JSON configuration file, enabling multi-tenant hosting with per-app resource limits and virtual host routing.

## Usage

```bash
nano-rs run --config apps.json
```

## Configuration Format

```json
{
  "apps": [
    {
      "hostname": "api.example.com",
      "sliver": "/path/to/api.sliver",
      "env_vars": {"API_KEY": "secret"},
      "limits": {
        "memory_mb": 256,
        "timeout_secs": 60,
        "workers": 8
      }
    },
    {
      "hostname": "blog.example.com",
      "entrypoint": "/path/to/blog.js",
      "env_vars": {"DB_URL": "localhost"},
      "limits": {
        "memory_mb": 128,
        "timeout_secs": 30,
        "workers": 4
      }
    }
  ],
  "server": {
    "port": 8080,
    "host": "0.0.0.0"
  }
}
```

## App Configuration

Each app in the `apps` array defines a hosted application:

### Required Fields

- **hostname** (string): Domain name this app responds to. Used for virtual host routing.

### App Type Fields (one required)

- **sliver** (string, optional): Path to a sliver bundle. The app loads from the bundle's VFS, using precompiled bytecode when its V8 version matches.
- **entrypoint** (string, optional): Path to JavaScript entrypoint file. Creates fresh isolates per request.

### Optional Fields

- **env_vars** (object): Environment variables injected into JS global scope. Key-value pairs of strings.
- **limits** (object): Resource limits for this app:
  - **memory_mb**: Maximum memory in MB (16-2048, default: 128)
  - **timeout_secs**: Request timeout in seconds (1-300, default: 30)
  - **workers**: Number of worker threads (1-32, default: 4)
  - **cpu_time_ms**: CPU time limit per request in milliseconds (1-1000, default: 50)
  - **cpu_time_enabled**: Enable CPU time tracking and termination (default: true)

## Server Configuration

Global server settings control the HTTP listener:

- **port** (number): Port to bind (1-65535, default: 8080)
- **host** (string): Bind address (default: "0.0.0.0")
  - Use "0.0.0.0" to listen on all interfaces
  - Use "127.0.0.1" for localhost only
  - Use specific IP for interface binding

## Virtual Host Routing

Requests are routed based on the HTTP `Host` header:

```
Host: api.example.com     → routes to API app
Host: blog.example.com    → routes to Blog app
Host: unknown.com         → 404 Not Found
```

## Per-App Resource Limits

Each app operates within its configured limits:

### Memory Limits
- Sliver-based apps: Enforced via OOM monitor per worker isolate
- Entrypoint apps: Inherits global worker pool memory settings
- Soft eviction at 85% memory usage (allows current requests to complete)
- Hard eviction at 95% memory usage (immediate termination)

### CPU Time Limits
- Per-request CPU time tracking with microsecond precision
- Cloudflare-style 50ms default limit (configurable 1-1000ms)
- Timer-based termination using Linux timer_create syscall
- V8 TerminateExecution called on CPU timeout
- Distinguishes CPU timeout from wall-clock timeout

### Worker Count
- Creates N dedicated worker threads per app
- Workers handle requests in parallel with context reset between requests
- LRU eviction prefers stateless isolates when memory pressure detected

### Timeout
- Configured timeout is validated at startup
- Per-request timeout enforcement uses Tower middleware (30s default)
- Separate from CPU time limits (wall-clock vs CPU time)

## Example Configurations

### Single App with Sliver

```json
{
  "apps": [
    {
      "hostname": "app.example.com",
      "sliver": "./app.sliver",
      "limits": {"memory_mb": 256, "workers": 8}
    }
  ]
}
```

### Multiple Apps with Mixed Types

```json
{
  "apps": [
    {
      "hostname": "api.example.com",
      "sliver": "/apps/api.sliver",
      "env_vars": {"API_VERSION": "v1"},
      "limits": {"memory_mb": 512, "timeout_secs": 60, "workers": 16}
    },
    {
      "hostname": "static.example.com",
      "entrypoint": "/apps/static.js",
      "limits": {"memory_mb": 64, "timeout_secs": 10, "workers": 2}
    }
  ],
  "server": {"port": 3000, "host": "127.0.0.1"}
}
```

### Development Configuration

```json
{
  "apps": [
    {
      "hostname": "localhost",
      "entrypoint": "./index.js",
      "env_vars": {"DEBUG": "true"},
      "limits": {"memory_mb": 128, "workers": 4}
    }
  ],
  "server": {"port": 8080, "host": "127.0.0.1"}
}
```

## Validation

Configuration is validated at load time:

- Hostname format (DNS-compatible)
- Port range (1-65535)
- Memory limits (16-2048 MB)
- Timeout range (1-300 seconds)
- Worker count (1-32)
- Unique hostnames (case-insensitive)
- Environment variable keys (no suspicious patterns)
- Path traversal prevention in entrypoint/sliver paths

## CLI Integration

The `--config` flag integrates with other run modes:

```bash
# Config mode (multi-app)
nano-rs run --config apps.json

# Sliver mode (single app from snapshot)
nano-rs run --sliver app.sliver

# Default mode (no args, basic server)
nano-rs run
```

## Migration from Single-App

To migrate from single-app hosting to config mode:

1. Create a JSON config file with your existing app
2. Move per-app settings (hostname, limits) into the config
3. Run with `--config` instead of `--sliver` or default mode

Example migration:

```bash
# Before
nano-rs run --sliver app.sliver

# After (config.json)
{
  "apps": [{"hostname": "app.example.com", "sliver": "./app.sliver"}]
}
nano-rs run --config config.json
```

## Environment Variables

NANO can load configuration from environment variables in addition to the JSON file:

- `NANO_PORT`: Override server port
- `NANO_HOST`: Override server host
- `NANO_ADMIN_API_KEY`: Enable admin API server
- `NANO_ADMIN_UNIX_SOCKET`: Enable Unix socket admin

## Security Considerations

- Each app gets dedicated worker pool (no cross-app sharing)
- Memory limits prevent resource exhaustion
- Path validation prevents directory traversal attacks
- Environment variables are explicitly configured per-app
- Generic error messages to clients; detailed errors in logs

## Troubleshooting

### Config fails to load

```bash
# Check JSON validity
jq . config.json

# Validate with NANO
cargo run -- run --config config.json 2>&1 | head -20
```

### Port already in use

```bash
# Check what's using the port
lsof -i :8080

# Use different port in config
# "server": {"port": 8081}
```

### Hostname not routing

- Verify `Host` header matches configured hostname exactly (case-insensitive)
- Check for typos in hostname configuration
- Ensure DNS or /etc/hosts resolves to server IP

## See Also

- `docs/SLIVER_WORKFLOW.md` - Creating and managing sliver files
- `README.md` - Quick start guide
