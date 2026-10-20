# Recipes

Each block is one round for `ego-browser nodejs` (heredoc in bash; on
PowerShell save it as a `.mjs` file and `import()` it, see SKILL.md). Replace
`<id>` with the `spaceId` printed in round 1. Check signatures with
`help("<api>")` when the installed version differs.

## 1. Open, look, decide (round 1 of anything)

```js
const task = await taskSpace("compare laptop prices");
const page = task.page("p1");
await page.goto("https://example.com/laptops");
await page.waitForLoadState();
console.log({ spaceId: task.spaceId, page: page.label, url: await page.url() });
console.log(await page.snapshot());
```

## 2. Search, then extract a compact table

```js
const task = await taskSpace(<id>);
const page = task.page("p1");
await page.fill("loc=role:searchbox", "noise cancelling headphones");
await page.press("loc=role:searchbox", "Enter");
await page.waitForSelector("css=[data-component-type='s-search-result']", { state: "visible" });
const rows = await page.evaluate((limit) =>
  [...document.querySelectorAll("[data-component-type='s-search-result']")].slice(0, limit).map((n) => ({
    title: n.querySelector("h2")?.textContent?.trim(),
    price: n.querySelector(".a-price .a-offscreen")?.textContent?.trim() ?? null,
    url: n.querySelector("h2 a")?.href,
  })), 10);
console.log(JSON.stringify(rows, null, 1));
```

Selectors are site-specific: take them from a snapshot or a quick
`page.evaluate` probe, not from memory.

## 3. Read a signed-in dashboard

```js
const task = await taskSpace("read billing usage");
const page = task.page("p1");
await page.goto("https://dashboard.example.com/billing");
await page.waitForLoadState();
const url = await page.url();
if (/login|signin|auth/i.test(url)) {
  await task.handOff();
  console.log({ handOff: true, spaceId: task.spaceId, reason: "sign-in needed", url });
} else {
  console.log(await page.snapshot());
}
```

If it hands off, tell the user to sign in in the ego window, then resume with
`takeOverTaskSpace(<id>)`.

## 4. Fill a form, stop before submitting

```js
const task = await taskSpace(<id>);
const page = task.page("p1");
await page.fill("@14", "Ada Lovelace");
await page.fill("@15", "ada@example.com");
await page.selectOption("@18", { label: "Engineering" });
console.log(await page.snapshot());
```

Show the user what will be sent, get a yes, then click submit in the next
round and wait for the confirmation state.

## 5. Paginated or infinite list

```js
const task = await taskSpace(<id>);
const page = task.page("p1");
const seen = new Map();
for (let i = 0; i < 8; i++) {
  const batch = await page.evaluate(() =>
    [...document.querySelectorAll("article")].map((a) => ({ id: a.dataset.id, text: a.innerText.slice(0, 200) })));
  for (const b of batch) seen.set(b.id ?? b.text, b);
  const next = await page.evaluate(() => !!document.querySelector("a[rel=next]"));
  if (next) {
    await page.click("a[rel=next]");
    await page.waitForLoadState();
  } else {
    await page.mouse.move(600, 400);
    await page.mouse.wheel(0, 2400, { label: "load more results" });
    await page.waitForTimeout(800);
  }
  if (seen.size >= 100) break;
}
console.log(JSON.stringify([...seen.values()].slice(0, 100)));
```

## 6. Download a file

```js
const task = await taskSpace(<id>);
const page = task.page("p1");
const pending = page.waitForEvent("download", { timeout: 30_000 });
await page.click("text=\"Export CSV\"");
const download = await pending;
await download.saveAs("C:/Users/<you>/Downloads/export.csv");
console.log({ saved: true, name: download.suggestedFilename() });
```

## 7. Upload a file

```js
const task = await taskSpace(<id>);
const page = task.page("p1");
const chooser = page.waitForFileChooser({ timeout: 10_000 });
await page.click("loc=role:button[name='Upload']");
await (await chooser).setFiles("C:/Users/<you>/report.pdf");
await page.waitForSelector("text=report.pdf", { state: "visible" });
console.log(await page.snapshot());
```

## 8. Compare several pages side by side

```js
const task = await taskSpace(<id>);
const urls = ["https://a.example/pricing", "https://b.example/pricing", "https://c.example/pricing"];
const out = [];
for (const url of urls) {
  const page = out.length === 0 ? task.page("p1") : await task.newPage();
  await page.goto(url);
  await page.waitForLoadState();
  out.push({ url, page: page.label, plans: await page.evaluate(() =>
    [...document.querySelectorAll("h2, h3")].map((h) => h.textContent.trim()).slice(0, 12)) });
}
console.log(JSON.stringify(out, null, 1));
```

## 9. QA pass on a local web app

```js
const task = await taskSpace("qa signup flow");
const page = task.page("p1");
await page.cdp("Runtime.enable");
await page.goto("http://localhost:3000/signup");
await page.fill("loc=role:textbox[name='Email']", "not-an-email");
await page.click("loc=role:button[name='Create account']");
await page.waitForSelector("text=valid email", { state: "visible", timeout: 5_000 }).catch(() => {});
const shot = await page.screenshot({ path: "C:/Users/<you>/qa/signup-invalid.png" });
// Event shape: check help("page.events") for the installed version.
const errors = (await page.events()).filter((e) => (e.method ?? e.type) === "Runtime.exceptionThrown");
console.log({ shot, consoleErrors: errors.length });
console.log(await page.snapshot());
```

## 10. Finish

```js
const task = await taskSpace(<id>);
await task.finish({ keep: [] });
console.log("done");
```
