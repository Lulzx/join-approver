import { openPath } from "@tauri-apps/plugin-opener";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import {
  api,
  type AuthState,
  type Channel,
  type FloodEvent,
  type JobEvent,
  type Me,
  type Outcome,
  type Person,
  type Report,
  type Stats,
} from "./api";

// ---------- tiny DOM helpers (text only: names from Telegram are untrusted) ----------

type Attrs = Record<string, string | boolean | number | EventListener | undefined>;
type Child = Node | string | number | null | undefined | false;

function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: Attrs = {},
  ...children: Child[]
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === undefined || v === false) continue;
    if (k.startsWith("on") && typeof v === "function") el.addEventListener(k.slice(2), v);
    else if (k === "class") el.className = String(v);
    else if (v === true) el.setAttribute(k, "");
    else el.setAttribute(k, String(v));
  }
  for (const c of children) {
    if (c === null || c === undefined || c === false) continue;
    el.append(c instanceof Node ? c : document.createTextNode(String(c)));
  }
  return el;
}

const nf = new Intl.NumberFormat();
const fmt = (n: number) => nf.format(n);
const signed = (n: number) => (n > 0 ? "+" : n < 0 ? "−" : "±") + fmt(Math.abs(n));
const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });

function ago(unix: number): string {
  const s = unix - Date.now() / 1000;
  const units: [Intl.RelativeTimeFormatUnit, number][] = [
    ["year", 31536000], ["month", 2592000], ["week", 604800], ["day", 86400], ["hour", 3600], ["minute", 60],
  ];
  for (const [unit, size] of units) if (Math.abs(s) >= size) return rtf.format(Math.round(s / size), unit);
  return "just now";
}

function duration(minutes: number): string {
  if (!isFinite(minutes) || minutes <= 0) return "—";
  if (minutes < 1) return "under a minute";
  if (minutes < 60) return `${Math.round(minutes)} min`;
  const hrs = Math.floor(minutes / 60);
  return `${hrs} h ${Math.round(minutes % 60)} min`;
}

function initials(title: string): string {
  const words = title.replace(/[^\p{L}\p{N} ]/gu, "").trim().split(/\s+/);
  return (words[0]?.[0] ?? "#").concat(words[1]?.[0] ?? "").toUpperCase();
}

function hue(id: number): number {
  return Math.abs(id * 47) % 360;
}

const icon = {
  check: "✓",
  cross: "✕",
  dot: "•",
};

// A sheet-style dialog: the webview's own confirm()/alert() are unreliable.
function ask(title: string, body: string, ok = "OK", cancel: string | null = "Cancel"): Promise<boolean> {
  return new Promise((resolve) => {
    const close = (v: boolean) => {
      overlay.remove();
      document.removeEventListener("keydown", key);
      resolve(v);
    };
    const key = (e: KeyboardEvent) => { if (e.key === "Escape") close(false); };
    const okBtn = h("button", { class: "btn primary", onclick: () => close(true) }, ok);
    const overlay = h("div", { class: "overlay" },
      h("div", { class: "sheet", role: "alertdialog", "aria-modal": "true", "aria-labelledby": "sheet-title" },
        h("h3", { id: "sheet-title" }, title),
        ...body.split("\n\n").map((p) => h("p", {}, p)),
        h("div", { class: "sheet-actions" },
          cancel ? h("button", { class: "btn", onclick: () => close(false) }, cancel) : null,
          okBtn)));
    document.addEventListener("keydown", key);
    document.body.append(overlay);
    okBtn.focus();
  });
}

// ---------- state ----------

type PendingState = { count: number; items: Person[]; more: boolean; loading: boolean; error: string };

/** A round picture: the real photo, else Telegram's inline preview, else initials. */
function avatar(id: number, name: string, cls: string, photo?: string | null): HTMLElement {
  const el = h("span", { class: `avatar ${cls}`, style: `--h:${hue(id)}` });
  if (photo) el.append(h("img", { src: photo, alt: "", draggable: "false" }));
  else el.textContent = initials(name);
  return el;
}

const channelPic = (c: Channel) => state.photos.get(c.id) ?? c.thumb;

type RunState = {
  channelId: number;
  title: string;
  startCount: number;
  total: number;
  approved: number;
  parked: number;
  attempted: number;
  subscribers: number | null;
  perMinute: number;
  delayMs: number;
  status: string;
  stopping: boolean;
  auditPath: string;
  /** 0 = everyone. */
  limit: number;
};

const state = {
  platform: "macos",
  auth: null as AuthState | null,
  me: null as Me | null,
  channels: [] as Channel[],
  channelsLoading: false,
  channelsError: "",
  stats: new Map<number, Stats | "loading" | "error">(),
  selected: null as number | null,
  pending: new Map<number, PendingState>(),
  photos: new Map<number, string>(),
  mePhoto: null as string | null,
  query: "",
  run: null as RunState | null,
  report: null as { channelId: number; report: Report } | null,
  flood: null as { until: number; seconds: number; delayMs: number } | null,
  opts: { verifyEach: true, limit: 0, delayMs: 1500 },
  update: null as null | { version: string; status: "available" | "downloading" | "error"; done: number; total: number; error: string },
};

const root = document.getElementById("app")!;

// ---------- login ----------

