// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
// Cross-check for juicebar-probe: prints each device's battery as Logitech G HUB reports it over its
// local websocket (ws://localhost:9010, subprotocol "json"). Run with node 24 (global WebSocket, no
// package). Read-only: only GET requests. Prints a whitelist of fields (no serial or unit ID).
// Usage: node ghub_battery.mjs [--raw]   (--raw also prints the battery payload keys, never the values of other fields)

const raw = process.argv.includes("--raw");
const ws = new WebSocket("ws://localhost:9010", "json");
const pending = new Map();
let nextId = 1;
let events = 0;

const quit = (code) => { try { ws.close(); } catch {} setTimeout(() => process.exit(code), 50); };
const timer = setTimeout(() => {
  console.log(`done: 10 s elapsed, ${events} battery change event(s) seen`);
  quit(0);
}, 9000);

function get(path) {
  const msgId = String(nextId++);
  return new Promise((resolve) => {
    pending.set(msgId, resolve);
    ws.send(JSON.stringify({ msgId, verb: "GET", path }));
    setTimeout(() => { if (pending.delete(msgId)) resolve(null); }, 3000);
  });
}

ws.onerror = (e) => { console.log(`G HUB websocket error: ${e.message ?? e.type}`); clearTimeout(timer); quit(1); };
ws.onclose = (e) => { if (e.code !== 1000 && e.code !== 1005) console.log(`closed ${e.code} ${e.reason}`); };
ws.onmessage = (e) => {
  let m;
  try { m = JSON.parse(String(e.data)); } catch { return; }
  if (m.msgId && pending.has(m.msgId)) { pending.get(m.msgId)(m); pending.delete(m.msgId); return; }
  if (typeof m.path === "string" && m.path.includes("/battery/") && m.verb !== "OPTIONS") {
    events++;
    console.log(`event ${m.path}: ${fmt(m.payload)}`);
  }
};

function fmt(p) {
  if (!p) return "(no payload)";
  const keys = ["percentage", "charging", "mileage", "maxLifeSpan", "consumption", "criticalLevel", "chargingError", "millivolts", "state", "level", "fullyCharged", "time_to_full", "time_to_empty"];
  const out = keys.filter((k) => k in p).map((k) => `${k}=${JSON.stringify(p[k])}`);
  if (raw) out.push(`[keys: ${Object.keys(p).join(",")}]`);
  return out.join(", ") || `(keys: ${Object.keys(p).join(",")})`;
}

ws.onopen = async () => {
  console.log(`connected to ws://localhost:9010, subprotocol "${ws.protocol}"`);
  ws.send(JSON.stringify({ msgId: "sub", verb: "SUBSCRIBE", path: "/battery/state/changed" }));
  const list = await get("/devices/list");
  if (!list || list.result?.code !== "SUCCESS") {
    console.log(`G HUB refused /devices/list: ${JSON.stringify(list?.result ?? "no answer")}`);
    clearTimeout(timer); quit(1); return;
  }
  const infos = list.payload?.deviceInfos ?? [];
  console.log(`G HUB lists ${infos.length} device(s)`);
  for (const d of infos) {
    const b = await get(`/battery/${d.id}/state`);
    const ok = b?.result?.code === "SUCCESS";
    console.log(`${d.id} ${d.displayName} [${d.deviceType}, ${d.connectionType}, ${d.state}]: ` +
      (ok ? fmt(b.payload) : `battery GET failed: ${JSON.stringify(b?.result ?? "no answer")}`));
  }
};
