// Screenshots of the real Join Approver UI for the demo, with a mocked Tauri
// bridge and invented data: no real channel or person appears in the video.
//
//   bun scripts/screens.ts     # needs ../dist (bun run build in the repo root)
//
// Writes media/app-*.png at 2x.
import { chromium } from "playwright-core";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const dist = join(import.meta.dir, "../../dist");
const out = join(import.meta.dir, "../media");
const html = readFileSync(join(dist, "index.html"), "utf8");

// Abstract gradient "photos": no faces, nobody real.
const avatar = (h1: number, h2: number, shape: number) => {
  const shapes = [
    `<circle cx="70" cy="35" r="26" fill="#fff" fill-opacity=".28"/>`,
    `<path d="M0 70 Q50 30 100 70 V100 H0Z" fill="#fff" fill-opacity=".22"/>`,
    `<rect x="22" y="22" width="56" height="56" rx="14" transform="rotate(18 50 50)" fill="#fff" fill-opacity=".2"/>`,
    `<circle cx="50" cy="40" r="17" fill="#fff" fill-opacity=".32"/><ellipse cx="50" cy="92" rx="34" ry="26" fill="#fff" fill-opacity=".32"/>`,
  ];
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="hsl(${h1} 80% 62%)"/><stop offset="1" stop-color="hsl(${h2} 75% 42%)"/></linearGradient></defs><rect width="100" height="100" fill="url(#g)"/>${shapes[shape % shapes.length]}</svg>`;
  return "data:image/svg+xml;base64," + btoa(svg);
};

const mock = `
<script>
(() => {
  const cbs = new Map(); let next = 1; const listeners = {};
  const av = ${avatar.toString()};
  const channels = [
    { id: 101, title: "Midnight Deals ⚡", username: null, isGroup: false, members: 48912, role: "owner", canApprove: true, thumb: null, hasPhoto: true },
    { id: 102, title: "Gadget Drops", username: "gadgetdrops", isGroup: false, members: 21407, role: "admin", canApprove: true, thumb: null, hasPhoto: true },
    { id: 103, title: "Travel Hacks Club", username: null, isGroup: true, members: 8350, role: "admin", canApprove: true, thumb: null, hasPhoto: true },
    { id: 104, title: "Book Swap Circle", username: "bookswap", isGroup: true, members: 2196, role: "owner", canApprove: true, thumb: null, hasPhoto: true },
    { id: 105, title: "Design Daily", username: "designdaily", isGroup: false, members: 15873, role: "admin", canApprove: false, thumb: null, hasPhoto: true },
  ];
  const photos = { 101: av(265, 330, 2), 102: av(190, 230, 0), 103: av(150, 200, 1), 104: av(25, 350, 0), 105: av(45, 15, 2) };
  const stats = { 101: { members: 48912, pending: 1186 }, 102: { members: 21407, pending: 243 }, 103: { members: 8350, pending: 57 }, 104: { members: 2196, pending: 12 }, 105: { members: 15873, pending: 31 } };
  const names = ["Aarav Mehta","Sofia Rossi","Kenji Watanabe","Maya Patel","Liam O'Connor","Zara Ahmed","Noah Kim","Isabella Cruz","Arjun Nair","Emma Schulz","Omar Haddad","Priya Iyer","Lucas Martin","Chloé Dubois","Mateo Silva","Ananya Rao"];
  window.__names = names; window.__av = av;
  const now = Date.now() / 1000;
  window.__TAURI_INTERNALS__ = {
    transformCallback(cb) { const id = next++; cbs.set(id, cb); return id; },
    unregisterCallback(id) { cbs.delete(id); },
    async invoke(cmd, args) {
      switch (cmd) {
        case "platform": return new URLSearchParams(location.search).get("p") ?? "macos";
        case "auth_state": return new URLSearchParams(location.search).get("s") === "0" ? { state: "signedOut" } : { state: "signedIn", me: { name: "Riya Kapoor", username: "riya", phone: null } };
        case "list_channels": return channels;
        case "channel_stats": return stats[args.id];
        case "channel_photo": return photos[args.id] ?? null;
        case "me_photo": return av(300, 260, 3);
        case "pending_page": return { telegramCount: stats[args.id].pending, more: true, items: Array.from({ length: 100 }, (_, i) => ({ userId: 9000 + i, name: names[i % names.length], date: now - 60 * (4 + i * i * 9), thumb: i % 4 === 2 ? null : av((i * 47) % 360, (i * 47 + 50) % 360, i) })) };
        case "cache_read": return null;
        case "plugin:event|listen": (listeners[args.event] ||= []).push(args.handler); return next++;
        default: return null;
      }
    },
  };
  window.__emit = (event, payload) => (listeners[event] || []).forEach((h) => cbs.get(h)?.({ event, id: 0, payload }));
})();
</script>`;

const server = Bun.serve({
  port: 0,
  fetch(req) {
    const url = new URL(req.url);
    if (url.pathname === "/") return new Response(html.replace("<head>", "<head>" + mock), { headers: { "content-type": "text/html" } });
    const f = Bun.file(join(dist, url.pathname));
    return f.size ? new Response(f) : new Response("", { status: 404 });
  },
});

const browser = await chromium.launch({ executablePath: "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" });
const page = await browser.newPage({ viewport: { width: 1100, height: 700 }, colorScheme: "dark", deviceScaleFactor: 2 });
page.on("pageerror", (e) => console.log("PAGEERROR", e.message));
await page.goto(`http://localhost:${server.port}/`);
// The vibrant sidebar, as macOS draws it over a dark desktop.
await page.addStyleTag({ content: `html{background:#24252b} .update{display:none}` });
await page.waitForSelector(".people li");
await page.click(".channel[data-id='101']");
await page.waitForFunction(() => document.querySelector(".pane-title h2")?.textContent?.startsWith("Midnight"));
await page.waitForSelector(".people li");
await page.waitForTimeout(600);
await page.screenshot({ path: join(out, "app-channel.png") });

await page.fill("input.search", "dea");
await page.waitForTimeout(150);
await page.screenshot({ path: join(out, "app-search.png") });
await page.fill("input.search", "");

await page.click(".actions .btn.primary");
await page.waitForSelector(".sheet");
await page.waitForTimeout(300);
await page.screenshot({ path: join(out, "app-confirm.png") });
await page.click(".sheet .btn.primary");
await page.evaluate(() => {
  const e = (window as any).__emit;
  const names: string[] = (window as any).__names;
  const av = (window as any).__av;
  e("job", { kind: "started", title: "Midnight Deals", startCount: 48912, pending: 1186, auditPath: "" });
  for (let i = 0; i < 12; i++) {
    const parked = i === 4;
    e("job", { kind: "item", userId: 9000 + i, name: names[(i + 5) % names.length], thumb: i % 3 === 1 ? null : av((i * 61) % 360, (i * 61 + 40) % 360, i),
      outcome: parked ? "parked" : "approved_verified", detail: parked ? "USER_CHANNELS_TOO_MUCH" : "",
      explanation: parked ? "Already in the maximum number of channels" : "" });
  }
  e("job", { kind: "status", text: "Approving…" });
  e("job", { kind: "progress", approved: 612, parked: 9, attempted: 621, remaining: 565, subscribers: 49519, startCount: 48912, perMinute: 19.4, delayMs: 1500 });
});
await page.waitForTimeout(400);
await page.screenshot({ path: join(out, "app-running.png") });

await page.evaluate(() => {
  (window as any).__emit("job", { kind: "done", report: {
    title: "Midnight Deals", minutes: 61.8, stoppedEarly: false, verified: 1172, unverified: [], already: 1, gone: 2,
    parked: [], joinedByRequest: 1172, oursInLog: 1172, approvedElsewhere: 0, otherJoins: 3, leaves: 8, removed: 0,
    startCount: 48912, endCount: 50079, expectedCount: 50079, pendingNow: 11, missingFromLog: [],
    floodWaits: 0, floodSeconds: 0, finalDelayMs: 1500, auditPath: "" } });
});
await page.waitForTimeout(500);
await page.screenshot({ path: join(out, "app-report.png") });

await browser.close();
server.stop();
console.log("wrote media/app-*.png");
