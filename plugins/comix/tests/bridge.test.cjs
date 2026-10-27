const { test, after } = require("node:test");
const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { fileURLToPath, pathToFileURL } = require("node:url");

// The bridge runs exactly as the host runs it: an async function body opted in by the marker,
// with `const request = {...}` prepended. Site modules are real ES modules loaded by import();
// the page, fetch and crypto around them are stubbed. No network is used.
const bridgeSource = fs.readFileSync(path.join(__dirname, "../src/bridge.js"), "utf8");
const AsyncFunction = (async () => {}).constructor;
const SITE_ORIGIN = "https://comix.to";

const fixtureDir = fs.mkdtempSync(path.join(os.tmpdir(), "comix-bridge-"));
after(() => fs.rmSync(fixtureDir, { recursive: true, force: true }));
fs.writeFileSync(path.join(fixtureDir, "package.json"), '{"type":"module"}\n');
fs.writeFileSync(path.join(fixtureDir, "recorder.js"), `export const calls = [];
export const responses = new Map();
export function call(name, args) {
  calls.push({ name, args });
  const respond = responses.get(name);
  if (respond === undefined) throw new Error("no fixture response for " + name);
  return respond(...args);
}
`);
const recorder = import(pathToFileURL(path.join(fixtureDir, "recorder.js")).href);

let fixtureCount = 0;

function sha256(file) {
  return crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

// Writes a site client module whose facades forward every call to the recorder.
function writeSite(name, { facade = ["o3", "nb", "E5", "iF"], pages = ["E5"] } = {}) {
  const file = path.join(fixtureDir, `apicore-${name}-${fixtureCount++}.js`);
  const members = (names, prefix) =>
    names.map(member => `${JSON.stringify(member)}: (...args) => call(${JSON.stringify(prefix + member)}, args)`).join(", ");
  fs.writeFileSync(file, `import { call } from "./recorder.js";
export const f = { ${members(facade, "")} };
export const n = { ${members(pages, "n.")} };
`);
  return { file, href: pathToFileURL(file).href, revision: sha256(file) };
}

function setGlobal(name, value) {
  Object.defineProperty(globalThis, name, { value, configurable: true, writable: true });
}

// Installs the page: its origin/URL, the resources and module references it lists, the frames its
// document holds (each { src, inDialog }), an optional itoShowPage handler, and a window whose
// listeners a test can fire (the site's captcha event).
function page({ origin = "null", href = `${SITE_ORIGIN}/`, resources = [], preload = [], scripts = [], moduleScripts = [], frames = [], showPage } = {}) {
  const listeners = new Map();
  setGlobal("location", { origin, href });
  setGlobal("performance", { getEntriesByType: () => resources.map(name => ({ name })) });
  const dialogFrames = frames.filter(frame => frame.inDialog).map(frame => ({ src: frame.src }));
  const queries = new Map([
    ['link[rel~="modulepreload"]', () => preload.map(url => ({ href: url }))],
    ["script[src]", () => scripts.map(url => ({ src: url }))],
    ['script[type="module"][src]', () => moduleScripts.map(url => ({ src: url }))],
    ['[role="dialog"] iframe', () => dialogFrames]
  ]);
  setGlobal("document", {
    querySelectorAll: selector => {
      const query = queries.get(selector);
      if (query === undefined) throw new Error(`unexpected selector: ${selector}`);
      return query();
    }
  });
  setGlobal("window", {
    ...(showPage ? { webkit: { messageHandlers: { itoShowPage: { postMessage: showPage } } } } : {}),
    addEventListener: (type, listener) => {
      if (!listeners.has(type)) listeners.set(type, new Set());
      listeners.get(type).add(listener);
    },
    removeEventListener: (type, listener) => listeners.get(type)?.delete(listener)
  });
  setGlobal("fetch", async href => {
    if (!String(href).startsWith("file:")) return { ok: false, status: 404, text: async () => "", arrayBuffer: async () => new ArrayBuffer(0) };
    const bytes = fs.readFileSync(fileURLToPath(href));
    return {
      ok: true,
      status: 200,
      text: async () => bytes.toString("utf8"),
      arrayBuffer: async () => bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength)
    };
  });
  return { emit: type => { for (const listener of [...(listeners.get(type) ?? [])]) listener({ detail: {} }); } };
}

