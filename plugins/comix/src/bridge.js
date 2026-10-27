// Read-only operations through the site's own API client, run as an async function body
// (the `//# ito-async` marker). The host prepends `const request = {...}` with:
//   origin     page origin this plugin serves (https://comix.to)
//   revision   SHA-256 hex of the API client module that the DTO mappings were derived from
//   operation  "list" | "top" | "detail" | "chapters" | "pages" (fixed allowlist)
//   key        manga hash id, or chapter id for "pages"
//   query      literal query string; decoded into the same params object the site client receives
// Every path returns a non-empty JSON document: {"ok":true,"data":...} or {"ok":false,"kind":...}.

const CHALLENGE_EVENT = "waf:captcha:open";
// Route the site's verification dialog loads in its frame; the dialog exists only while open.
const VERIFICATION_PATH = "/@waf/challenge";
const SITE_MODULE = /\/apicore-[^/]+\.js$/;
const MODULE_REFERENCE = /["'`]((?:\.{1,2}\/)?[^"'`\s]*apicore-[^"'`\s]*\.js)["'`]/g;
const FORBIDDEN_KEYS = ["__proto__", "prototype", "constructor"];
const OPERATIONS = new Set(["list", "top", "detail", "chapters", "pages"]);

class Refusal extends Error {
  constructor(kind) {
    super(kind);
    this.kind = kind;
  }
}

const refuse = kind => new Refusal(kind);

function classify(error) {
  if (error instanceof Refusal) return { kind: error.kind };
  const status = error?.response?.status;
  if (Number.isInteger(status)) return { kind: "http", status };
  if (error?.message === "waf_captcha_cancelled") return { kind: "challenge" };
  return { kind: "error" };
}

// Locates the one client module the page loaded: performance entries, preload links and
// script tags first; then, only if none, module references inside same-origin module roots.
async function locateModule(origin) {
  const found = new Set();
  const offer = (reference, base = location.href) => {
    const url = new URL(reference, base);
    if (url.origin === origin && SITE_MODULE.test(url.pathname)) found.add(url.href);
  };
  for (const entry of performance.getEntriesByType("resource")) offer(entry.name);
  for (const link of document.querySelectorAll('link[rel~="modulepreload"]')) offer(link.href);
  for (const script of document.querySelectorAll("script[src]")) offer(script.src);
  if (found.size === 0) {
    const roots = [
      ...document.querySelectorAll('script[type="module"][src]'),
      ...document.querySelectorAll('link[rel~="modulepreload"]')
    ].map(element => new URL(element.src || element.href, location.href).href);
    for (const root of roots) {
      if (new URL(root).origin !== origin) continue;
      const response = await fetch(root).catch(() => null);
      if (!response?.ok) continue;
      for (const match of (await response.text()).matchAll(MODULE_REFERENCE)) offer(match[1], root);
    }
  }
  if (found.size !== 1) throw refuse("missing-module");
  return [...found][0];
}

async function verifyRevision(href, revision) {
  const response = await fetch(href).catch(() => null);
  if (!response?.ok) throw refuse("missing-module");
  const digest = await crypto.subtle.digest("SHA-256", await response.arrayBuffer());
  const actual = Array.from(new Uint8Array(digest), byte => byte.toString(16).padStart(2, "0")).join("");
  if (actual !== revision) throw refuse("revision");
}

async function importSite(href) {
  try {
    return await import(href);
  } catch (_) {
    throw refuse("missing-module");
  }
}

// Same decoding the site's client sees for a URL query: numbers become numbers except
// search text, bracketed names become nested objects and arrays.
function parameters(query) {
  if (typeof query !== "string") throw refuse("query");
  const root = Object.create(null);
  for (const [key, raw] of new URLSearchParams(query)) {
    const parts = key.replace(/\]/g, "").split("[");
    if (parts.some(part => FORBIDDEN_KEYS.includes(part))) throw refuse("query");
    const number = /^-?\d+$/.test(raw) ? Number(raw) : NaN;
    const value = key !== "keyword" && Number.isSafeInteger(number) ? number : raw;
    let target = root;
    for (let index = 0; index < parts.length; index++) {
      const part = parts[index];
      if (index === parts.length - 1) {
        if (part === "") {
          if (!Array.isArray(target)) throw refuse("query");
          target.push(value);
        } else if (target[part] === undefined) {
          target[part] = value;
        } else if (Array.isArray(target[part])) {
          target[part].push(value);
        } else {
          target[part] = [target[part], value];
        }
      } else {
        target[part] ??= parts[index + 1] === "" ? [] : Object.create(null);
        target = target[part];
      }
    }
  }
  return root;
}

function invoke(owner, name, ...args) {
  if (typeof owner?.[name] !== "function") throw refuse("missing-facade");
  return owner[name](...args);
}

// The site mounts its verification dialog only while it is open, and that dialog's frame loads
// the verification route. The frame is the signal: the open event does not fire again for a
// dialog left open after a close or timeout, and the dialog text is not a stable identifier.
function verificationOpen() {
  const current = new URL(location.href);
  return [...document.querySelectorAll('[role="dialog"] iframe')].some(frame => {
    try {
      const url = new URL(frame.src, current);
      return url.origin === current.origin && url.pathname === VERIFICATION_PATH;
    } catch (_) {
      return false;
    }
  });
}

