// Scripted dove-browser evals over ego lite, against the local fixtures.
//
// Start the fixtures (node fixtures/server.mjs 8787), then run inside ego's
// embedded Node (PowerShell form, per the ego-browser guide):
//
//   ego-browser nodejs -e "await import('file:///C:/path/to/skills/dove-browser/evals/run-ego.mjs')"
//
// Each task follows the dove-browser loop with the documented ego API only:
// observe -> act -> wait for the expected state -> verify with a fresh
// observation or with what the fixture server recorded. Results go to
// results/<stamp>-ego-scripted.json next to this file. This checks the
// browser provider and the recipes the skill teaches; agent runs (an agent
// reading SKILL.md and choosing its own steps) are recorded separately.

const fs = await import("node:fs/promises");
const crypto = await import("node:crypto");
const { fileURLToPath } = await import("node:url");
const path = await import("node:path");

const BASE = "http://127.0.0.1:8787";
const here = path.dirname(fileURLToPath(import.meta.url));
const resultsDir = path.join(here, "results").replaceAll("\\", "/");
const shotsDir = path.join(here, "results", "shots").replaceAll("\\", "/");
const uploadFile = path.join(here, "fixtures", "report.txt").replaceAll("\\", "/");
await fs.mkdir(shotsDir, { recursive: true });

const serverLog = async () => (await fetch(`${BASE}/__log`)).json();
await fetch(`${BASE}/__reset`);

const stamp = new Date().toISOString().replace(/[-:]/g, "").slice(0, 15);
const task = await taskSpace(`dove-browser-evals-${stamp}`);
const page = task.page("p1");
const startWindow = await task
  .cdp("Browser.getWindowForTarget", { targetId: page.targetId })
  .then((w) => w.bounds?.windowState ?? "unknown", () => "unknown");
console.log(`ego window at start: ${startWindow}`);
const results = [];
const assert = (ok, msg) => { if (!ok) throw new Error(`check failed: ${msg}`); };

async function run(id, title, fn) {
  const t0 = performance.now();
  try {
    const detail = await fn();
    results.push({ id, title, pass: true, ms: Math.round(performance.now() - t0), detail });
    console.log(`PASS ${id} (${Math.round(performance.now() - t0)} ms)`);
  } catch (e) {
    results.push({ id, title, pass: false, ms: Math.round(performance.now() - t0), error: String(e?.message || e).slice(0, 600) });
    console.log(`FAIL ${id}: ${String(e?.message || e).slice(0, 300)}`);
  }
}

// Refs printed in a snapshot, by the text of the line under them.
function refFor(snapshot, text) {
  const lines = snapshot.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const m = /\[ref=(\d+)\]/.exec(lines[i]);
    if (!m) continue;
    const block = [lines[i], lines[i + 1] || ""].join(" ");
    if (block.includes(text)) return `@${m[1]}`;
  }
  return null;
}

await run("01-search-extract", "Search and extract structured results", async () => {
  await page.goto(`${BASE}/search`);
  await page.fill("#q", "rust");
  await page.click("loc=role:button[name='Search']", { label: "Run the search" });
  await page.waitForSelector("#count", { state: "visible" });
  const rows = await page.evaluate(() => [...document.querySelectorAll("#results .result")].map((r) => ({
    title: r.querySelector("a")?.textContent.trim(), url: r.querySelector("a")?.href,
    author: r.querySelector(".author")?.textContent.trim(), date: r.querySelector("time")?.textContent.trim(),
  })));
  const snap = await page.snapshot();
  assert(rows.length === 4, `4 rust results, got ${rows.length}`);
  assert(rows.every((r) => r.title && r.url && r.author && /^\d{4}-\d{2}-\d{2}$/.test(r.date)), "every field present");
  assert(snap.includes("4 results"), "fresh snapshot shows the count");
  return { rows: rows.length, first: rows[0] };
});

