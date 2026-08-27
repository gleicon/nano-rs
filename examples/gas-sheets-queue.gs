/**
 * gas-sheets-queue.gs — Google Apps Script drop-in example for nano:gas
 *
 * Reads rows from a Google Sheet, sends each to an LLM, writes results back.
 * This is a real .gs file: copy it to Google Apps Script and it works there too.
 * On nano-rs the `.gs` extension auto-selects the GAS shim — no changes needed.
 *
 * Setup:
 *   1. Create a Google Cloud service account and download the JSON key file.
 *   2. Share your Google Sheet with the service account email (as Editor).
 *   3. Set GOOGLE_SERVICE_ACCOUNT_KEY, SPREADSHEET_ID, and OPENAI_API_KEY in env_vars.
 *   4. Run with: nano-rs run -c examples/configs/gas-sheets-queue.json
 *
 * Sheet layout (row 1 is header, skipped):
 *   Column A: input prompt
 *   Column B: AI result (filled in by this script, left blank initially)
 *
 * Try it:
 *   # Dry-run: see which rows are pending without processing them
 *   curl "http://localhost:8080/?dry=1"
 *   → {"pending":3,"rows":[{"row":2,"prompt":"Summarise Q3 results"},…]}
 *
 *   # Process all pending rows (calls OpenAI, writes back to sheet)
 *   curl http://localhost:8080/
 *   → {"processed":3}
 *
 *   # Clear all results so rows can be re-processed
 *   curl -X POST http://localhost:8080/ \
 *        -H 'content-type: application/json' \
 *        -d '"reset"'
 *   → {"reset":true,"rows":3}
 */

// ── helpers ──────────────────────────────────────────────────────────────────

function getSheet() {
  return SpreadsheetApp.getActiveSpreadsheet().getActiveSheet();
}

// ── entry points ──────────────────────────────────────────────────────────────

async function doGet(e) {
  const dry = e.parameter && e.parameter.dry === '1';
  const sheet = getSheet();

  // getDataRange() and getValues() require await on nano-rs (async fetch).
  // In real GAS these are synchronous — the only change needed for nano:rs.
  const data = await (await sheet.getDataRange()).getValues();

  // Find rows with a prompt but no result (skip header row 0)
  const pending = [];
  for (var i = 1; i < data.length; i++) {
    if (data[i][0] && !data[i][1]) {
      pending.push({ row: i + 1, prompt: String(data[i][0]) });
    }
  }

  if (dry) {
    return JSON.stringify({ pending: pending.length, rows: pending });
  }

  var processed = 0;
  for (var j = 0; j < pending.length; j++) {
    var item = pending[j];

    var resp = await UrlFetchApp.fetch('https://api.openai.com/v1/chat/completions', {
      method: 'POST',
      headers: {
        'Authorization': 'Bearer ' + Nano.env.OPENAI_API_KEY,
        'Content-Type': 'application/json',
      },
      payload: JSON.stringify({
        model: 'gpt-4o-mini',
        messages: [{ role: 'user', content: item.prompt }],
        max_tokens: 256,
      }),
    });

    var answer = JSON.parse(resp.getContentText()).choices[0].message.content;

    // setValues() is buffered — all writes sent as one batchUpdate when doGet returns.
    sheet.getRange(item.row, 2).setValue(answer);
    Logger.log('Row %s processed: %s chars', item.row, answer.length);
    processed++;
  }

  // PropertiesService is synchronous (no await needed)
  PropertiesService.getScriptProperties().setProperty('last_run_count', String(processed));

  return JSON.stringify({ processed: processed });
}

async function doPost(e) {
  var path = '';
  try { path = new URL(e.parameter.__url__ || '').pathname; } catch (_) {}

  if (path === '/reset' || (e.postData && e.postData.contents === 'reset')) {
    var sheet = getSheet();
    var data = await (await sheet.getDataRange()).getValues();
    for (var i = 1; i < data.length; i++) {
      if (data[i][1]) sheet.getRange(i + 1, 2).setValue('');
    }
    return JSON.stringify({ reset: true, rows: data.length - 1 });
  }

  return JSON.stringify({ error: 'unknown path' });
}
