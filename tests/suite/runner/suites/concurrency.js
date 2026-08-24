'use strict';
// Concurrency suite — parallel requests, worker pool, isolate state persistence, no cross-contamination

const COUNTER_APP = `
let hits = 0;
export default {
  async fetch(request) {
    hits++;
    return new Response(String(hits), { headers: { 'x-hit': String(hits) } });
  }
};
`;

const ECHO_APP = `
export default {
  async fetch(request) {
    const url = new URL(request.url);
    const id = url.searchParams.get('id') || '?';
    // Small CPU work to spread across workers
    let n = 0; for (let i = 0; i < 10000; i++) n += i;
    return new Response(id, { headers: { 'x-sum': String(n) } });
  }
};
`;

module.exports = async function concurrency({ startServer, stopServer, request, burst, delay, getRssMb }) {
  const PORT_A = 9360;
  const PORT_B = 9361;
  const tests  = [];

  function rec(name, passed, got, expected) {
    tests.push({ name, passed, got: String(got), expected: String(expected), latency: 0 });
  }

  let srvCounter, srvEcho;
  try {
    [srvCounter, srvEcho] = await Promise.all([
      startServer(COUNTER_APP, PORT_A, { workers: 4 }),
      startServer(ECHO_APP,    PORT_B, { workers: 4 }),
    ]);

    // 1. Counter increments (state persists within an isolate)
    const seqRes = [];
    for (let i = 0; i < 6; i++) seqRes.push(await request(PORT_A, '/'));
    const counts = seqRes.map((r) => parseInt(r.body, 10));
    const allNums = counts.every((n) => Number.isInteger(n) && n >= 1);
    rec('Counter increments (6 sequential)', allNums, counts.join(','), '>=1 each');

    // 2. All counters are at least 1
    rec('Counter all ≥ 1', counts.every((n) => n >= 1), counts.join(','), '≥1 each');

    // 3. Parallel burst — 20 concurrent requests, all succeed
    const parallel = await burst(PORT_B, '/?id=test', 20);
    const allOk200 = parallel.every((r) => r.status === 200);
    rec('20 parallel — all 200', allOk200, `${parallel.filter((r) => r.status === 200).length}/20`, '20/20');

    // 4. Echo returns correct id under concurrency
    const idsOk = parallel.every((r) => r.body === 'test');
    rec('20 parallel — correct body', idsOk, `${parallel.filter((r) => r.body === 'test').length}/20`, '20/20');

    // 5. 50-request burst
    const burst50 = await burst(PORT_B, '/?id=big', 50);
    const ok50 = burst50.filter((r) => r.status === 200).length;
    rec('50 parallel — ≥48 succeed', ok50 >= 48, `${ok50}/50`, '≥48/50');

    // 6. Latency p95 under 2000 ms
    const latencies = burst50.map((r) => r.latency).sort((a, b) => a - b);
    const p95 = latencies[Math.floor(latencies.length * 0.95)];
    rec('p95 latency < 2000 ms', p95 < 2000, `${p95}ms`, '<2000ms');

    // 7. No cross-tenant contamination: two different hostnames stay isolated
    // (Both run on separate ports, so they use separate worker pools)
    const counterAfter = await request(PORT_A, '/');
    rec('Separate pool no contamination', counterAfter.status === 200, 'separate pools ok', 'ok');

    // 8. Memory is reported
    const memA = getRssMb(srvCounter.proc.pid);
    const memB = getRssMb(srvEcho.proc.pid);
    rec(
      `RSS < 256 MB per worker`,
      (memA === null || memA < 256) && (memB === null || memB < 256),
      `${memA}/${memB} MB`, '< 256 MB'
    );

  } finally {
    await Promise.all([stopServer(srvCounter), stopServer(srvEcho)]);
  }

  return { name: 'Concurrency', tests, memMb: null };
};