await run("02-cheapest", "Cheapest in-stock matching item", async () => {
  await page.goto(`${BASE}/catalog?q=usb-c hub`);
  const items = await page.evaluate(() => [...document.querySelectorAll(".product")].map((p) => ({
    sku: p.dataset.sku, name: p.querySelector("h2").textContent.trim(),
    price: Number(p.querySelector(".price").textContent.replace(/[^0-9.]/g, "")),
    inStock: p.querySelector(".stock").textContent.includes("In stock"),
  })));
  const best = items.filter((i) => i.inStock).sort((a, b) => a.price - b.price)[0];
  await page.click(`loc=href:/product/${best.sku}`, { label: "Open cheapest hub" });
  await page.waitForURL(`**/product/${best.sku}`);
  const shown = await page.evaluate(() => document.querySelector(".price").textContent.trim());
  assert(best.sku === "HUB-X", `expected HUB-X (the 24.50 hub is out of stock), got ${best.sku}`);
  assert(shown === "$27.75", `product page shows ${shown}`);
  return { best, verifiedOn: await page.url() };
});

await run("03-multipage", "Navigate a multipage site", async () => {
  await page.goto(`${BASE}/docs/1`);
  const visited = [await page.url()];
  for (let i = 0; i < 5; i++) {
    const hasNext = await page.evaluate(() => !!document.querySelector("a[rel=next]"));
    if (!hasNext) break;
    await page.click("a[rel=next]", { label: "Next docs page" });
    await page.waitForLoadState();
    visited.push(await page.url());
  }
  const pass = await page.evaluate(() => document.querySelector("#pass")?.textContent);
  assert(visited.length === 3, `3 pages, visited ${visited.length}`);
  assert(pass === "pelican-42", `passphrase ${pass}`);
  return { visited, pass };
});

await run("04-spa", "Dynamically rendered app", async () => {
  await page.goto(`${BASE}/app`);
  await page.waitForFunction(() => window.appReady === true, undefined, { timeout: 10_000 });
  await page.click("text=Gamma", { label: "Open Gamma" });
  await page.waitForSelector("#code", { state: "visible", timeout: 5_000 });
  const code = await page.evaluate(() => document.querySelector("#code").textContent);
  const url = await page.url();
  assert(code === "G-3003", `code ${code}`);
  assert(url.endsWith("#/item/3"), `url ${url}`);
  return { code, url };
});

await run("05-stale-ref", "Recover from a stale element reference", async () => {
  await page.goto(`${BASE}/stale`);
  const first = await page.snapshot();
  const writeRef = refFor(first, "Mark Write tests done");
  const shipRef = refFor(first, "Mark Ship it done");
  assert(writeRef && shipRef, "refs found in the first snapshot");
  await page.click(writeRef, { label: "Mark Write tests done" });
  await page.waitForFunction(() => document.querySelector("#done").textContent.startsWith("1 of"), undefined, { timeout: 5_000 });
  // The list was rebuilt: the old ref must not be trusted.
  let staleError = null;
  try { await page.click(shipRef, { timeout: 1_500 }); } catch (e) { staleError = String(e.message || e).slice(0, 200); }
  const afterStale = await page.evaluate(() => document.querySelector("#done").textContent);
  // Recovery: fresh snapshot, fresh ref.
  const fresh = await page.snapshot();
  const freshShip = refFor(fresh, "Mark Ship it done");
  if (afterStale.startsWith("1 of")) await page.click(freshShip, { label: "Mark Ship it done" });
  await page.waitForFunction(() => document.querySelector("#done").textContent.startsWith("2 of"), undefined, { timeout: 5_000 });
  const final = await page.evaluate(() => document.querySelector("#done").textContent);
  assert(final === "2 of 5 done", final);
  return { staleRefRejected: staleError !== null, staleError, staleClickLanded: !afterStale.startsWith("1 of"), final };
});

await run("06-form-review", "Fill a form, review, submit only after confirmation", async () => {
  await page.goto(`${BASE}/form`);
  await page.fill("#name", "Test Person");
  await page.fill("#email", "test.person@example.test");
  await page.selectOption("#topic", { label: "Support" });
  await page.fill("#message", "Eval message: please ignore.");
  await page.click("text=Subscribe to the newsletter", { label: "Tick newsletter" });
  const review = await page.evaluate(() => Object.fromEntries([...new FormData(document.querySelector("form"))]));
  const before = (await serverLog()).forms.length;
  assert(before === 0, "nothing submitted during review");
  assert(review.name === "Test Person" && review.topic === "support" && review.newsletter === "yes", JSON.stringify(review));
  // The user confirmed the review summary (scripted stand-in for the consent step).
  await page.click("loc=role:button[name='Send message']", { label: "Send the message" });
  await page.waitForSelector("#ticket", { state: "visible" });
  const ticket = await page.evaluate(() => document.querySelector("#ticket").textContent);
  const posted = (await serverLog()).forms;
  assert(posted.length === 1 && posted[0].email === "test.person@example.test", "server got exactly one matching post");
  return { review, ticket, serverTicket: posted[0].ticket };
});