function loginView(): HTMLElement {
  const needsPassword = state.auth?.state === "passwordNeeded";
  let step: "phone" | "code" | "password" = needsPassword ? "password" : "phone";
  const box = h("div", { class: "login-card" });
  const error = h("p", { class: "form-error", role: "alert" });

  const show = () => {
    box.replaceChildren();
    error.textContent = "";
    box.append(
      h("div", { class: "app-mark" }, h("span", {}, icon.check)),
      h("h1", {}, "Join Approver"),
    );
    if (step === "phone") {
      const input = h("input", {
        type: "tel", placeholder: "+91 98765 43210", autocomplete: "tel", required: true,
        "aria-label": "Phone number",
      });
      const form = h("form", {}, input, error, h("button", { class: "btn primary wide", type: "submit" }, "Send code"));
      form.addEventListener("submit", async (e) => {
        e.preventDefault();
        await busy(form, async () => {
          await api.sendCode(input.value);
          step = "code";
          show();
        });
      });
      box.append(
        h("p", { class: "lede" }, "Sign in with the Telegram account that administers your channel."),
        form,
        h("p", { class: "fineprint" },
          "Telegram sends the login code to your Telegram app. The login stays on this computer."),
      );
      setTimeout(() => input.focus());
    } else if (step === "code") {
      const input = h("input", {
        inputmode: "numeric", placeholder: "12345", autocomplete: "one-time-code", required: true,
        "aria-label": "Login code", class: "code",
      });
      const form = h("form", {}, input, error, h("button", { class: "btn primary wide", type: "submit" }, "Sign in"));
      form.addEventListener("submit", async (e) => {
        e.preventDefault();
        await busy(form, async () => {
          const next = await api.signIn(input.value);
          if (next.state === "passwordNeeded") {
            state.auth = next;
            step = "password";
            show();
          } else await signedIn(next);
        });
      });
      box.append(
        h("p", { class: "lede" }, "Enter the code Telegram just sent to your Telegram app."),
        form,
        h("button", { class: "btn link", onclick: () => { step = "phone"; show(); } }, "Use a different number"),
      );
      setTimeout(() => input.focus());
    } else {
      const hint = state.auth?.state === "passwordNeeded" ? state.auth.hint : null;
      const input = h("input", {
        type: "password", placeholder: hint ? `Hint: ${hint}` : "Password", required: true,
        autocomplete: "current-password", "aria-label": "Two-step verification password",
      });
      const form = h("form", {}, input, error, h("button", { class: "btn primary wide", type: "submit" }, "Continue"));
      form.addEventListener("submit", async (e) => {
        e.preventDefault();
        await busy(form, async () => signedIn(await api.checkPassword(input.value)));
      });
      box.append(h("p", { class: "lede" }, "This account has two-step verification. Enter its password."), form);
      setTimeout(() => input.focus());
    }
  };

  async function busy(form: HTMLFormElement, fn: () => Promise<void>) {
    const btn = form.querySelector("button")!;
    const inputs = form.querySelectorAll("input");
    btn.disabled = true;
    inputs.forEach((i) => (i.disabled = true));
    btn.classList.add("busy");
    try {
      await fn();
    } catch (err) {
      error.textContent = String(err);
      btn.disabled = false;
      inputs.forEach((i) => (i.disabled = false));
      btn.classList.remove("busy");
      inputs[0]?.focus();
    }
  }

  show();
  return h("main", { class: "login", "data-tauri-drag-region": true }, box);
}

async function signedIn(next: AuthState) {
  state.auth = next;
  if (next.state === "signedIn") state.me = next.me;
  render();
  await loadChannels();
}

// ---------- channels ----------

// ---------- local cache ----------

type Snapshot = {
  me: Me;
  mePhoto: string | null;
  channels: Channel[];
  stats: [number, Stats][];
  photos: [number, string][];
  selected: number | null;
};

let persistTimer = 0;
/** Saves what's on screen, so the next launch opens on it at once. */
function persist() {
  clearTimeout(persistTimer);
  persistTimer = window.setTimeout(() => {
    if (!state.me) return;
    const snap: Snapshot = {
      me: state.me,
      mePhoto: state.mePhoto,
      channels: state.channels,
      stats: [...state.stats].filter((e): e is [number, Stats] => typeof e[1] === "object"),
      photos: [...state.photos],
      selected: state.selected,
    };
    api.cacheWrite("snapshot", snap).catch(() => {});
  }, 500);
}

function restore(snap: Snapshot) {
  state.auth = { state: "signedIn", me: snap.me };
  state.me = snap.me;
  state.mePhoto = snap.mePhoto;
  state.channels = snap.channels;
  state.stats = new Map(snap.stats);
  state.photos = new Map(snap.photos);
  state.selected = snap.selected ?? snap.channels[0]?.id ?? null;
}

/** A channel as the backend finds it, so the list fills in as it loads. */
function onChannelFound(c: Channel) {
  const i = state.channels.findIndex((x) => x.id === c.id);
  if (i >= 0) state.channels[i] = c;
  else state.channels.push(c);
  if (state.selected === null) {
    state.selected = c.id;
    prioritize(c.id);
  }
  state.channelsLoading = false;
  renderSoon();
  persist();
}

async function loadChannels() {
  // With a saved list on screen, refresh quietly behind it.
  state.channelsLoading = !state.channels.length;
  state.channelsError = "";
  render();
  try {
    const fresh = await api.listChannels();
    // A changed photo id shows up as a new thumb: drop the stale photo.
    for (const c of fresh) {
      const old = state.channels.find((x) => x.id === c.id);
      if (old && old.thumb !== c.thumb) state.photos.delete(c.id);
    }
    state.channels = fresh;
    persist();
    sortChannels();
    if (state.selected === null || !state.channels.some((c) => c.id === state.selected)) {
      state.selected = state.channels[0]?.id ?? null;
    }
  } catch (e) {
    state.channelsError = String(e);
  }
  state.channelsLoading = false;
  render();
  // Counts and photos for every channel, in the background; whatever is
  // clicked jumps the queue (see `prioritize`).
  for (const c of state.channels) {
    enqueue(`stats:${c.id}`, () => refreshStats(c.id));
    if (c.hasPhoto && !state.photos.has(c.id)) enqueue(`photo:${c.id}`, () => loadPhoto(c.id));
  }
  if (!state.mePhoto) enqueue("photo:me", loadMePhoto);
  if (state.selected !== null) prioritize(state.selected);
  enqueue("sort", async () => { sortChannels(); if (!state.run) render(); persist(); });
}

