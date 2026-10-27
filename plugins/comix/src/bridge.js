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
    list: () => invoke(api.f, "h0", parameters(request.query)),
    top: () => invoke(api.f, "Nf", parameters(request.query)),
    detail: () => invoke(api.f, "kX", key()),
    chapters: () => invoke(api.f, "dq", key(), parameters(request.query)),
    pages: () => invoke(api.n, "kX", key())
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

// DTO normalization. The site's mappers emit obfuscated keys; these are the mapper outputs:
//   item   wl: sk=hid, Sl=title, zD=type, Gh=status, wM=synopsis, FJ=latestChapter,
//          _5={VP:medium, mJ:large}; detail Al adds WD=authors, A_=artists, Mf=genres ({rr,Sl})
//   meta   ol: cC=page, kk=lastPage (the envelope t9/zV is adapted only in paged())
//   chapter bn: yH=id, HU=groupId, _x=number, ek=name, hi=votes, e8=isOfficial, Jj={HU,ek}|null
//   pages  n.kX: rB=[{q4=baseUrl+url}]; q4 is resolved against the page URL
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
  return { small: null, medium: optional(value.VP, text), large: optional(value.mJ, text) };
}

function mangaSummary(item) {
  object(item);
  return {
    hash_id: text(item.sk),
    title: text(item.Sl),
    type: text(item.zD),
    status: text(item.Gh),
    synopsis: optional(item.wM, text),
    latest_chapter: optional(item.FJ, number),
    poster: optional(item._5, poster)
  };
}

function term(item) {
  object(item);
  return { term_id: integer(item.rr), title: text(item.Sl) };
}

function mangaDetail(item) {
  return {
    ...mangaSummary(item),
    author: list(object(item).WD, term),
    artist: list(item.A_, term),
    genre: list(item.Mf, term)
  };
}

function chapter(item) {
  object(item);
  return {
    chapter_id: integer(item.yH),
    scanlation_group_id: optional(item.HU, integer),
    number: number(item._x),
    name: optional(item.ek, text),
    votes: integer(item.hi),
    is_official: flag(item.e8),
    scanlation_group: item.Jj ? { name: text(object(item.Jj).ek) } : null
  };
}

function paged(raw, convert) {
  object(raw);
  const meta = object(raw.zV);
  return {
    items: list(raw.t9, convert),
    meta: { page: integer(meta.cC), last_page: integer(meta.kk) }
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
      return { images: list(object(raw).rB, image => ({ url: absoluteUrl(object(image).q4) })) };
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
