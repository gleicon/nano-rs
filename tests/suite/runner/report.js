'use strict';
const fs   = require('fs');
const path = require('path');

function badge(passed, total, skipped = 0) {
  const pct = total ? Math.round((passed / total) * 100) : 0;
  if (pct === 100) return `<span class="badge green">${passed}/${total} ✓</span>`;
  if (pct >= 80)   return `<span class="badge yellow">${passed}/${total} ${pct}%</span>`;
  return              `<span class="badge red">${passed}/${total} ${pct}%</span>`;
}

function statusDot(t) {
  if (t.skipped) return '<span class="dot skip" title="skipped">–</span>';
  return t.passed
    ? '<span class="dot pass" title="pass">✓</span>'
    : '<span class="dot fail" title="fail">✗</span>';
}

function esc(s) {
  return String(s)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function renderSuite(suite) {
  const all     = suite.tests;
  const passed  = all.filter((t) => t.passed === true).length;
  const skipped = all.filter((t) => t.skipped).length;
  const total   = all.length;
  const failed  = total - passed - skipped;
  const suitePct = total ? Math.round((passed / (total - skipped)) * 100) : 0;
  const cls = suitePct === 100 ? 'suite-pass' : suitePct >= 80 ? 'suite-warn' : 'suite-fail';

  const rows = all.map((t) => `
    <tr class="${t.passed === true ? 'row-pass' : t.skipped ? 'row-skip' : 'row-fail'}">
      <td>${statusDot(t)}</td>
      <td>${esc(t.name)}</td>
      <td class="mono">${esc(t.got)}</td>
      <td class="mono">${esc(t.expected)}</td>
      <td class="right">${t.latency ? t.latency + ' ms' : '—'}</td>
    </tr>`).join('');

  const memBadge = suite.memMb != null
    ? `<span class="meta">RSS ${suite.memMb} MB</span>` : '';

  return `
  <details class="${cls}" open>
    <summary>
      <span class="suite-name">${esc(suite.name)}</span>
      ${badge(passed, total - skipped, skipped)}
      ${skipped ? `<span class="badge grey">${skipped} skipped</span>` : ''}
      ${memBadge}
      <span class="timing">${suite.durationMs ? suite.durationMs + ' ms' : ''}</span>
    </summary>
    <table>
      <thead><tr><th></th><th>Test</th><th>Got</th><th>Expected</th><th>Latency</th></tr></thead>
      <tbody>${rows}</tbody>
    </table>
  </details>`;
}

function buildHtml(run) {
  const { version, timestamp, suites, durationMs } = run;
  const allTests  = suites.flatMap((s) => s.tests);
  const passed    = allTests.filter((t) => t.passed === true).length;
  const skipped   = allTests.filter((t) => t.skipped).length;
  const total     = allTests.length - skipped;
  const pct       = total ? Math.round((passed / total) * 100) : 0;
  const grade     = pct === 100 ? 'A' : pct >= 90 ? 'B' : pct >= 75 ? 'C' : 'D';
  const gradeClass = pct === 100 ? 'green' : pct >= 90 ? 'blue' : pct >= 75 ? 'yellow' : 'red';

  const summaryRows = suites.map((s) => {
    const sp  = s.tests.filter((t) => t.passed === true).length;
    const sk  = s.tests.filter((t) => t.skipped).length;
    const st  = s.tests.length - sk;
    const spct = st ? Math.round((sp / st) * 100) : 0;
    return `<tr>
      <td>${esc(s.name)}</td>
      <td class="right">${sp}/${st}</td>
      <td><div class="bar-wrap"><div class="bar" style="width:${spct}%;background:${spct === 100 ? '#22c55e' : spct >= 80 ? '#f59e0b' : '#ef4444'}"></div></div></td>
      <td class="right">${spct}%</td>
      <td class="right">${s.memMb != null ? s.memMb + ' MB' : '—'}</td>
      <td class="right">${s.durationMs ? s.durationMs + ' ms' : '—'}</td>
    </tr>`;
  }).join('');

  const suiteHtml = suites.map(renderSuite).join('\n');

  return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>nano-rs Test Report — ${timestamp}</title>
<style>
*{box-sizing:border-box;margin:0;padding:0}
body{font-family:system-ui,sans-serif;background:#0f172a;color:#e2e8f0;padding:24px;line-height:1.5}
h1{font-size:1.8rem;font-weight:700;margin-bottom:4px}
.meta{color:#94a3b8;font-size:.85rem}
.grade{font-size:3rem;font-weight:900;margin:12px 0}
.grade.green{color:#22c55e}.grade.blue{color:#3b82f6}.grade.yellow{color:#f59e0b}.grade.red{color:#ef4444}
.summary{background:#1e293b;border-radius:8px;padding:20px;margin:20px 0}
.summary table{width:100%;border-collapse:collapse;font-size:.9rem}
.summary th{text-align:left;color:#94a3b8;padding:6px 8px;border-bottom:1px solid #334155}
.summary td{padding:6px 8px;border-bottom:1px solid #1e293b}
.right{text-align:right}
.bar-wrap{background:#334155;border-radius:4px;height:8px;width:120px;overflow:hidden}
.bar{height:100%;border-radius:4px;transition:width .3s}
details{background:#1e293b;border-radius:8px;margin:12px 0;overflow:hidden}
summary{padding:14px 18px;cursor:pointer;display:flex;align-items:center;gap:10px;list-style:none;font-weight:600}
summary::-webkit-details-marker{display:none}
.suite-name{flex:1}
.suite-pass summary{border-left:4px solid #22c55e}
.suite-warn summary{border-left:4px solid #f59e0b}
.suite-fail summary{border-left:4px solid #ef4444}
table{width:100%;border-collapse:collapse;font-size:.88rem}
thead th{text-align:left;color:#94a3b8;padding:8px 12px;background:#0f172a;position:sticky;top:0}
tbody td{padding:7px 12px;border-top:1px solid #0f172a;vertical-align:top}
.row-pass{background:#0f2a1a}.row-fail{background:#2a0f0f}.row-skip{background:#1a1a2a}
.dot{font-weight:700;font-size:1rem}.dot.pass{color:#22c55e}.dot.fail{color:#ef4444}.dot.skip{color:#64748b}
.mono{font-family:monospace;font-size:.8rem;max-width:260px;overflow-wrap:anywhere}
.badge{padding:2px 8px;border-radius:12px;font-size:.78rem;font-weight:700}
.badge.green{background:#14532d;color:#86efac}
.badge.yellow{background:#451a03;color:#fde68a}
.badge.red{background:#450a0a;color:#fca5a5}
.badge.grey{background:#1e293b;color:#94a3b8}
.timing{color:#94a3b8;font-size:.8rem;margin-left:auto}
footer{margin-top:32px;color:#475569;font-size:.8rem;text-align:center}
</style>
</head>
<body>
<h1>nano-rs Test Report</h1>
<p class="meta">${esc(version)} · ${esc(timestamp)} · ${durationMs ? durationMs + ' ms total' : ''}</p>
<div class="grade ${gradeClass}">${grade}</div>
<p class="meta">${passed}/${total} tests passed${skipped ? ` · ${skipped} skipped` : ''} · ${pct}%</p>

<div class="summary">
  <table>
    <thead><tr><th>Suite</th><th class="right">Pass/Run</th><th>Progress</th><th class="right">%</th><th class="right">RSS</th><th class="right">Duration</th></tr></thead>
    <tbody>${summaryRows}</tbody>
  </table>
</div>

${suiteHtml}

<footer>Generated by nano-rs test suite · <a href="https://github.com/gleicon/nano-rs" style="color:#60a5fa">github.com/gleicon/nano-rs</a></footer>
</body>
</html>`;
}

function save(run, outDir) {
  const ts   = run.timestamp.replace(/[:.]/g, '-');
  const html = buildHtml(run);
  const name = `report-${ts}.html`;
  const dest = path.join(outDir, name);
  const latest = path.join(outDir, 'latest.html');
  fs.mkdirSync(outDir, { recursive: true });
  fs.writeFileSync(dest, html);
  fs.writeFileSync(latest, html);
  return { dest, latest };
}

module.exports = { buildHtml, save };