// ---------- background queue ----------
// Every call shares one pace in the backend, so loading is one task at a
// time; a queue lets the selected channel go next instead of waiting for
// the whole sweep.

type Task = { key: string; run: () => Promise<void> };
const queue: Task[] = [];
let working = false;

function enqueue(key: string, run: () => Promise<void>, urgent = false) {
  const i = queue.findIndex((t) => t.key === key);
  if (i >= 0) {
    if (!urgent) return;
    queue.splice(i, 1);
  }
  if (urgent) queue.unshift({ key, run });
  else queue.push({ key, run });
  if (!working) work();
}

async function work() {
  working = true;
  while (queue.length) {
    const t = queue.shift()!;
    try { await t.run(); } catch { /* each task reports its own errors */ }
  }
  working = false;
}

/** The clicked channel: its counts (and first page) load next. */
function prioritize(id: number) {
  const c = state.channels.find((x) => x.id === id);
  if (!c) return;
  if (c.hasPhoto && !state.photos.has(id)) enqueue(`photo:${id}`, () => loadPhoto(id), true);
  enqueue(`stats:${id}`, () => refreshStats(id), true);
}

function select(id: number) {
  state.selected = id;
  prioritize(id);
  render();
  persist();
}

async function loadPhoto(id: number) {
  const url = await api.channelPhoto(id).catch(() => null);
  if (!url) return;
  state.photos.set(id, url);
  if (!state.run || state.run.channelId !== id) renderSoon();
  persist();
}

async function loadMePhoto() {
  state.mePhoto = await api.mePhoto().catch(() => null);
  if (state.mePhoto) { renderSoon(); persist(); }
}

const pendingOf = (id: number) => {
  const s = state.stats.get(id);
  return s && typeof s === "object" ? s.pending : 0;
};

/** Channels that can be approved first, the most pending at the top. */
function sortChannels() {
  state.channels.sort((a, b) =>
    Number(b.canApprove) - Number(a.canApprove) || pendingOf(b.id) - pendingOf(a.id) || a.title.localeCompare(b.title));
}

// Accent- and case-insensitive: "bazar" finds "Bazaar", "cafe" finds "Café".
const fold = (s: string) => s.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();

function matches(c: Channel, q: string): boolean {
  if (!q) return true;
  const needle = fold(q.trim().replace(/^@/, ""));
  return fold(c.title).includes(needle) || (c.username ? fold(c.username).includes(needle) : false);
}

/** Filters the list in place, so typing never rebuilds (and blurs) the field. */
function applySearch() {
  const q = state.query;
  let shown = 0;
  document.querySelectorAll<HTMLElement>(".channel[data-id]").forEach((el) => {
    const c = state.channels.find((x) => x.id === Number(el.dataset.id));
    const hit = !!c && matches(c, q);
    el.hidden = !hit;
    if (hit) shown++;
  });
  const none = document.getElementById("no-match");
  if (none) none.hidden = shown > 0 || !q;
}

function channelSub(c: Channel): string {
  return `${c.role === "owner" ? "Owner" : "Admin"} · ${c.isGroup ? "Group" : "Channel"}`;
}

async function refreshStats(id: number) {
  // A saved count stays up while the new one loads.
  if (typeof state.stats.get(id) !== "object") state.stats.set(id, "loading");
  updateSidebarBadges();
  if (id === state.selected) render();
  try {
    state.stats.set(id, await api.channelStats(id));
    persist();
  } catch {
    state.stats.set(id, "error");
  }
  updateSidebarBadges();
  if (id === state.selected && !state.run) render();
}