// A frame the document holds: inside a [role="dialog"] element or outside any dialog.
const dialogFrame = src => ({ src, inDialog: true });
const pageFrame = src => ({ src, inDialog: false });

// Runs the composed body exactly as the host prepends it and returns the raw reply string.
async function runRaw(request) {
  const script = `//# ito-async\nconst request = ${JSON.stringify(request)};\n${bridgeSource}`;
  assert.ok(script.startsWith("//# ito-async\n"));
  const reply = await new AsyncFunction(script)();
  assert.equal(typeof reply, "string");
  assert.ok(reply.length > 0, "every path must return a non-empty reply");
  return reply;
}

async function run(request) {
  return JSON.parse(await runRaw(request));
}

async function reset() {
  (await recorder).calls.length = 0;
  (await recorder).responses.clear();
}

// Own keys and values of recorded arguments. Null prototypes and undefined properties are kept as the
// bridge sent them: the assertions see the real objects, not a JSON copy of them.
function plain(value) {
  if (Array.isArray(value)) return value.map(plain);
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(Object.keys(value).map(key => [key, plain(value[key])]));
  }
  return value;
}

function request(site, overrides = {}) {
  return { origin: "null", revision: site.revision, operation: "list", query: "", ...overrides };
}

// Parks the site's facade call until the test settles it. `arrived` resolves once the call reaches
// the site; `resolve` and `reject` settle the call the way the site would.
async function parkSiteCall(name) {
  const gate = {};
  gate.arrived = new Promise(resolve => {
    gate.enter = resolve;
  });
  gate.result = new Promise((resolve, reject) => {
    gate.resolve = resolve;
    gate.reject = reject;
  });
  (await recorder).responses.set(name, () => {
    gate.enter();
    return gate.result;
  });
  return gate;
}

const mangaDto = (overrides = {}) => ({
  qa: "hid-1",
  Rv: "One",
  vI: "manga",
  QZ: "releasing",
  vl: null,
  Zk: 12,
  G3: { Qq: "https://img.test/m.jpg", XN: null },
  ...overrides
});

const summary = {
  hash_id: "hid-1",
  title: "One",
  type: "manga",
  status: "releasing",
  synopsis: null,
  latest_chapter: 12,
  poster: { small: null, medium: "https://img.test/m.jpg", large: null }
};

test("list operation sends literal params to the site's list facade and adapts the envelope once", async () => {
  await reset();
  const site = writeSite("list");
  (await recorder).responses.set("o3", () => ({
    Kh: [mangaDto()],
    kZ: { mM: 2, DP: 3, Kn: 30, IN: 12, Qs: 13, oo: 24, Su: true, pI: true }
  }));
  page({ resources: [site.href] });
  const query = new URLSearchParams([
    ["page", "2"],
    ["keyword", "O'Reilly & café [edition]"],
    ["genres[]", "-8"],
    ["genres[]", "13"],
    ["order[relevance]", "desc"]
  ]).toString();

  const reply = await run(request(site, { query }));
  assert.deepEqual(reply, { ok: true, data: { items: [summary], meta: { page: 2, last_page: 3 } } });

  assert.deepEqual(plain((await recorder).calls), [{
    name: "o3",
    args: [{ page: 2, keyword: "O'Reilly & café [edition]", genres: [-8, 13], order: { relevance: "desc" } }]
  }]);
});

test("numeric search text stays text while numeric filters become numbers", async () => {
  await reset();
  const site = writeSite("numeric");
  (await recorder).responses.set("o3", () => ({ Kh: [], kZ: { mM: 1, DP: 1 } }));
  page({ resources: [site.href] });

  const reply = await run(request(site, { query: "keyword=00123&limit=30" }));

  assert.equal(reply.ok, true);
  assert.deepEqual(plain((await recorder).calls[0].args), [{ keyword: "00123", limit: 30 }]);
});