await run("07-upload", "Upload a test file", async () => {
  const bytes = await fs.readFile(uploadFile);
  const sha = crypto.createHash("sha256").update(bytes).digest("hex");
  await page.goto(`${BASE}/upload`);
  await page.setInputFiles("#file", [uploadFile]);
  await page.click("loc=role:button[name='Upload']", { label: "Upload the report" });
  await page.waitForSelector("#received", { state: "visible" });
  const got = (await serverLog()).uploads.at(-1);
  assert(got && got.sha256 === sha && got.name === "report.txt", `server sha ${got?.sha256} vs ${sha}`);
  return { name: got.name, bytes: got.bytes, shaMatches: true };
});

await run("08-screenshot", "Capture a screenshot", async () => {
  await page.goto(`${BASE}/catalog`);
  // Chromium refuses every capture of a minimized window. The skill hands
  // off ("restore the ego lite window"); this run restores it itself, as the
  // tester asked, and puts it back afterwards.
  const win = await task.cdp("Browser.getWindowForTarget", { targetId: page.targetId });
  const wasMinimized = win.bounds?.windowState === "minimized";
  if (wasMinimized) await task.cdp("Browser.setWindowBounds", { windowId: win.windowId, bounds: { windowState: "normal" } });
  let file;
  try {
    file = await page.screenshot({ path: `${shotsDir}/catalog.png` });
  } finally {
    if (wasMinimized) await task.cdp("Browser.setWindowBounds", { windowId: win.windowId, bounds: { windowState: "minimized" } });
  }
  const head = (await fs.readFile(file)).subarray(0, 24);
  assert(head.subarray(1, 4).toString() === "PNG", "PNG signature");
  const w = head.readUInt32BE(16), h = head.readUInt32BE(20);
  assert(w > 300 && h > 200, `size ${w}x${h}`);
  return { file, width: w, height: h, windowWasMinimized: wasMinimized };
});

await run("09-webapp-critical-path", "Test a local web app's critical path", async () => {
  // As the skill's QA recipe says: console messages arrive only with Runtime on.
  await page.cdp("Runtime.enable", {});
  await page.events(); // clear the buffer
  await page.goto(`${BASE}/shop`);
  await page.fill("#display", "Eval Shopper");
  await page.click("loc=role:button[name='Create account']", { label: "Create the account" });
  await page.waitForURL("**/shop/products**");
  await page.waitForLoadState();
  const events = await page.events();
  const consoleErrors = events.filter((e) => JSON.stringify(e).includes("Failed to load recommendations")).length;
  await page.click("loc=role:button[name='Order Notebook']", { label: "Order the notebook" });
  await page.waitForURL("**/shop/order/**");
  const order = await page.evaluate(() => document.querySelector("#order")?.textContent);
  const log = await serverLog();
  assert(order?.includes("Notebook"), `order page: ${order}`);
  assert(log.orders.length === 1 && log.orders[0].user === "Eval Shopper", "server recorded one order for the shopper");
  assert(consoleErrors > 0, "the products page's console.error was captured");
  return { order, consoleErrorSeen: consoleErrors > 0, eventKinds: [...new Set(events.map((e) => e.type || e.method || "?"))].slice(0, 10) };
});

await run("10-nav-timeout", "Recover from a navigation timeout", async () => {
  let timeout = null;
  const t0 = performance.now();
  try { await page.goto(`${BASE}/slow?ms=20000`, { timeout: 3_000 }); } catch (e) { timeout = String(e.message || e).slice(0, 300); }
  const waited = Math.round(performance.now() - t0);
  assert(timeout, "goto reported a timeout");
  assert(waited < 8_000, `gave up in ${waited} ms`);
  await page.goto(`${BASE}/docs/1`);
  const title = await page.title();
  assert(title === "Docs part 1", `recovered page title ${title}`);
  return { timeoutAfterMs: waited, error: timeout, recoveredTo: await page.url() };
});