function sidebar(): HTMLElement {
  const list = h("nav", { class: "channel-list", "aria-label": "Channels" });
  const paused = state.flood && state.flood.until > Date.now() && !state.run;
  if (state.channelsLoading) {
    list.append(h("div", { class: "side-note" }, h("span", { class: "spinner" }),
      paused ? `Telegram asked for a pause. Continuing in ${Math.ceil((state.flood!.until - Date.now()) / 1000)} s…` : "Loading channels…"));
  }
  else if (state.channelsError) list.append(h("div", { class: "side-note error" }, state.channelsError));
  else if (!state.channels.length)
    list.append(h("div", { class: "side-note" }, "This account doesn't own or administer any channels or groups."));
  for (const c of state.channels) {
    const running = state.run?.channelId === c.id;
    list.append(
      h("button", {
        class: "channel" + (c.id === state.selected ? " selected" : "") + (c.canApprove ? "" : " dim"),
        "data-id": c.id,
        title: c.canApprove ? undefined : "Your admin role can't approve join requests here",
        onclick: () => select(c.id),
      },
        avatar(c.id, c.title, "", channelPic(c)),
        h("span", { class: "channel-text" },
          h("span", { class: "channel-title" }, c.title),
          h("span", { class: "channel-sub" }, channelSub(c)),
        ),
        running ? h("span", { class: "badge live" }, h("span", { class: "spinner small" })) : h("span", { class: "badge", "data-badge": c.id }),
      ),
    );
  }
  list.append(h("div", { class: "side-note", id: "no-match", hidden: true }, "No channels match."));

  const search = h("input", {
    type: "search", class: "search", placeholder: "Search", value: state.query, "aria-label": "Search channels",
    spellcheck: "false", autocomplete: "off",
  });
  search.addEventListener("input", () => { state.query = search.value; applySearch(); });
  search.addEventListener("keydown", (e) => {
    if (e.key === "Escape") { search.value = ""; state.query = ""; applySearch(); search.blur(); }
    if (e.key === "Enter") {
      // Enter opens the first match.
      const first = state.channels.find((c) => matches(c, state.query));
      if (first) select(first.id);
    }
  });

  const me = state.me;
  return h("aside", { class: "sidebar" },
    h("div", { class: "side-head", "data-tauri-drag-region": true },
      h("span", { class: "side-title" }, "Channels"),
      h("button", { class: "icon-btn", title: "Refresh", "aria-label": "Refresh channels", onclick: () => loadChannels() }, "↻"),
    ),
    state.channels.length > 0 ? h("div", { class: "search-wrap" }, search) : null,
    list,
    updateCard(),
    h("footer", { class: "account" },
      avatar(me?.name.length ?? 1, me?.name ?? "?", "small", state.mePhoto),
      h("span", { class: "channel-text" },
        h("span", { class: "channel-title" }, me?.name ?? ""),
        h("span", { class: "channel-sub" }, me?.username ? `@${me.username}` : me?.phone ? `+${me.phone}` : ""),
      ),
      h("button", { class: "btn small ghost", onclick: signOut, disabled: !!state.run }, "Sign out"),
    ),
  );
}

function updateSidebarBadges() {
  document.querySelectorAll<HTMLElement>("[data-badge]").forEach((el) => {
    const s = state.stats.get(Number(el.dataset.badge));
    el.textContent = s && typeof s === "object" && s.pending > 0 ? fmt(s.pending) : "";
    el.classList.toggle("empty", !(s && typeof s === "object" && s.pending > 0));
  });
}

async function signOut() {
  if (!(await ask("Sign out?", "You'll need a new login code from Telegram to sign back in.", "Sign out"))) return;
  await api.signOut();
  Object.assign(state, { auth: { state: "signedOut" }, me: null, mePhoto: null, channels: [], selected: null, report: null });
  state.stats.clear();
  state.photos.clear();
  state.pending.clear();
  render();
}

// ---------- channel pane ----------

function channelPane(c: Channel): HTMLElement {
  if (state.run?.channelId === c.id) return runPane();
  if (state.report?.channelId === c.id) return reportPane(state.report.report);

  const s = state.stats.get(c.id);
  const stats = s && typeof s === "object" ? s : null;
  const pending = stats?.pending ?? 0;
  const busyElsewhere = !!state.run;

  const statCard = (label: string, value: string, sub?: Child) =>
    h("div", { class: "stat" }, h("span", { class: "stat-label" }, label), h("span", { class: "stat-value" }, value), sub ? h("span", { class: "stat-sub" }, sub) : null);

  const loadingValue = s === "error" ? "—" : "…";

  // Options
  const o = state.opts;
  const mode = h("div", { class: "segmented", role: "radiogroup", "aria-label": "Confirmation" },
    ...([[true, "Confirm each person"], [false, "Confirm at the end"]] as const).map(([v, label]) =>
      h("button", {
        role: "radio", "aria-checked": String(o.verifyEach === v), class: o.verifyEach === v ? "on" : "",
        onclick: () => { o.verifyEach = v; render(); },
      }, label)),
  );
  const perPerson = ((o.verifyEach ? 2 : 1) * o.delayMs * 1.05) / 1000;
  const limit = Math.min(o.limit || pending, pending);
  const eta = (limit * perPerson) / 60;

  const pace = h("input", {
    type: "range", min: 500, max: 3000, step: 100, value: o.delayMs, "aria-label": "Pause between calls",
  });
  const paceLabel = h("span", { class: "range-value" }, `${(o.delayMs / 1000).toFixed(1)} s`);
  pace.addEventListener("input", () => {
    o.delayMs = Number(pace.value);
    paceLabel.textContent = `${(o.delayMs / 1000).toFixed(1)} s`;
  });
  pace.addEventListener("change", () => render());

  const limitInput = h("input", {
    type: "number", min: 0, placeholder: "All", value: o.limit || "", class: "num", "aria-label": "Approve at most",
  });
  limitInput.addEventListener("change", () => { o.limit = Math.max(0, Math.floor(Number(limitInput.value) || 0)); render(); });

  const start = h("button", {
    class: "btn primary large",
    disabled: !stats || pending === 0 || busyElsewhere || !c.canApprove,
    onclick: () => startRun(c),
  }, !stats ? "Loading…"
    : pending === 0 ? "No pending requests"
    : `Approve ${fmt(limit || pending)} request${(limit || pending) === 1 ? "" : "s"}`);

  return h("section", { class: "pane" },
    header(c, h("button", { class: "btn ghost", onclick: () => enqueue(`stats:${c.id}`, () => refreshStats(c.id), true), disabled: s === "loading" }, "Refresh")),
    c.canApprove ? null : h("div", { class: "banner" },
      h("strong", {}, "Your admin role can't approve requests here. "),
      "Telegram only lets the owner, or admins allowed to add members, approve join requests."),
    h("div", { class: "stats" },
      statCard("Subscribers", stats ? fmt(stats.members) : loadingValue),
      statCard("Pending requests", stats ? fmt(stats.pending) : loadingValue),
    ),
    h("div", { class: "group" },
      h("div", { class: "row" },
        h("div", { class: "row-text" }, h("span", { class: "row-title" }, "Confirmation"),
          h("span", { class: "row-sub" }, o.verifyEach
            ? "Looks each person up right after approving them. Slower, and certain from the first approval."
            : "Checks everyone against the admin log at the end. About twice as fast, and just as certain.")),
        mode),
      h("div", { class: "row" },
        h("div", { class: "row-text" }, h("span", { class: "row-title" }, "Pause between calls"),
          h("span", { class: "row-sub" }, "Longer is gentler on Telegram's rate limits. The app slows down on its own if Telegram asks it to.")),
        h("div", { class: "range" }, pace, paceLabel)),
      h("div", { class: "row" },
        h("div", { class: "row-text" }, h("span", { class: "row-title" }, "Approve at most"),
          h("span", { class: "row-sub" }, "Leave empty to approve everyone. A small number is a good first test.")),
        limitInput),
    ),
    h("div", { class: "actions" },
      h("span", { class: "hint" },
        !c.canApprove ? ""
          : busyElsewhere ? "Another channel is being approved."
          : stats && pending > 0 ? (eta < 1 ? "Under a minute at this pace" : `About ${duration(eta)} at this pace`)
          : ""),
      start),
    pendingSection(c),
  );
}