test("top operation calls the site's top facade with its own trending parameters", async () => {
  await reset();
  const site = writeSite("top");
  (await recorder).responses.set("nb", () => [mangaDto()]);
  page({ resources: [site.href] });

  const reply = await run(request(site, { operation: "top", query: "type=trending&days=1&limit=20&genres[]=-87264" }));

  assert.deepEqual(reply, { ok: true, data: [summary] });
  assert.deepEqual(plain((await recorder).calls), [{ name: "nb", args: [{ type: "trending", days: 1, limit: 20, genres: [-87264] }] }]);
});

test("detail operation maps the site's term DTOs to authors, artists and genres", async () => {
  await reset();
  const site = writeSite("detail");
  (await recorder).responses.set("E5", () => ({
    ...mangaDto(),
    Gd: [{ Xl: 7, Rv: "Auth" }],
    VC: [],
    xI: [{ Xl: 8, Rv: "Action" }]
  }));
  page({ resources: [site.href] });

  const reply = await run(request(site, { operation: "detail", key: "hid-1" }));

  assert.deepEqual(reply, {
    ok: true,
    data: {
      ...summary,
      author: [{ title: "Auth" }],
      artist: [],
      genre: [{ title: "Action" }]
    }
  });
  assert.deepEqual(plain((await recorder).calls), [{ name: "E5", args: ["hid-1"] }]);
});

test("detail with null or absent term lists maps them to null, and a non-array list is still refused", async () => {
  await reset();
  const site = writeSite("detail-lists");
  page({ resources: [site.href] });
  const detail = { operation: "detail", key: "hid-1" };
  const nullable = { ...summary, author: null, artist: null, genre: null };

  (await recorder).responses.set("E5", () => ({ ...mangaDto(), Gd: null, VC: null, xI: null }));
  assert.deepEqual(await run(request(site, detail)), { ok: true, data: nullable });

  (await recorder).responses.set("E5", () => mangaDto());
  assert.deepEqual(await run(request(site, detail)), { ok: true, data: nullable });

  (await recorder).responses.set("E5", () => ({ ...mangaDto(), Gd: "Auth" }));
  assert.deepEqual(await run(request(site, detail)), { ok: false, kind: "shape" });
});

test("chapters operation passes page, limit and order to the site's chapter facade", async () => {
  await reset();
  const site = writeSite("chapters");
  (await recorder).responses.set("iF", () => ({
    Kh: [
      { Lc: 501, p7: null, uR: 1.5, HY: null, K3: "", jI: 4, jq: false, Ix: null, _u: "x" },
      { Lc: 502, p7: 9, uR: 2, HY: null, K3: "Two", jI: 0, jq: true, Ix: { p7: 9, K3: "Group" }, _u: "y" }
    ],
    kZ: { mM: 1, DP: 2 }
  }));
  page({ resources: [site.href] });

  const reply = await run(request(site, { operation: "chapters", key: "hid-1", query: "limit=100&page=1&order[number]=desc" }));

  assert.deepEqual(reply, {
    ok: true,
    data: {
      items: [
        { chapter_id: 501, scanlation_group_id: null, number: 1.5, name: "", votes: 4, is_official: false, scanlation_group: null },
        { chapter_id: 502, scanlation_group_id: 9, number: 2, name: "Two", votes: 0, is_official: true, scanlation_group: { name: "Group" } }
      ],
      meta: { page: 1, last_page: 2 }
    }
  });
  assert.deepEqual(plain((await recorder).calls), [{ name: "iF", args: ["hid-1", { limit: 100, page: 1, order: { number: "desc" } }] }]);
});

