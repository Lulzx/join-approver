// One command to ship an update:
//
//   bun run release                 # 0.1.0 -> 0.1.1
//   bun run release minor           # 0.1.0 -> 0.2.0
//   bun run release 1.0.0 --notes "Faster approvals"
//   bun run release --dry-run       # build and stage, publish nothing
//
// Bumps the version everywhere, builds macOS (universal) and Windows, signs
// the update bundles with the release key, writes latest.json, and publishes
// a GitHub release that installed copies pick up on their own.
//
// The private key stays in ~/.tauri and its password in the macOS Keychain;
// only the installers, their signatures and latest.json are uploaded.

import { $ } from "bun";
import { existsSync, mkdirSync, readFileSync, rmSync, copyFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const REPO = "Lulzx/join-approver";
const KEY = join(homedir(), ".tauri", "join-approver.key");
const KEYCHAIN_ITEM = "join-approver-updater";
const root = join(import.meta.dir, "..");
const tauri = join(root, "src-tauri");

const args = process.argv.slice(2);
const dryRun = args.includes("--dry-run");
const notesAt = args.indexOf("--notes");
const notes = notesAt >= 0 ? args[notesAt + 1] : "Fixes and improvements.";
const bump = args.find((a, i) => !a.startsWith("--") && (notesAt < 0 || i !== notesAt + 1)) ?? "patch";

const die = (msg: string): never => {
  console.error(`\n✕ ${msg}`);
  process.exit(1);
};

// ---- version ----
const confPath = join(tauri, "tauri.conf.json");
const conf = JSON.parse(readFileSync(confPath, "utf8"));
const [maj, min, pat] = conf.version.split(".").map(Number);
const version =
  bump === "major" ? `${maj + 1}.0.0`
  : bump === "minor" ? `${maj}.${min + 1}.0`
  : bump === "patch" ? `${maj}.${min}.${pat + 1}`
  : /^\d+\.\d+\.\d+$/.test(bump) ? bump
  : die(`"${bump}" isn't patch, minor, major or a version like 1.2.3`);
const tag = `v${version}`;

// ---- preflight: fail before spending ten minutes building ----
if (!existsSync(KEY)) die(`No release key at ${KEY}`);
if (!existsSync(join(root, ".env"))) die("No .env with TG_API_ID / TG_API_HASH");
const password = (await $`security find-generic-password -s ${KEYCHAIN_ITEM} -w`.quiet().nothrow()).stdout.toString().trim();
if (!password) die(`No "${KEYCHAIN_ITEM}" password in the Keychain`);
if (!dryRun) {
  if ((await $`gh auth status`.quiet().nothrow()).exitCode !== 0) die("gh isn't signed in (gh auth login)");
  if ((await $`gh release view ${tag} --repo ${REPO}`.quiet().nothrow()).exitCode === 0) die(`${tag} is already released`);
}
if (!existsSync(join(root, ".git"))) die("Not a git checkout");
if (!dryRun && (await $`git -C ${root} status --porcelain`.text()).trim()) {
  die("Uncommitted changes: commit them first, so the release matches a commit");
}

console.log(`Releasing ${conf.version} → ${version}${dryRun ? " (dry run)" : ""}`);

// ---- bump ----
conf.version = version;
writeFileSync(confPath, JSON.stringify(conf, null, 2) + "\n");
const pkgPath = join(root, "package.json");
const pkg = JSON.parse(readFileSync(pkgPath, "utf8"));
pkg.version = version;
writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n");
const cargoPath = join(tauri, "Cargo.toml");
writeFileSync(cargoPath, readFileSync(cargoPath, "utf8").replace(/^version = ".*"$/m, `version = "${version}"`));

// ---- build ----
const env = {
  ...process.env,
  PATH: [join(homedir(), ".cargo/bin"), "/opt/homebrew/opt/llvm/bin", "/opt/homebrew/opt/lld/bin", process.env.PATH].join(":"),
  TAURI_SIGNING_PRIVATE_KEY: readFileSync(KEY, "utf8"),
  TAURI_SIGNING_PRIVATE_KEY_PASSWORD: password,
};
console.log("Building macOS (universal)…");
await $`bun run tauri build --target universal-apple-darwin`.cwd(root).env(env).quiet();
console.log("Building Windows (x64)…");
await $`bun run tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc`.cwd(root).env(env).quiet();

// ---- stage, with names GitHub won't mangle ----
const out = join(root, "release", tag);
rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });
const mac = join(tauri, "target/universal-apple-darwin/release/bundle");
const win = join(tauri, "target/x86_64-pc-windows-msvc/release/bundle/nsis");
const files = {
  dmg: [join(mac, `dmg/Join Approver_${version}_universal.dmg`), `JoinApprover_${version}_mac.dmg`],
  macUpdate: [join(mac, "macos/Join Approver.app.tar.gz"), `JoinApprover_${version}_mac.app.tar.gz`],
  winSetup: [join(win, `Join Approver_${version}_x64-setup.exe`), `JoinApprover_${version}_windows-setup.exe`],
} as const;
for (const [src, name] of Object.values(files)) {
  if (!existsSync(src)) die(`Build output missing: ${src}`);
  copyFileSync(src, join(out, name));
}
const sig = (src: string) => {
  const p = `${src}.sig`;
  if (!existsSync(p)) die(`Unsigned: ${p} missing`);
  return readFileSync(p, "utf8").trim();
};
const url = (name: string) => `https://github.com/${REPO}/releases/download/${tag}/${name}`;
const macEntry = { signature: sig(files.macUpdate[0]), url: url(files.macUpdate[1]) };
const latest = {
  version,
  notes,
  pub_date: new Date().toISOString(),
  platforms: {
    "darwin-aarch64": macEntry,
    "darwin-x86_64": macEntry,
    "windows-x86_64": { signature: sig(files.winSetup[0]), url: url(files.winSetup[1]) },
  },
};
writeFileSync(join(out, "latest.json"), JSON.stringify(latest, null, 2) + "\n");
const upload = [...Object.values(files).map(([, name]) => join(out, name)), join(out, "latest.json")];
console.log(`Staged in ${out}:`);
for (const f of upload) console.log(`  ${f.split("/").pop()}`);

if (dryRun) {
  console.log("\nDry run: nothing published. The version bump is left in place; revert it with git if needed.");
  process.exit(0);
}

// ---- publish ----
// The bump is committed, tagged and pushed first, so the release is built
// from exactly the commit its tag points at.
if ((await $`gh repo view ${REPO}`.quiet().nothrow()).exitCode !== 0) {
  console.log(`Creating ${REPO}…`);
  await $`gh repo create ${REPO} --public --source ${root} --remote origin --description ${"Approve Telegram join requests, and prove each one landed"}`.quiet();
}
await $`git -C ${root} commit -qam ${`Release ${tag}`}`;
await $`git -C ${root} tag ${tag}`;
await $`git -C ${root} push -q origin HEAD ${tag}`;
const body = `${notes}\n\n**Download:** macOS (Apple Silicon and Intel): \`${files.dmg[1]}\` · Windows: \`${files.winSetup[1]}\`\n\nInstalled copies update themselves.`;
await $`gh release create ${tag} ${upload} --repo ${REPO} --verify-tag --title ${`Join Approver ${version}`} --notes ${body}`;
console.log(`\n✓ Published ${tag}: https://github.com/${REPO}/releases/tag/${tag}`);