function header(c: Channel, ...right: Child[]): HTMLElement {
  return h("header", { class: "pane-head", "data-tauri-drag-region": true },
    avatar(c.id, c.title, "large", channelPic(c)),
    h("div", { class: "pane-title" },
      h("h2", {}, c.title),
      h("span", { class: "channel-sub" }, c.isGroup ? "Group" : "Channel", c.username ? ` · @${c.username}` : " · Private"),
    ),
    h("div", { class: "head-actions" }, ...right),
  );
}

function pendingSection(c: Channel): HTMLElement {
  const p = state.pending.get(c.id);
  if (!p && c.canApprove) queueMicrotask(() => loadPending(c.id, false));
  const wrap = h("div", { class: "group pending" });
  const head = h("div", { class: "group-head" }, h("span", {}, "Who's waiting"));
  wrap.append(head);
  if (!c.canApprove) {
    wrap.append(h("p", { class: "muted pad" }, "Only admins who can add members can see and approve requests."));
    return wrap;
  }
  if (p && p.count > 0) head.append(h("span", { class: "muted" }, `${fmt(p.items.length)} of ${fmt(p.count)}`));
  const list = h("ul", { class: "people" });
  for (const it of p?.items ?? []) {
    list.append(h("li", {},
      avatar(it.userId, it.name, "tiny", it.thumb),
      h("span", { class: "person" }, it.name),
      h("span", { class: "muted" }, ago(it.date))));
  }
  if (p?.items.length) wrap.append(list);
  if (!p || (p.loading && !p.items.length)) {
    wrap.append(h("p", { class: "muted pad" }, h("span", { class: "spinner" }), " Loading…"));
  } else if (p.error) {
    wrap.append(h("p", { class: "form-error pad" }, p.error, " ",
      h("button", { class: "btn small", onclick: () => loadPending(c.id, p.items.length > 0) }, "Try again")));
  } else if (!p.items.length) {
    wrap.append(h("p", { class: "muted pad" }, "Nobody is waiting."));
  } else if (p.more) {
    wrap.append(h("div", { class: "more" },
      h("button", { class: "btn", disabled: p.loading, onclick: () => loadPending(c.id, true) },
        p.loading ? "Loading…" : "Show more")));
  }
  return wrap;
}

/** The first page, or the next one: Telegram serves them 100 at a time. */
async function loadPending(id: number, more: boolean) {
  const prev = state.pending.get(id);
  if (prev?.loading) return;
  const p: PendingState = more && prev ? { ...prev, loading: true, error: "" }
    : { count: prev?.count ?? 0, items: prev?.items ?? [], more: false, loading: true, error: "" };
  state.pending.set(id, p);
  if (!more && !prev) {
    const saved = await api.cacheRead<{ count: number; items: Person[] }>(`pending_${id}`);
    if (saved && state.pending.get(id)?.loading) {
      Object.assign(p, { count: saved.count, items: saved.items });
      render();
    }
  }
  if (more) render();
  try {
    const page = await api.pendingPage(id, more);
    const seen = new Set(more ? p.items.map((i) => i.userId) : []);
    const items = more ? [...p.items, ...page.items.filter((i) => !seen.has(i.userId))] : page.items;
    state.pending.set(id, { count: page.telegramCount || p.count, items, more: page.more, loading: false, error: "" });
    if (!more) api.cacheWrite(`pending_${id}`, { count: page.telegramCount, items: page.items }).catch(() => {});
  } catch (e) {
    state.pending.set(id, { ...p, loading: false, error: String(e) });
  }
  if (!state.run || state.run.channelId !== id) render();
}

// ---------- running ----------

type FeedItem = { userId: number; name: string; thumb: string | null; outcome: Outcome; detail: string; explanation: string };
const feed: FeedItem[] = [];