test("pages operation returns direct image URLs, resolving relative ones against the page", async () => {
  await reset();
  const site = writeSite("pages");
  (await recorder).responses.set("n.E5", () => ({
    CP: [{ _u: "https://cdn.test/1.jpg", tu: [1], eR: "alt", Da: "node", hf: 100, _Q: 200 }, { _u: "/2.jpg", tu: [], eR: "", Da: "node", hf: 100, _Q: 200 }]
  }));
  page({ resources: [site.href], href: `${SITE_ORIGIN}/title/hid-1/501` });

  const reply = await run(request(site, { operation: "pages", key: "501" }));

  assert.deepEqual(reply, { ok: true, data: { images: [{ url: "https://cdn.test/1.jpg" }, { url: `${SITE_ORIGIN}/2.jpg` }] } });
  assert.deepEqual(plain((await recorder).calls), [{ name: "n.E5", args: ["501"] }]);
});

test("non-web page URLs are refused as a shape failure, never passed through", async () => {
  await reset();
  const site = writeSite("scheme");
  (await recorder).responses.set("n.E5", () => ({ CP: [{ _u: "javascript:alert(1)", tu: [], eR: "", Da: "node", hf: 1, _Q: 1 }] }));
  page({ resources: [site.href] });

  assert.deepEqual(await run(request(site, { operation: "pages", key: "501" })), { ok: false, kind: "shape" });
});

test("a module whose hash differs from the pinned revision is refused before any site call", async () => {
  await reset();
  const site = writeSite("revision");
  (await recorder).responses.set("o3", () => ({ Kh: [], kZ: { mM: 1, DP: 1 } }));
  page({ resources: [site.href] });

  assert.deepEqual(await run(request(site, { revision: "0".repeat(64) })), { ok: false, kind: "revision" });
  assert.equal((await recorder).calls.length, 0);
});

test("a missing facade method is refused rather than guessed at", async () => {
  await reset();
  const site = writeSite("nofacade", { facade: ["nb", "E5", "iF"] });
  page({ resources: [site.href] });

  assert.deepEqual(await run(request(site, { query: "page=1" })), { ok: false, kind: "missing-facade" });
  assert.equal((await recorder).calls.length, 0);
});

test("only the read-only allowlist is dispatched; arbitrary facade names are refused", async () => {
  await reset();
  const site = writeSite("allowlist");
  page({ resources: [site.href] });

  assert.deepEqual(await run(request(site, { operation: "invoke", query: "" })), { ok: false, kind: "operation" });
  assert.deepEqual(await run(request(site, { operation: "o3", query: "" })), { ok: false, kind: "operation" });
  assert.deepEqual(await run(request(site, { operation: "constructor", query: "" })), { ok: false, kind: "operation" });
  assert.equal((await recorder).calls.length, 0);
});

test("no client module, or more than one, is refused as missing", async () => {
  await reset();
  const first = writeSite("ambiguous-a");
  const second = writeSite("ambiguous-b");
  page({ resources: [] });
  assert.deepEqual(await run(request(first)), { ok: false, kind: "missing-module" });
  page({ resources: [first.href, second.href] });
  assert.deepEqual(await run(request(first)), { ok: false, kind: "missing-module" });
});

test("client candidates from another origin are ignored", async () => {
  await reset();
  const site = writeSite("foreign");
  (await recorder).responses.set("o3", () => ({ Kh: [], kZ: { mM: 1, DP: 1 } }));
  page({ resources: ["https://other.invalid/_app/apicore-x.js", site.href] });

  assert.deepEqual(await run(request(site)), { ok: true, data: { items: [], meta: { page: 1, last_page: 1 } } });
});

test("the client is found through the main module when the page lists no preload", async () => {
  await reset();
  const site = writeSite("linked");
  (await recorder).responses.set("o3", () => ({ Kh: [], kZ: { mM: 1, DP: 1 } }));
  const mainFile = path.join(fixtureDir, `main-${fixtureCount++}.js`);
  fs.writeFileSync(mainFile, `export const boot = () => import("./${path.basename(site.file)}");\n`);
  page({ moduleScripts: [pathToFileURL(mainFile).href] });

  assert.deepEqual(await run(request(site)), { ok: true, data: { items: [], meta: { page: 1, last_page: 1 } } });
  assert.equal((await recorder).calls.length, 1);
});

