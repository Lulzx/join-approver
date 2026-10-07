// Typed wrappers around the Rust commands and events.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Me = { name: string; username: string | null; phone: string | null };

export type AuthState =
  | { state: "signedOut" }
  | { state: "signedIn"; me: Me }
  | { state: "passwordNeeded"; hint: string | null };

export type Channel = {
  id: number;
  title: string;
  username: string | null;
  isGroup: boolean;
  members: number | null;
  role: "owner" | "admin";
  canApprove: boolean;
  thumb: string | null;
  hasPhoto: boolean;
};

export type Stats = { members: number; pending: number };

export type Person = { userId: number; name: string; date: number; thumb: string | null };

export type PendingPage = { telegramCount: number; items: Person[]; more: boolean };

export type Options = { verifyEach: boolean; limit: number; delayMs: number };

export type Stuck = { userId: number; name: string; thumb: string | null; reason: string; explanation: string };

export type Report = {
  title: string;
  minutes: number;
  stoppedEarly: boolean;
  verified: number;
  unverified: string[];
  already: number;
  gone: number;
  parked: Stuck[];
  joinedByRequest: number;
  oursInLog: number;
  approvedElsewhere: number;
  otherJoins: number;
  leaves: number;
  removed: number;
  startCount: number;
  endCount: number;
  expectedCount: number;
  pendingNow: number;
  missingFromLog: string[];
  floodWaits: number;
  floodSeconds: number;
  finalDelayMs: number;
  auditPath: string;
};

export type Outcome =
  | "approved"
  | "approved_verified"
  | "approved_unverified"
  | "already_member"
  | "request_gone"
  | "parked"
  | "fatal";

export type JobEvent =
  | { kind: "started"; title: string; startCount: number; pending: number; auditPath: string }
  | { kind: "item"; userId: number; name: string; thumb: string | null; outcome: Outcome; detail: string; explanation: string }
  | {
      kind: "progress";
      approved: number;
      parked: number;
      attempted: number;
      remaining: number;
      subscribers: number | null;
      startCount: number;
      perMinute: number;
      delayMs: number;
    }
  | { kind: "status"; text: string }
  | { kind: "done"; report: Report }
  | { kind: "failed"; error: string };

export type FloodEvent = { seconds: number; delayMs: number };

export const api = {
  platform: () => invoke<string>("platform"),
  authState: () => invoke<AuthState>("auth_state"),
  sendCode: (phone: string) => invoke<void>("send_code", { phone }),
  signIn: (code: string) => invoke<AuthState>("sign_in", { code }),
  checkPassword: (password: string) => invoke<AuthState>("check_password", { password }),
  signOut: () => invoke<void>("sign_out"),
  listChannels: () => invoke<Channel[]>("list_channels"),
  channelStats: (id: number) => invoke<Stats>("channel_stats", { id }),
  pendingPage: (id: number, more: boolean) => invoke<PendingPage>("pending_page", { id, more }),
  channelPhoto: (id: number) => invoke<string | null>("channel_photo", { id }),
  mePhoto: () => invoke<string | null>("me_photo"),
  cacheRead: <T>(key: string): Promise<T | null> =>
    invoke<string | null>("cache_read", { key }).then((s) => {
      try { return s ? (JSON.parse(s) as T) : null; } catch { return null; }
    }),
  cacheWrite: (key: string, value: unknown) => invoke<void>("cache_write", { key, json: JSON.stringify(value) }),
  startApproval: (id: number, options: Options) => invoke<void>("start_approval", { id, options }),
  stopApproval: () => invoke<void>("stop_approval"),
  auditFolder: () => invoke<string>("audit_folder"),
  onJob: (fn: (e: JobEvent) => void): Promise<UnlistenFn> => listen<JobEvent>("job", (e) => fn(e.payload)),
  onChannelFound: (fn: (c: Channel) => void): Promise<UnlistenFn> =>
    listen<Channel>("channel-found", (e) => fn(e.payload)),
  onFlood: (fn: (e: FloodEvent) => void): Promise<UnlistenFn> =>
    listen<FloodEvent>("flood", (e) => fn(e.payload)),
};