async function startRun(c: Channel) {
  const s = state.stats.get(c.id);
  const pending = s && typeof s === "object" ? s.pending : 0;
  const n = Math.min(state.opts.limit || pending, pending);
  const ok = await ask(
    `Approve ${fmt(n)} join request${n === 1 ? "" : "s"}?`,
    `Everyone approved joins “${c.title}” straight away. This can't be undone from here.\n\nYou can stop at any time; the report still covers everyone handled so far.`,
    "Approve",
  );
  if (!ok) return;
  feed.length = 0;
  state.report = null;
  state.flood = null;
  state.run = {
    channelId: c.id, title: c.title, startCount: 0, total: n, approved: 0, parked: 0, attempted: 0,
    subscribers: null, perMinute: 0, delayMs: state.opts.delayMs, status: "Starting…", stopping: false, auditPath: "",
    limit: state.opts.limit,
  };
  render();
  try {
    await api.startApproval(c.id, state.opts);
  } catch (e) {
    state.run = null;
    render();
    ask("Couldn't start", String(e), "OK", null);
  }
}

function runPane(): HTMLElement {
  const r = state.run!;
  const c = state.channels.find((x) => x.id === r.channelId)!;
  const done = r.approved + r.parked;
  const pct = r.total ? Math.min(100, (r.attempted / r.total) * 100) : 0;
  const subsDelta = r.subscribers !== null ? r.subscribers - r.startCount : null;
  const remainingMin = r.perMinute > 0 ? (r.total - r.attempted) / r.perMinute : NaN;

  const stop = h("button", {
    class: "btn", disabled: r.stopping,
    onclick: async () => { r.stopping = true; r.status = "Stopping after this person, then writing the report…"; render(); await api.stopApproval(); },
  }, r.stopping ? "Stopping…" : "Stop");

  const floodBanner = state.flood && state.flood.until > Date.now()
    ? h("div", { class: "banner" },
        h("strong", {}, "Telegram asked for a pause. "),
        `Waiting ${Math.ceil((state.flood.until - Date.now()) / 1000)} s, then continuing at ${(state.flood.delayMs / 1000).toFixed(1)} s between calls.`)
    : null;

  return h("section", { class: "pane" },
    header(c, stop),
    h("div", { class: "progress-block" },
      h("div", { class: "progress-text" },
        h("span", { class: "status" }, h("span", { class: "spinner" }), r.status),
        h("span", { class: "muted" }, isFinite(remainingMin) ? `about ${duration(remainingMin)} left` : "")),
      h("div", { class: "bar", role: "progressbar", "aria-valuenow": Math.round(pct), "aria-valuemin": 0, "aria-valuemax": 100 },
        h("div", { class: "fill", style: `width:${pct}%` })),
    ),
    floodBanner,
    h("div", { class: "stats four" },
      h("div", { class: "stat" }, h("span", { class: "stat-label" }, "Approved"), h("span", { class: "stat-value good" }, fmt(r.approved)),
        h("span", { class: "stat-sub" }, `of ${fmt(r.total)}`)),
      h("div", { class: "stat" }, h("span", { class: "stat-label" }, "Couldn't approve"), h("span", { class: "stat-value" + (r.parked ? " warn" : "") }, fmt(r.parked)),
        h("span", { class: "stat-sub" }, "retried at the end")),
      h("div", { class: "stat" }, h("span", { class: "stat-label" }, "Subscribers"),
        h("span", { class: "stat-value" }, r.subscribers !== null ? fmt(r.subscribers) : r.startCount ? fmt(r.startCount) : "…"),
        h("span", { class: "stat-sub" + (subsDelta !== null && subsDelta >= 0 ? " good" : "") },
          subsDelta !== null ? `${signed(subsDelta)} since start` : "re-read every 50")),
      h("div", { class: "stat" }, h("span", { class: "stat-label" }, "Pace"), h("span", { class: "stat-value" }, r.perMinute ? `${Math.round(r.perMinute)}` : "…"),
        h("span", { class: "stat-sub" }, `per minute · ${(r.delayMs / 1000).toFixed(1)} s gap`)),
    ),
    h("div", { class: "group feed-group" },
      h("div", { class: "group-head" }, h("span", {}, "Activity"), h("span", { class: "muted" }, `${fmt(done)} handled`)),
      feedList(),
    ),
  );
}

const outcomeLabel: Record<Outcome, [string, string]> = {
  approved: ["Approved", "good"],
  approved_verified: ["Approved · confirmed", "good"],
  approved_unverified: ["Approved · not a member now", "warn"],
  already_member: ["Already a member", "muted"],
  request_gone: ["Request withdrawn", "muted"],
  parked: ["Couldn't approve", "warn"],
  fatal: ["Stopped", "bad"],
};

function feedRow(f: FeedItem): HTMLElement {
  const [label, tone] = outcomeLabel[f.outcome];
  return h("li", { title: f.detail || undefined },
    h("span", { class: `mark ${tone}` }, tone === "good" ? icon.check : tone === "warn" || tone === "bad" ? icon.cross : icon.dot),
    avatar(f.userId, f.name, "tiny", f.thumb),
    h("span", { class: "person" }, f.name),
    h("span", { class: `muted outcome` }, f.explanation ? `${label} · ${f.explanation}` : label),
  );
}

function feedList(): HTMLElement {
  const ul = h("ul", { class: "people feed", id: "feed" });
  for (const f of feed.slice(0, 200)) ul.append(feedRow(f));
  if (!feed.length) ul.append(h("li", { class: "muted" }, "Nothing yet."));
  return ul;
}

// Live updates patch the running pane instead of rebuilding the whole window.
let renderQueued = false;
function renderSoon() {
  if (renderQueued) return;
  renderQueued = true;
  requestAnimationFrame(() => { renderQueued = false; render(); });
}

