#!/usr/bin/env node
'use strict';
const fs   = require('fs');
const path = require('path');

const utils  = require('./utils');
const report = require('./report');

const SUITES_DIR = path.join(__dirname, 'suites');
const REPORTS_DIR = process.env.REPORTS_DIR ||
  path.resolve(__dirname, '../../../reports/suite');

// Ordered list: fast suites first, slow (concurrency, memory) last
const SUITE_NAMES = [
  'foundation',
  'wintertc',
  'node-compat',
  'kv',
  'crypto',
  'wasm',
  'gas',
  'concurrency',
  'memory',
];

// Filter by --suite=name,name or NANO_SUITE env var
function selectedSuites() {
  const arg = process.argv.find((a) => a.startsWith('--suite='));
  const env = process.env.NANO_SUITE;
  const raw = arg ? arg.slice('--suite='.length) : env;
  if (!raw) return SUITE_NAMES;
  return raw.split(',').map((s) => s.trim()).filter(Boolean);
}

async function runSuite(name, ctx) {
  const fn = require(path.join(SUITES_DIR, name));
  const t0 = Date.now();
  let result;
  try {
    result = await fn(ctx);
  } catch (err) {
    result = {
      name,
      tests: [{ name: 'suite runner', passed: false, got: err.message, expected: 'no error', latency: 0 }],
      memMb: null,
    };
  }
  result.durationMs = Date.now() - t0;
  return result;
}

async function main() {
  const version = utils.nanoVersion();
  const timestamp = new Date().toISOString();

  console.log('\n╔══════════════════════════════════╗');
  console.log('║   nano-rs integration test suite  ║');
  console.log('╚══════════════════════════════════╝');
  console.log(`\nBinary : ${utils.BINARY}`);
  console.log(`Version: ${version}`);
  console.log(`Date   : ${timestamp}\n`);

  const ctx = {
    startServer:  utils.startServer,
    stopServer:   utils.stopServer,
    request:      utils.request,
    burst:        utils.burst,
    delay:        utils.delay,
    getRssMb:     utils.getRssMb,
  };

  const names   = selectedSuites();
  const suites  = [];
  const overall = Date.now();

  for (const name of names) {
    if (!fs.existsSync(path.join(SUITES_DIR, name + '.js'))) {
      console.warn(`  WARN: suite '${name}' not found, skipping`);
      continue;
    }

    process.stdout.write(`  ${name.padEnd(16)}`);
    const result = await runSuite(name, ctx);
    suites.push(result);

    const passed  = result.tests.filter((t) => t.passed === true).length;
    const skipped = result.tests.filter((t) => t.skipped).length;
    const total   = result.tests.length - skipped;
    const pct     = total ? Math.round((passed / total) * 100) : 0;
    const icon    = pct === 100 ? '✓' : pct >= 80 ? '~' : '✗';

    console.log(`${icon}  ${passed}/${total} (${pct}%)  ${result.durationMs}ms`);

    // Print failures for immediate feedback
    result.tests
      .filter((t) => t.passed === false)
      .forEach((t) => console.log(`     FAIL: ${t.name} → got "${t.got}" expected "${t.expected}"`));
  }

  const durationMs = Date.now() - overall;
  const run = { version, timestamp, suites, durationMs };

  // Summary line
  const allTests = suites.flatMap((s) => s.tests);
  const p = allTests.filter((t) => t.passed === true).length;
  const sk = allTests.filter((t) => t.skipped).length;
  const tot = allTests.length - sk;
  const pct = tot ? Math.round((p / tot) * 100) : 0;
  console.log(`\n${'─'.repeat(40)}`);
  console.log(`Total: ${p}/${tot} passed (${pct}%)  ${durationMs}ms\n`);

  // Save HTML report
  const { dest, latest } = report.save(run, REPORTS_DIR);
  console.log(`Report: ${dest}`);
  console.log(`Latest: ${latest}\n`);

  // Exit non-zero if any non-skipped test failed
  const hasFail = allTests.some((t) => t.passed === false);
  process.exit(hasFail ? 1 : 0);
}

main().catch((err) => {
  console.error('Suite runner fatal error:', err);
  process.exit(2);
});
