// README screenshots of the real Join Approver UI, with the same mocked Tauri
// bridge and invented data as scripts/screens.ts: nobody real appears.
//
//   bun scripts/docs-shots.ts  # needs ../dist (bun run build in the repo root)
//
// Writes ../docs/*.png.
import { chromium } from "playwright-core";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const dist = join(import.meta.dir, "../../dist");
const out = join(import.meta.dir, "../../docs");
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

async function open(platform: string, scheme: "light" | "dark", signedIn = true) {
  const page = await browser.newPage({ viewport: { width: 1100, height: 700 }, colorScheme: scheme, deviceScaleFactor: 2 });
  page.on("pageerror", (e) => console.log("PAGEERROR", e.message));
  await page.goto(`http://localhost:${server.port}/?p=${platform}&s=${signedIn ? 1 : 0}`);
  // macOS draws its vibrant sidebar over the desktop; fake a desktop tint behind it.
  const bg = platform === "macos" ? (scheme === "dark" ? "#24252b" : "#e9e9ef") : "transparent";
  await page.addStyleTag({ content: `html{background:${bg}} .update{display:none}` });
  if (!signedIn) {
    await page.waitForSelector(".login-card");
    await page.waitForTimeout(300);
    return page;
  }
  await page.waitForSelector(".people li");
  await page.click(".channel[data-id='101']");
  await page.waitForFunction(() => document.querySelector(".pane-title h2")?.textContent?.startsWith("Midnight"));
  await page.waitForSelector(".people li");
  await page.waitForTimeout(600);
  return page;
}

const shot = (page: any, name: string) => page.screenshot({ path: join(out, `${name}.png`) });

let page = await open("macos", "dark", false);
await shot(page, "sign-in");
await page.close();

page = await open("macos", "light");
await shot(page, "channel-light");
await page.close();

page = await open("windows", "light");
await shot(page, "windows");
await page.close();

page = await open("macos", "dark");
await shot(page, "channel");
await page.fill("input.search", "dea");
await page.waitForTimeout(150);
await shot(page, "search");
await page.fill("input.search", "");
await page.click(".actions .btn.primary");
await page.waitForSelector(".sheet");
await page.waitForTimeout(300);
await shot(page, "confirm");
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
  e("flood", { seconds: 23, delayMs: 2250 });
});
await page.waitForTimeout(400);
await shot(page, "approving");
await page.evaluate(() => {
  (window as any).__emit("job", { kind: "done", report: {
    title: "Midnight Deals", minutes: 61.8, stoppedEarly: false, verified: 1172, unverified: [], already: 1, gone: 2,
    parked: [
      { userId: 9004, name: "Liam O'Connor", reason: "USER_CHANNELS_TOO_MUCH", explanation: "Already in the maximum number of channels and groups" },
      { userId: 9011, name: "Deleted account", reason: "INPUT_USER_DEACTIVATED", explanation: "Deleted account" },
    ],
    joinedByRequest: 1172, oursInLog: 1172, approvedElsewhere: 0, otherJoins: 3, leaves: 8, removed: 0,
    startCount: 48912, endCount: 50079, expectedCount: 50079, pendingNow: 11, missingFromLog: [],
    floodWaits: 1, floodSeconds: 23, finalDelayMs: 2250, auditPath: "" } });
});
await page.waitForTimeout(500);
await shot(page, "report");

await browser.close();
server.stop();
console.log(`wrote ${out}/*.png`);