function onJob(e: JobEvent) {
  const r = state.run;
  switch (e.kind) {
    case "started":
      if (r) Object.assign(r, { startCount: e.startCount, auditPath: e.auditPath, total: r.limit ? Math.min(r.limit, e.pending) : e.pending });
      break;
    case "status":
      if (r && !r.stopping) r.status = e.text;
      break;
    case "item":
      feed.unshift({ userId: e.userId, name: e.name, thumb: e.thumb, outcome: e.outcome, detail: e.detail, explanation: e.explanation });
      if (feed.length > 400) feed.length = 400;
      break;
    case "progress":
      if (r) Object.assign(r, {
        approved: e.approved, parked: e.parked, attempted: e.attempted, perMinute: e.perMinute, delayMs: e.delayMs,
        subscribers: e.subscribers ?? r.subscribers, startCount: e.startCount,
        // Done so far plus still queued; new requests mid-run can grow it.
        total: r.limit ? Math.min(r.limit, e.attempted + e.remaining) : e.attempted + e.remaining,
      });
      break;
    case "done":
      state.report = { channelId: r?.channelId ?? state.selected!, report: e.report };
      state.run = null;
      state.flood = null;
      if (state.report.channelId) refreshStats(state.report.channelId);
      notify(e.report);
      break;
    case "failed":
      state.run = null;
      ask("The run stopped", `${e.error}\n\nEveryone handled before this is in the audit log.`, "OK", null);
      break;
  }
  renderSoon();
}

function onFlood(e: FloodEvent) {
  state.flood = { until: Date.now() + e.seconds * 1000, seconds: e.seconds, delayMs: e.delayMs };
  if (state.run) state.run.delayMs = e.delayMs;
  renderSoon();
}

function notify(r: Report) {
  if (document.hasFocus()) return;
  document.title = `Done · ${fmt(r.verified)} approved`;
}

// ---------- report ----------

function reportPane(r: Report): HTMLElement {
  const c = state.channels.find((x) => x.id === state.report!.channelId)!;
  const delta = r.endCount - r.startCount;
  const gap = r.endCount - r.expectedCount;
  const allConfirmed = r.unverified.length === 0 && r.missingFromLog.length === 0;

  const verdict = allConfirmed
    ? h("div", { class: "verdict good" }, h("span", { class: "big-mark" }, icon.check),
        h("div", {}, h("strong", {}, `${fmt(r.verified)} approved, every one confirmed`),
          h("p", {}, `Each approval is in the channel's admin log as a join by request.${r.stoppedEarly ? " Stopped early on request." : ""}`)))
    : h("div", { class: "verdict warn" }, h("span", { class: "big-mark" }, "!"),
        h("div", {}, h("strong", {}, `${fmt(r.verified)} confirmed, ${fmt(r.unverified.length)} not confirmed`),
          h("p", {}, "Some approvals don't show as members or in the admin log. They're listed below.")));

  const row = (label: string, value: Child, tone = "") =>
    h("div", { class: "kv" }, h("span", {}, label), h("span", { class: tone }, value));

  return h("section", { class: "pane" },
    header(c,
      h("button", { class: "btn ghost", onclick: () => openPath(r.auditPath).catch(() => api.auditFolder().then(openPath)) }, "Open audit log"),
      h("button", { class: "btn primary", onclick: () => { state.report = null; render(); } }, "Done")),
    verdict,
    h("div", { class: "stats" },
      h("div", { class: "stat" }, h("span", { class: "stat-label" }, "Subscribers"),
        h("span", { class: "stat-value" }, fmt(r.endCount)),
        h("span", { class: "stat-sub good" }, `${signed(delta)} (was ${fmt(r.startCount)})`)),
      h("div", { class: "stat" }, h("span", { class: "stat-label" }, "Still pending"),
        h("span", { class: "stat-value" }, fmt(r.pendingNow)),
        h("span", { class: "stat-sub" }, `took ${duration(r.minutes)}`)),
    ),
    h("div", { class: "group" },
      h("div", { class: "group-head" }, h("span", {}, "How the count adds up")),
      row("Started with", fmt(r.startCount)),
      row("Joined by request (admin log)", signed(r.joinedByRequest), "good"),
      r.approvedElsewhere ? row("  of which approved by someone else", fmt(r.approvedElsewhere), "muted") : null,
      r.otherJoins ? row("Joined another way", signed(r.otherJoins)) : null,
      r.leaves ? row("Left during the run", signed(-r.leaves)) : null,
      r.removed ? row("Removed by admins", signed(-r.removed)) : null,
      row("Expected", fmt(r.expectedCount)),
      row("Telegram's count", fmt(r.endCount)),
      h("div", { class: "kv total" }, h("span", {}, gap === 0 ? "Matches exactly" : "Difference"),
        h("span", { class: gap === 0 ? "good" : "warn" }, gap === 0 ? icon.check : `${signed(gap)} (Telegram's count can trail the log briefly)`)),
    ),
    h("div", { class: "group" },
      h("div", { class: "group-head" }, h("span", {}, "Everyone handled")),
      row("Approved and confirmed", fmt(r.verified), "good"),
      r.already ? row("Were already members", fmt(r.already)) : null,
      r.gone ? row("Withdrew their request first", fmt(r.gone)) : null,
      row("Couldn't be approved", fmt(r.parked.length), r.parked.length ? "warn" : ""),
      row("Rate-limit pauses", r.floodWaits ? `${r.floodWaits} (${fmt(r.floodSeconds)} s waited)` : "None"),
    ),
    r.parked.length ? h("div", { class: "group" },
      h("div", { class: "group-head" }, h("span", {}, "Still pending, and why"), h("span", { class: "muted" }, "Only they can fix these")),
      h("ul", { class: "people" }, ...r.parked.map((p) =>
        h("li", { title: p.reason }, h("span", { class: "mark warn" }, icon.cross), avatar(p.userId, p.name, "tiny", p.thumb),
          h("span", { class: "person" }, p.name), h("span", { class: "muted outcome" }, p.explanation)))),
    ) : null,
    r.unverified.length ? h("div", { class: "group" },
      h("div", { class: "group-head" }, h("span", {}, "Approved but not confirmed")),
      h("ul", { class: "people" }, ...r.unverified.map((n) => h("li", {}, h("span", { class: "mark warn" }, "?"), h("span", { class: "person" }, n))))) : null,
  );
}