test("a challenge during a call rejects the operation instead of leaving it parked", async () => {
  await reset();
  const site = writeSite("challenge");
  let started;
  const invoked = new Promise(resolve => {
    started = resolve;
  });
  (await recorder).responses.set("o3", () => {
    started();
    return new Promise(() => {});
  });
  const sitePage = page({ resources: [site.href] });

  const pending = runRaw(request(site, { query: "page=1" }));
  await invoked;
  sitePage.emit("waf:captcha:open");

  assert.deepEqual(JSON.parse(await pending), { ok: false, kind: "challenge" });
});

test("a host reply of true keeps the call pending until the site resolves it", async () => {
  await reset();
  const site = writeSite("shown");
  const gate = await parkSiteCall("o3");
  const asked = [];
  const sitePage = page({ resources: [site.href], showPage: value => {
    asked.push(value);
    return true;
  } });
  const pending = runRaw(request(site, { query: "page=1" }));

  await gate.arrived;
  sitePage.emit("waf:captcha:open");
  gate.resolve({ Kh: [mangaDto()], kZ: { mM: 1, DP: 1 } });

  assert.deepEqual(JSON.parse(await pending), { ok: true, data: { items: [summary], meta: { page: 1, last_page: 1 } } });
  assert.deepEqual(asked, [null]);
});

test("a host reply of false rejects the call with a challenge", async () => {
  await reset();
  const site = writeSite("declined");
  const gate = await parkSiteCall("o3");
  const sitePage = page({ resources: [site.href], showPage: () => false });
  const pending = runRaw(request(site, { query: "page=1" }));

  await gate.arrived;
  sitePage.emit("waf:captcha:open");

  assert.deepEqual(JSON.parse(await pending), { ok: false, kind: "challenge" });
});

test("a rejecting, throwing or missing host handler counts as a refusal", async () => {
  const handlers = [
    ["rejecting", () => Promise.reject(new Error("refused"))],
    ["throwing", () => {
      throw new Error("no handler");
    }],
    ["missing", undefined]
  ];
  for (const [label, showPage] of handlers) {
    await reset();
    const site = writeSite(`refusal-${label}`);
    const gate = await parkSiteCall("o3");
    const sitePage = page({ resources: [site.href], showPage });
    const pending = runRaw(request(site, { query: "page=1" }));

    await gate.arrived;
    sitePage.emit("waf:captcha:open");

    assert.deepEqual(JSON.parse(await pending), { ok: false, kind: "challenge" }, label);
  }
});

test("a verification dialog already open when the call starts asks the host, and waits when shown", async () => {
  await reset();
  const site = writeSite("already-shown");
  const gate = await parkSiteCall("o3");
  const asked = [];
  page({ resources: [site.href], frames: [dialogFrame(`${SITE_ORIGIN}/@waf/challenge`)], showPage: value => {
    asked.push(value);
    return true;
  } });
  const pending = runRaw(request(site, { query: "page=1" }));

  await gate.arrived;
  gate.resolve({ Kh: [], kZ: { mM: 1, DP: 1 } });

  assert.deepEqual(JSON.parse(await pending), { ok: true, data: { items: [], meta: { page: 1, last_page: 1 } } });
  assert.deepEqual(asked, [null]);
});

test("a verification dialog already open when the call starts is refused when the host declines", async () => {
  await reset();
  const site = writeSite("already-declined");
  await parkSiteCall("o3");
  page({ resources: [site.href], frames: [dialogFrame(`${SITE_ORIGIN}/@waf/challenge`)], showPage: () => false });

  assert.deepEqual(JSON.parse(await runRaw(request(site, { query: "page=1" }))), { ok: false, kind: "challenge" });
});

