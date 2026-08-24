'use strict';
const fs   = require('fs');
const http = require('http');
const path = require('path');
const { spawn, execFileSync } = require('child_process');

const BINARY = process.env.NANO_BINARY ||
  path.resolve(__dirname, '../../../target/release/nano-rs');

const TMP_DIR = path.join(require('os').tmpdir(), 'nano-suite');
fs.mkdirSync(TMP_DIR, { recursive: true });

// ── HTTP helpers ──────────────────────────────────────────────────────────────

function request(port, pathname, opts = {}) {
  return new Promise((resolve) => {
    const t0 = Date.now();
    const options = {
      hostname: opts.host || 'localhost',
      port,
      path: pathname,
      method: opts.method || 'GET',
      headers: {
        Host: opts.host || 'localhost',
        ...(opts.headers || {}),
      },
      timeout: opts.timeout || 15000,
    };

    const req = http.request(options, (res) => {
      let body = '';
      res.on('data', (c) => (body += c));
      res.on('end', () =>
        resolve({ ok: true, status: res.statusCode, body, latency: Date.now() - t0 })
      );
    });
    req.on('error', (e) =>
      resolve({ ok: false, status: 0, body: '', error: e.message, latency: Date.now() - t0 })
    );
    req.on('timeout', () => {
      req.destroy();
      resolve({ ok: false, status: 0, body: '', error: 'timeout', latency: Date.now() - t0 });
    });
    if (opts.body) req.write(opts.body);
    req.end();
  });
}

function delay(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

async function burst(port, pathname, count, opts = {}) {
  return Promise.all(Array.from({ length: count }, () => request(port, pathname, opts)));
}

// ── Server lifecycle ──────────────────────────────────────────────────────────

async function startServer(appCode, port, opts = {}) {
  const ext   = opts.ext || '.js';
  const app   = path.join(TMP_DIR, `app-${port}${ext}`);
  const cfg   = path.join(TMP_DIR, `cfg-${port}.json`);

  fs.writeFileSync(app, appCode);
  fs.writeFileSync(cfg, JSON.stringify({
    server: { host: '127.0.0.1', port },
    apps: [{
      hostname: 'localhost',
      entrypoint: app,
      limits: { workers: opts.workers || 4, memory_mb: opts.memory_mb || 128, timeout_secs: opts.timeout_secs || 30 },
      ...(opts.env_vars ? { env_vars: opts.env_vars } : {}),
    }],
  }, null, 2));

  const proc = spawn(BINARY, ['run', '-c', cfg], {
    stdio: ['ignore', 'pipe', 'pipe'],
    env: { ...process.env, RUST_LOG: 'error' },
  });

  let stderr = '';
  proc.stderr.on('data', (d) => (stderr += d));
  proc.on('exit', (code) => {
    if (code && code !== 0) process.stderr.write(`[nano port=${port}] exited ${code}: ${stderr}\n`);
  });

  for (let i = 0; i < 60; i++) {
    await delay(200);
    const r = await request(port, '/');
    if (r.status > 0) return { proc, port, app, cfg };
  }
  proc.kill('SIGKILL');
  throw new Error(`Server on port ${port} never became ready. stderr: ${stderr.slice(0, 400)}`);
}

async function stopServer(srv) {
  if (!srv) return;
  try { srv.proc.kill('SIGTERM'); } catch (_) {}
  await delay(800);
  try { srv.proc.kill('SIGKILL'); } catch (_) {}
  try { fs.unlinkSync(srv.app); } catch (_) {}
  try { fs.unlinkSync(srv.cfg); } catch (_) {}
}

// ── Memory measurement ────────────────────────────────────────────────────────

function getRssMb(pid) {
  try {
    const out = execFileSync('ps', ['-o', 'rss=', '-p', String(pid)], { timeout: 2000 })
      .toString().trim();
    return Math.round(parseInt(out, 10) / 1024);
  } catch (_) {
    return null;
  }
}

// ── nano-rs binary info ───────────────────────────────────────────────────────

function nanoVersion() {
  try {
    return execFileSync(BINARY, ['--version'], { timeout: 5000 }).toString().trim();
  } catch (_) {
    return 'unknown';
  }
}

// ── Test assertion helpers ────────────────────────────────────────────────────

function check(name, expr, actual, expected) {
  const passed = expr;
  return { name, passed, actual: String(actual), expected: String(expected) };
}

module.exports = {
  BINARY,
  request,
  burst,
  delay,
  startServer,
  stopServer,
  getRssMb,
  nanoVersion,
  check,
};