// Asks the host to show the site's verification page. True when shown or already shown; a missing,
// rejecting or throwing handler counts as refused.
async function showVerification() {
  try {
    return (await window.webkit?.messageHandlers?.itoShowPage?.postMessage(null)) === true;
  } catch (_) {
    return false;
  }
}

// Calls the site's facades. While its verification dialog is open the host is asked to show it,
// and the call keeps waiting for the site's own result; it is refused only if the host declines.
async function perform(api, request) {
  const key = () => {
    if (typeof request.key !== "string" || request.key === "") throw refuse("operation");
    return request.key;
  };
  const calls = {
    list: () => invoke(api.f, "o3", parameters(request.query)),
    top: () => invoke(api.f, "nb", parameters(request.query)),
    detail: () => invoke(api.f, "E5", key()),
    chapters: () => invoke(api.f, "iF", key(), parameters(request.query)),
    pages: () => invoke(api.n, "E5", key())
  };
  let rejectChallenge;
  const challenged = new Promise((_, reject) => {
    rejectChallenge = reject;
  });
  const onChallenge = async () => {
    if (!(await showVerification())) rejectChallenge(refuse("challenge"));
  };
  window.addEventListener(CHALLENGE_EVENT, onChallenge);
  try {
    if (verificationOpen()) onChallenge();
    return await Promise.race([Promise.resolve().then(calls[request.operation]), challenged]);
  } finally {
    window.removeEventListener(CHALLENGE_EVENT, onChallenge);
  }
}

// DTO normalization. The current pinned client emits:
//   item   f.o3/f.nb/f.E5: qa=hid, Rv=title, vI=type, QZ=status, vl=synopsis,
//          Zk=latestChapter, G3={Qq:medium, XN:large}
//          detail adds Gd=authors, VC=artists, xI=genres ({Xl:id,Rv:title})
//   meta   list envelope {Kh:items,kZ:meta}; meta mM=page, DP=lastPage
//   chapter f.iF: Lc=id, p7=groupId, uR=number, K3=name, jI=votes,
//                jq=isOfficial, Ix={p7,K3}|null
//   pages  n.E5: CP=[{_u:absolute URL, ...}]
// Required fields are never defaulted: a missing or mistyped field fails the operation.
function object(value) {
  if (value === null || typeof value !== "object") throw refuse("shape");
  return value;
}

function text(value) {
  if (typeof value !== "string") throw refuse("shape");
  return value;
}

function integer(value) {
  if (!Number.isSafeInteger(value)) throw refuse("shape");
  return value;
}

function number(value) {
  if (typeof value !== "number" || !Number.isFinite(value)) throw refuse("shape");
  return value;
}

function flag(value) {
  if (typeof value !== "boolean") throw refuse("shape");
  return value;
}

function optional(value, convert) {
  return value === undefined || value === null ? null : convert(value);
}

function list(value, convert) {
  if (!Array.isArray(value)) throw refuse("shape");
  return value.map(item => convert(item));
}

function absoluteUrl(value) {
  let url;
  try {
    url = new URL(text(value), location.href);
  } catch (_) {
    throw refuse("shape");
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") throw refuse("shape");
  return url.href;
}

function poster(value) {
  object(value);
  return { small: null, medium: optional(value.Qq, text), large: optional(value.XN, text) };
}

function mangaSummary(item) {
  object(item);
  return {
    hash_id: text(item.qa),
    title: text(item.Rv),
    type: text(item.vI),
    status: text(item.QZ),
    synopsis: optional(item.vl, text),
    latest_chapter: optional(item.Zk, number),
    poster: optional(item.G3, poster)
  };
}

function term(item) {
  object(item);
  return { title: text(item.Rv) };
}

function termList(value) {
  return optional(value, items => list(items, term));
}

function mangaDetail(item) {
  return {
    ...mangaSummary(item),
    author: termList(item.Gd),
    artist: termList(item.VC),
    genre: termList(item.xI)
  };
}

function chapter(item) {
  object(item);
  return {
    chapter_id: integer(item.Lc),
    scanlation_group_id: optional(item.p7, integer),
    number: number(item.uR),
    name: optional(item.K3, text),
    votes: integer(item.jI),
    is_official: flag(item.jq),
    scanlation_group: item.Ix ? { name: text(object(item.Ix).K3) } : null
  };
}

function paged(raw, convert) {
  object(raw);
  const meta = object(raw.kZ);
  return {
    items: list(raw.Kh, convert),
    meta: { page: integer(meta.mM), last_page: integer(meta.DP) }
  };
}

function normalize(operation, raw) {
  switch (operation) {
    case "list":
      return paged(raw, mangaSummary);
    case "top":
      return list(raw, mangaSummary);
    case "detail":
      return mangaDetail(raw);
    case "chapters":
      return paged(raw, chapter);
    case "pages":
      return { images: list(object(raw).CP, image => ({ url: absoluteUrl(object(image)._u) })) };
    default:
      throw refuse("operation");
  }
}

async function run(request) {
  if (!OPERATIONS.has(request.operation)) throw refuse("operation");
  if (location.origin !== request.origin) throw refuse("origin");
  const href = await locateModule(request.origin);
  await verifyRevision(href, request.revision);
  const api = await importSite(href);
  return normalize(request.operation, await perform(api, request));
}

try {
  return JSON.stringify({ ok: true, data: await run(request) });
} catch (error) {
  const { kind, status } = classify(error);
  return JSON.stringify(status === undefined ? { ok: false, kind } : { ok: false, kind, status });
}
