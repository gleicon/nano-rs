/**
 * gas-notes.gs — zero-config GAS drop-in example for nano:gas
 *
 * A simple note store backed by PropertiesService (which is synchronous
 * and requires no Google credentials — it runs on nano:kv internally).
 *
 * Run with:
 *   nano-rs run -c examples/configs/gas-notes.json
 *
 * Endpoints:
 *   GET  /                              — list all notes as JSON
 *   POST / {"title":"x","body":"y"}    — create a note, returns its id
 *   POST / {"function":"getNote","args":["<id>"]}   — fetch one note
 *   POST / {"function":"deleteNote","args":["<id>"]} — delete a note
 *   POST / {"function":"clearAll"}                   — delete everything
 *
 * This file runs unchanged in real Google Apps Script (deploy as Web App).
 * On nano-rs: set GAS_COMPAT=true in env_vars — no other config needed.
 */

var PROPS = PropertiesService.getScriptProperties();  // synchronous

// ── entry points ──────────────────────────────────────────────────────────────

function doGet(e) {
  var id = e.parameter && e.parameter.id;
  if (id) return getNote(id);

  var all = PROPS.getProperties();
  var notes = [];
  for (var k in all) {
    if (k.indexOf('note:') === 0) {
      try { notes.push(JSON.parse(all[k])); } catch (_) {}
    }
  }
  notes.sort(function(a, b) { return b.created - a.created; });

  Logger.log('GET / — returned %s notes', notes.length);
  return JSON.stringify({ count: notes.length, notes: notes });
}

function doPost(e) {
  var body;
  try { body = JSON.parse(e.postData.contents); } catch (_) {
    return JSON.stringify({ error: 'invalid JSON' });
  }

  var id = Utilities.getUuid();
  var note = {
    id: id,
    title: body.title || 'Untitled',
    body: body.body || '',
    created: Date.now(),
    checksum: Utilities.base64Encode(body.title + body.body),
  };

  PROPS.setProperty('note:' + id, JSON.stringify(note));
  Logger.log('Created note %s: %s', id, note.title);
  return JSON.stringify({ id: id, note: note });
}

// ── directly-dispatched functions ─────────────────────────────────────────────
// Called via POST {"function":"getNote","args":["<id>"]}

function getNote(id) {
  var raw = PROPS.getProperty('note:' + id);
  if (!raw) return JSON.stringify({ error: 'not found', id: id });
  return raw;
}

function deleteNote(id) {
  var raw = PROPS.getProperty('note:' + id);
  if (!raw) return JSON.stringify({ error: 'not found', id: id });
  PROPS.deleteProperty('note:' + id);
  Logger.log('Deleted note %s', id);
  return JSON.stringify({ deleted: id });
}

function clearAll() {
  var all = PROPS.getProperties();
  var count = 0;
  for (var k in all) {
    if (k.indexOf('note:') === 0) { PROPS.deleteProperty(k); count++; }
  }
  Logger.log('Cleared %s notes', count);
  return JSON.stringify({ cleared: count });
}

function status() {
  var all = PROPS.getProperties();
  var count = Object.keys(all).filter(function(k) { return k.indexOf('note:') === 0; }).length;
  return JSON.stringify({
    notes: count,
    user: Session.getEffectiveUser().getEmail(),
    runtime: typeof Nano !== 'undefined' ? 'nano-rs' : 'google-apps-script',
  });
}