test("a call without a verification frame never asks the host, even with other frames open", async () => {
  await reset();
  const site = writeSite("quiet");
  (await recorder).responses.set("o3", () => ({ Kh: [], kZ: { mM: 1, DP: 1 } }));
  const asked = [];
  page({
    resources: [site.href],
    frames: [pageFrame("https://other.invalid/@waf/challenge"), pageFrame(`${SITE_ORIGIN}/title/hid-1`)],
    showPage: value => {
      asked.push(value);
      return true;
    }
  });

  assert.deepEqual(await run(request(site, { query: "page=1" })), { ok: true, data: { items: [], meta: { page: 1, last_page: 1 } } });
  assert.deepEqual(asked, []);
});

test("a same-origin verification route outside a dialog does not ask the host", async () => {
  await reset();
  const site = writeSite("loose-challenge");
  (await recorder).responses.set("o3", () => ({ Kh: [], kZ: { mM: 1, DP: 1 } }));
  const asked = [];
  page({
    resources: [site.href],
    frames: [pageFrame(`${SITE_ORIGIN}/@waf/challenge`)],
    showPage: value => {
      asked.push(value);
      return true;
    }
  });

  assert.deepEqual(await run(request(site, { query: "page=1" })), { ok: true, data: { items: [], meta: { page: 1, last_page: 1 } } });
  assert.deepEqual(asked, []);
});

test("a verification the user cancels after the host showed it is still a challenge", async () => {
  await reset();
  const site = writeSite("cancelled");
  const gate = await parkSiteCall("o3");
  const sitePage = page({ resources: [site.href], showPage: () => true });
  const pending = runRaw(request(site, { query: "page=1" }));

  await gate.arrived;
  sitePage.emit("waf:captcha:open");
  gate.reject(new Error("waf_captcha_cancelled"));

  assert.deepEqual(JSON.parse(await pending), { ok: false, kind: "challenge" });
});

test("a cancelled captcha is a challenge, and HTTP failures keep their status without the body", async () => {
  await reset();
  const site = writeSite("failures");
  page({ resources: [site.href] });
  (await recorder).responses.set("o3", () => {
    throw new Error("waf_captcha_cancelled");
  });
  assert.deepEqual(await run(request(site, { query: "page=1" })), { ok: false, kind: "challenge" });

  (await recorder).responses.set("o3", () => {
    throw Object.assign(new Error("forbidden"), { response: { status: 403, data: { secret: "do-not-leak" } } });
  });
  const raw = await runRaw(request(site, { query: "page=1" }));
  assert.deepEqual(JSON.parse(raw), { ok: false, kind: "http", status: 403 });
  assert.equal(raw.includes("do-not-leak"), false);
});

test("a chapter without a required flag fails the whole operation rather than returning partial items", async () => {
  await reset();
  const site = writeSite("partial");
  (await recorder).responses.set("iF", () => ({
    Kh: [{ Lc: 501, p7: null, uR: 1, K3: "", jI: 0, Ix: null }],
    kZ: { mM: 1, DP: 2 }
  }));
  page({ resources: [site.href] });

  assert.deepEqual(await run(request(site, { operation: "chapters", key: "hid-1", query: "page=1" })), { ok: false, kind: "shape" });
});

test("page metadata is required: a missing current page is not defaulted", async () => {
  await reset();
  const site = writeSite("meta");
  (await recorder).responses.set("o3", () => ({ Kh: [], kZ: { DP: 3 } }));
  page({ resources: [site.href] });

  assert.deepEqual(await run(request(site, { query: "page=1" })), { ok: false, kind: "shape" });
});

test("query names cannot reach prototype machinery", async () => {
  await reset();
  const site = writeSite("proto");
  page({ resources: [site.href] });

  assert.deepEqual(await run(request(site, { query: "__proto__[polluted]=1" })), { ok: false, kind: "query" });
  assert.equal({}.polluted, undefined);
  assert.equal((await recorder).calls.length, 0);
});

test("a page on another origin is refused before any client lookup", async () => {
  await reset();
  const site = writeSite("origin");
  page({ origin: "https://other.invalid", resources: [site.href] });

  assert.deepEqual(await run(request(site)), { ok: false, kind: "origin" });
  assert.equal((await recorder).calls.length, 0);
});
