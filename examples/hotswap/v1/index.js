// Hot-swap demo — version 1.
// Serve, then deploy v2 over the same sliver file and SIGHUP the process:
// the response flips to v2 with zero downtime and no hostname change.
function fetch(request) {
  return {
    status: 200,
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ version: "v1", message: "hello from v1" }),
  };
}