// ---------- updates ----------
// Signed releases from GitHub: the app only installs what the release key
// signed, and never in the middle of an approval run.

let available: Update | null = null;

async function checkForUpdate() {
  if (state.update?.status === "downloading") return;
  try {
    const u = await check();
    if (!u) return;
    available = u;
    state.update = { version: u.version, status: "available", done: 0, total: 0, error: "" };
    renderSoon();
  } catch {
    // Offline, or no release yet: try again next time.
  }
}

async function installUpdate() {
  const u = available;
  const up = state.update;
  if (!u || !up) return;
  if (state.run) {
    await ask("Finish the run first", "The update restarts the app. Install it once the approval run is done.", "OK", null);
    return;
  }
  up.status = "downloading";
  up.done = 0;
  render();
  try {
    await u.downloadAndInstall((ev) => {
      if (ev.event === "Started") up.total = ev.data.contentLength ?? 0;
      else if (ev.event === "Progress") up.done += ev.data.chunkLength;
      renderSoon();
    });
    await relaunch();
  } catch (e) {
    up.status = "error";
    up.error = String(e);
    render();
  }
}

function updateCard(): HTMLElement | null {
  const u = state.update;
  if (!u) return null;
  if (u.status === "downloading") {
    const pct = u.total ? Math.round((u.done / u.total) * 100) : 0;
    return h("div", { class: "update" },
      h("span", {}, `Downloading ${u.version}… ${u.total ? pct + "%" : ""}`),
      h("div", { class: "bar" }, h("div", { class: "fill", style: `width:${pct}%` })));
  }
  return h("div", { class: "update" },
    h("span", {}, u.status === "error" ? `Update failed: ${u.error}` : `Version ${u.version} is available`),
    h("button", { class: "btn small primary", onclick: installUpdate }, u.status === "error" ? "Try again" : "Install & restart"));
}

// ---------- shell ----------

function render() {
  const active = document.activeElement as HTMLElement | null;
  const keepFocus = active?.tagName === "INPUT" && root.contains(active) ? active.getAttribute("aria-label") : null;
  const feedScroll = document.getElementById("feed")?.parentElement?.scrollTop;
  const listScroll = document.querySelector(".channel-list")?.scrollTop;

  if (!state.auth) {
    root.replaceChildren(h("main", { class: "splash", "data-tauri-drag-region": true }, h("span", { class: "spinner" }), "Connecting to Telegram…"));
    return;
  }
  if (state.auth.state !== "signedIn") {
    if (!root.querySelector(".login")) root.replaceChildren(loginView());
    return;
  }
  const c = state.channels.find((x) => x.id === state.selected);
  const main = h("main", { class: "content" },
    c ? channelPane(c) : h("section", { class: "pane empty", "data-tauri-drag-region": true },
      h("p", { class: "muted" }, state.channelsLoading ? "" : "Pick a channel on the left.")));
  root.replaceChildren(h("div", { class: "shell" }, sidebar(), main));
  updateSidebarBadges();
  applySearch();

  const list = document.querySelector(".channel-list");
  if (list && listScroll) list.scrollTop = listScroll;
  const feedEl = document.getElementById("feed")?.parentElement;
  if (feedEl && feedScroll) feedEl.scrollTop = feedScroll;
  if (keepFocus) (root.querySelector(`input[aria-label="${keepFocus}"]`) as HTMLElement | null)?.focus();
}

// Pause notices count down on their own.
setInterval(() => { if (state.flood && state.flood.until > Date.now() - 1000 && (state.run || state.channelsLoading)) renderSoon(); }, 1000);

window.addEventListener("focus", () => { document.title = "Join Approver"; });

// ⌘F / Ctrl+F jumps to the channel search.
window.addEventListener("keydown", (e) => {
  if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "f") {
    const s = document.querySelector<HTMLInputElement>("input.search");
    if (s) { e.preventDefault(); s.focus(); s.select(); }
  }
});

async function boot() {
  state.platform = await api.platform();
  document.documentElement.dataset.platform = state.platform;
  await api.onJob(onJob);
  await api.onFlood(onFlood);
  await api.onChannelFound(onChannelFound);
  const snap = await api.cacheRead<Snapshot>("snapshot");
  if (snap?.me) restore(snap);
  render();
  let auth: AuthState;
  try {
    auth = await api.authState();
  } catch (e) {
    if (snap?.me) {
      // Offline: the saved view stays, marked as such.
      state.channelsError = `Offline. Showing what was saved. (${e})`;
      render();
      return;
    }
    root.replaceChildren(h("main", { class: "splash" },
      h("p", { class: "form-error" }, String(e)),
      h("button", { class: "btn", onclick: () => location.reload() }, "Try again")));
    return;
  }
  checkForUpdate();
  setInterval(checkForUpdate, 6 * 60 * 60 * 1000);
  if (auth.state === "signedIn") await signedIn(auth);
  else {
    Object.assign(state, { auth, me: null, mePhoto: null, channels: [], selected: null });
    state.stats.clear();
    state.photos.clear();
    render();
  }
}

boot();