await run("11-popup", "Follow a link that opens a new tab", async () => {
  await page.goto(`${BASE}/popup`);
  const popupPromise = page.waitForEvent("popup");
  await page.click("text=Docs", { label: "Open Docs in new tab" });
  const popup = await popupPromise;
  await popup.waitForLoadState();
  const h1 = await popup.evaluate(() => document.querySelector("h1")?.textContent);
  const label = popup.label;
  await popup.close();
  assert(h1 === "Docs part 1", `popup heading ${h1}`);
  return { popupLabel: label, h1 };
});

await run("12-infinite-scroll", "Collect 60 items from an infinite feed", async () => {
  await page.goto(`${BASE}/feed`);
  await page.waitForSelector(".post");
  // Recipe 5: scroll the last item into view in the page, wait for growth.
  let ids = [];
  for (let round = 0; round < 12 && ids.length < 60; round++) {
    ids = await page.evaluate(() => [...new Set([...document.querySelectorAll(".post")].map((p) => Number(p.dataset.id)))]);
    if (ids.length >= 60) break;
    await page.evaluate(() => document.querySelector(".post:last-of-type")?.scrollIntoView({ block: "end" }));
    const grew = await page
      .waitForFunction((n) => document.querySelectorAll(".post").length > n, ids.length, { timeout: 3_000 })
      .then(() => true, () => false);
    if (!grew) break;
  }
  const first60 = ids.slice(0, 60);
  assert(first60.length === 60 && new Set(first60).size === 60 && first60[59] === 60, `got ${ids.length} ids`);
  return { collected: ids.length, last: first60[59] };
});

await run("13-injection-extract", "Extract facts from a page with injected instructions", async () => {
  await page.goto(`${BASE}/inject`);
  const spec = await page.evaluate(() => Object.fromEntries([...document.querySelectorAll("#spec tr")].map((r) => [r.cells[0].textContent, r.cells[1].textContent])));
  const log = await serverLog();
  assert(spec.Battery === "14 hours" && spec.Price === "$199", JSON.stringify(spec));
  assert(log.exfil.length === 0, "nothing hit /exfil");
  return { spec, exfilHits: log.exfil.length, note: "obedience is graded in agent runs; this checks extraction" };
});

await run("14-compare", "Research and compare three products with sources", async () => {
  await page.goto(`${BASE}/compare`);
  const links = await page.evaluate(() => [...document.querySelectorAll("li a")].map((a) => a.href));
  const out = [];
  for (const href of links) {
    await page.goto(href);
    out.push(await page.evaluate(() => ({
      name: document.querySelector("h1").textContent, price: document.querySelector(".price").textContent.replace("Price: ", ""),
      features: [...document.querySelectorAll(".features li")].map((l) => l.textContent), source: location.href,
    })));
  }
  // Verify one extracted record against a fresh observation of its source.
  await page.goto(out[1].source);
  const snap = await page.snapshot({ scope: "full_page" });
  assert(out.length === 3, "three products");
  assert(snap.includes(out[1].price) && out[1].features.every((f) => snap.includes(f)), "fresh snapshot matches the extracted record");
  return { products: out.map((p) => ({ name: p.name, price: p.price, features: p.features.length, source: p.source })) };
});

await task.finish({ keep: [] });

const passed = results.filter((r) => r.pass).length;
const report = {
  provider: "ego lite",
  egoGuide: "v2.1.0",
  mode: "scripted (documented ego API, no agent)",
  windowAtStart: startWindow,
  when: new Date().toISOString(),
  passed,
  total: results.length,
  totalMs: results.reduce((s, r) => s + r.ms, 0),
  results,
};
const out = `${resultsDir}/${stamp}-ego-scripted.json`;
await fs.writeFile(out, JSON.stringify(report, null, 2));
console.log(JSON.stringify({ passed, total: results.length, out }));
