// Hot-swap demo — version 2 (the new deploy).
function fetch(request) {
  return {
    status: 200,
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ version: "v2", message: "hello from v2" }),
  };
}
