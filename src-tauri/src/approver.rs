//! Approve a channel's pending join requests, and prove each one landed.
//!
//! The same guarantees as the original script:
//!
//! 1. per user: after approving, the user is looked up as a participant, so an
//!    approval only counts once Telegram confirms the membership (or, in fast
//!    mode, once the admin log shows the join);
//! 2. the admin log: every join-by-request since the run started is matched
//!    against the users approved, and every leave/kick is counted;
//! 3. the subscriber count: start and end are compared with what (1) and (2)
//!    predict, so churn during the run is explained rather than hidden.
//!
//! Nothing is dropped silently: the pending list is re-read until it holds no
//! request that can still be acted on; requests that fail are parked with
//! Telegram's reason, retried once more at the end, and listed in the report.
//! Every attempt is appended to a JSONL audit log.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use grammers_client::{tl, InvocationError};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::media;
use crate::tg::{error_name, Tg};

#[derive(Clone, Copy)]
pub struct ChannelRef {
    pub id: i64,
    pub access_hash: i64,
}

impl ChannelRef {
    pub fn input_channel(&self) -> tl::enums::InputChannel {
        tl::types::InputChannel { channel_id: self.id, access_hash: self.access_hash }.into()
    }
    pub fn input_peer(&self) -> tl::enums::InputPeer {
        tl::types::InputPeerChannel { channel_id: self.id, access_hash: self.access_hash }.into()
    }
}

/// Approval errors, by what they mean for the request.
const ALREADY: &[&str] = &["USER_ALREADY_PARTICIPANT"];
/// Withdrawn, expired, or handled elsewhere.
const GONE: &[&str] = &["HIDE_REQUESTER_MISSING"];
/// The whole run is pointless.
const FATAL: &[&str] = &[
    "CHAT_ADMIN_REQUIRED",
    "CHAT_WRITE_FORBIDDEN",
    "CHANNEL_PRIVATE",
    "CHANNEL_INVALID",
    "USERS_TOO_MUCH",
    "AUTH_KEY_UNREGISTERED",
    "SESSION_REVOKED",
];

/// Plain-language reasons for the refusals people will actually see.
pub fn explain(reason: &str) -> &'static str {
    match reason {
        "USER_CHANNELS_TOO_MUCH" => "Already in the maximum number of channels and groups",
        "INPUT_USER_DEACTIVATED" | "USER_DEACTIVATED" => "Deleted account",
        "USER_BANNED_IN_CHANNEL" | "USER_KICKED" => "Banned from this channel",
        "USER_PRIVACY_RESTRICTED" => "Their privacy settings block it",
        "USER_NOT_RETURNED" => "Telegram did not return this account",
        "CONNECTION_LOST" => "Connection kept dropping",
        _ => "Telegram refused it",
    }
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    /// Look every approved user up as a participant straight away. Off, the
    /// admin log confirms them in bulk at the end: half the calls, same proof.
    pub verify_each: bool,
    /// Approve at most this many (0 = all).
    pub limit: u32,
    pub delay_ms: u64,
}

#[derive(Serialize, Clone)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum JobEvent {
    #[serde(rename_all = "camelCase")]
    Started { title: String, start_count: i64, pending: usize, audit_path: String },
    #[serde(rename_all = "camelCase")]
    Item {
        user_id: i64,
        name: String,
        thumb: Option<String>,
        outcome: &'static str,
        detail: String,
        explanation: &'static str,
    },
    #[serde(rename_all = "camelCase")]
    Progress {
        approved: usize,
        parked: usize,
        attempted: usize,
        remaining: usize,
        subscribers: Option<i64>,
        start_count: i64,
        per_minute: f64,
        delay_ms: u64,
    },
    Status { text: String },
    Done { report: Report },
    Failed { error: String },
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Stuck {
    pub user_id: i64,
    pub name: String,
    pub thumb: Option<String>,
    pub reason: String,
    pub explanation: &'static str,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub title: String,
    pub minutes: f64,
    pub stopped_early: bool,
    pub verified: usize,
    pub unverified: Vec<String>,
    pub already: usize,
    pub gone: usize,
    pub parked: Vec<Stuck>,
    pub joined_by_request: usize,
    pub ours_in_log: usize,
    pub approved_elsewhere: usize,
    pub other_joins: usize,
    pub leaves: usize,
    pub removed: usize,
    pub start_count: i64,
    pub end_count: i64,
    pub expected_count: i64,
    pub pending_now: i64,
    pub missing_from_log: Vec<String>,
    pub flood_waits: u32,
    pub flood_seconds: u64,
    pub final_delay_ms: u64,
    pub audit_path: String,
}

#[derive(Clone)]
pub struct Requester {
    pub user_id: i64,
    pub access_hash: Option<i64>,
    pub name: String,
    pub date: i32,
    /// Telegram's inline preview of their photo, as a data URL: free, no
    /// download.
    pub thumb: Option<String>,
}

impl Requester {
    fn input_user(&self) -> Option<tl::enums::InputUser> {
        Some(tl::types::InputUser { user_id: self.user_id, access_hash: self.access_hash? }.into())
    }
    fn input_peer(&self) -> Option<tl::enums::InputPeer> {
        Some(tl::types::InputPeerUser { user_id: self.user_id, access_hash: self.access_hash? }.into())
    }
}

pub fn user_label(u: &tl::types::User) -> String {
    if u.deleted {
        return "Deleted account".into();
    }
    let name = [u.first_name.as_deref(), u.last_name.as_deref()]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    match (&u.username, name.is_empty()) {
        (Some(un), true) => format!("@{un}"),
        (Some(un), false) => format!("{name} (@{un})"),
        (None, true) => u.id.to_string(),
        (None, false) => name,
    }
}

pub struct Counts {
    pub members: i64,
    pub pending: i64,
}

pub async fn counts(tg: &Tg, ch: ChannelRef) -> Result<Counts, InvocationError> {
    let tl::enums::messages::ChatFull::Full(full) =
        tg.call(&tl::functions::channels::GetFullChannel { channel: ch.input_channel() }).await?;
    Ok(match full.full_chat {
        tl::enums::ChatFull::ChannelFull(f) => Counts {
            members: f.participants_count.unwrap_or(0) as i64,
            pending: f.requests_pending.unwrap_or(0) as i64,
        },
        tl::enums::ChatFull::Full(_) => Counts { members: 0, pending: 0 },
    })
}

/// Where the next page of pending requests starts.
#[derive(Clone)]
pub struct Cursor {
    date: i32,
    user: tl::enums::InputUser,
}

pub struct Page {
    pub items: Vec<Requester>,
    pub total: i64,
    /// `None` once a page brings nothing new: the end of the list.
    pub next: Option<Cursor>,
}

/// One page of pending requests, newest first. Pages are not a fixed size
/// (100, then 99s), so only a page with nothing new marks the end.
pub async fn pending_page(
    tg: &Tg,
    ch: ChannelRef,
    from: Option<&Cursor>,
) -> Result<Page, InvocationError> {
    let tl::enums::messages::ChatInviteImporters::Importers(res) = tg
        .call(&tl::functions::messages::GetChatInviteImporters {
            requested: true,
            subscription_expired: false,
            peer: ch.input_peer(),
            link: None,
            q: None,
            offset_date: from.map_or(0, |c| c.date),
            offset_user: from.map_or(tl::enums::InputUser::Empty, |c| c.user.clone()),
            limit: 100,
        })
        .await?;
    let users: HashMap<i64, tl::types::User> = res
        .users
        .into_iter()
        .filter_map(|u| match u {
            tl::enums::User::User(u) => Some((u.id, u)),
            tl::enums::User::Empty(_) => None,
        })
        .collect();
    let mut items = Vec::new();
    let mut last = None;
    for tl::enums::ChatInviteImporter::Importer(imp) in res.importers {
        last = Some((imp.user_id, imp.date));
        let user = users.get(&imp.user_id);
        items.push(Requester {
            user_id: imp.user_id,
            access_hash: user.and_then(|u| u.access_hash),
            name: user.map(user_label).unwrap_or_else(|| imp.user_id.to_string()),
            date: imp.date,
            thumb: user.and_then(|u| match &u.photo {
                Some(tl::enums::UserProfilePhoto::Photo(p)) => p.stripped_thumb.as_deref(),
                _ => None,
            })
            .and_then(media::stripped_to_data_url),
        });
    }
    let next = last.and_then(|(uid, date)| {
        let hash = users.get(&uid)?.access_hash?;
        Some(Cursor { date, user: tl::types::InputUser { user_id: uid, access_hash: hash }.into() })
    });
    Ok(Page { items, total: res.count as i64, next })
}

/// Every pending request, all pages; the caller compares the total with
/// Telegram's count.
pub async fn pending(tg: &Tg, ch: ChannelRef) -> Result<(Vec<Requester>, i64), InvocationError> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut cursor = None;
    let mut total = None;
    loop {
        let page = pending_page(tg, ch, cursor.as_ref()).await?;
        total.get_or_insert(page.total);
        let before = out.len();
        out.extend(page.items.into_iter().filter(|r| seen.insert(r.user_id)));
        if out.len() == before || page.next.is_none() {
            return Ok((out, total.unwrap_or(0)));
        }
        cursor = page.next;
    }
}

async fn admin_log_head(tg: &Tg, ch: ChannelRef) -> Result<i64, InvocationError> {
    let tl::enums::channels::AdminLogResults::Results(res) = tg
        .call(&tl::functions::channels::GetAdminLog {
            channel: ch.input_channel(),
            q: String::new(),
            events_filter: None,
            admins: None,
            max_id: 0,
            min_id: 0,
            limit: 1,
        })
        .await?;
    Ok(res
        .events
        .first()
        .map(|tl::enums::ChannelAdminLogEvent::Event(e)| e.id)
        .unwrap_or(0))
}

fn membership_filter() -> tl::enums::ChannelAdminLogEventsFilter {
    tl::types::ChannelAdminLogEventsFilter {
        join: true,
        leave: true,
        invite: true,
        ban: true,
        unban: false,
        kick: true,
        unkick: false,
        promote: false,
        demote: false,
        info: false,
        settings: false,
        pinned: false,
        edit: false,
        delete: false,
        group_call: false,
        invites: false,
        send: false,
        forums: false,
        sub_extend: false,
        edit_rank: false,
    }
    .into()
}

#[derive(Default)]
struct LogTally {
    by_request: HashSet<i64>,
    other_joins: usize,
    leaves: usize,
    removed: usize,
}

async fn admin_log_since(tg: &Tg, ch: ChannelRef, min_id: i64) -> Result<LogTally, InvocationError> {
    use tl::enums::ChannelAdminLogEventAction as A;
    let mut tally = LogTally::default();
    let mut max_id = 0;
    loop {
        let tl::enums::channels::AdminLogResults::Results(res) = tg
            .call(&tl::functions::channels::GetAdminLog {
                channel: ch.input_channel(),
                q: String::new(),
                events_filter: Some(membership_filter()),
                admins: None,
                max_id,
                min_id,
                limit: 100,
            })
            .await?;
        let n = res.events.len();
        for tl::enums::ChannelAdminLogEvent::Event(ev) in res.events {
            max_id = if max_id == 0 { ev.id } else { max_id.min(ev.id) };
            match ev.action {
                A::ParticipantJoinByRequest(_) => {
                    tally.by_request.insert(ev.user_id);
                }
                A::ParticipantJoin | A::ParticipantJoinByInvite(_) | A::ParticipantInvite(_) => {
                    tally.other_joins += 1
                }
                A::ParticipantLeave => tally.leaves += 1,
                A::ParticipantToggleBan(_) => tally.removed += 1,
                _ => {}
            }
        }
        if n < 100 {
            return Ok(tally);
        }
    }
}

async fn is_member(tg: &Tg, ch: ChannelRef, r: &Requester) -> Result<bool, InvocationError> {
    let Some(peer) = r.input_peer() else { return Ok(false) };
    match tg
        .call(&tl::functions::channels::GetParticipant {
            channel: ch.input_channel(),
            participant: peer,
        })
        .await
    {
        Ok(tl::enums::channels::ChannelParticipant::Participant(p)) => Ok(!matches!(
            p.participant,
            tl::enums::ChannelParticipant::Left(_) | tl::enums::ChannelParticipant::Banned(_)
        )),
        Err(InvocationError::Rpc(e)) if e.name == "USER_NOT_PARTICIPANT" => Ok(false),
        Err(e) => Err(e),
    }
}

struct Run<'a> {
    tg: &'a Tg,
    app: &'a AppHandle,
    ch: ChannelRef,
    verify_each: bool,
    audit: File,
    approved: BTreeMap<i64, Requester>,
    verified: HashSet<i64>,
    already: usize,
    gone: usize,
    parked: BTreeMap<i64, (Requester, String)>,
}

impl Run<'_> {
    fn record(&mut self, r: &Requester, outcome: &'static str, detail: &str) {
        let line = serde_json::json!({
            "at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            "user_id": r.user_id, "who": r.name, "outcome": outcome, "detail": detail,
        });
        let _ = writeln!(self.audit, "{line}");
        let _ = self.audit.flush();
        let _ = self.app.emit(
            "job",
            JobEvent::Item {
                user_id: r.user_id,
                name: r.name.clone(),
                thumb: r.thumb.clone(),
                outcome,
                detail: detail.to_string(),
                explanation: if detail.is_empty() { "" } else { explain(detail) },
            },
        );
    }

    async fn approve(&mut self, r: &Requester) -> Result<(), String> {
        let Some(user) = r.input_user() else {
            self.park(r, "USER_NOT_RETURNED");
            return Ok(());
        };
        let res = self
            .tg
            .call(&tl::functions::messages::HideChatJoinRequest {
                approved: true,
                peer: self.ch.input_peer(),
                user_id: user,
            })
            .await;
        if let Err(e) = res {
            let reason = error_name(&e);
            if FATAL.contains(&reason.as_str()) {
                self.record(r, "fatal", &reason);
                return Err(format!("Telegram stopped the run: {reason}"));
            }
            self.parked.remove(&r.user_id);
            if ALREADY.contains(&reason.as_str()) {
                self.already += 1;
                self.record(r, "already_member", &reason);
                return Ok(());
            }
            if GONE.contains(&reason.as_str()) {
                self.gone += 1;
                self.record(r, "request_gone", &reason);
                return Ok(());
            }
            // Known permanent reasons and anything unrecognised alike: left
            // pending, retried at the end, listed in the report.
            self.park(r, &reason);
            return Ok(());
        }

        self.parked.remove(&r.user_id);
        self.approved.insert(r.user_id, r.clone());
        if !self.verify_each {
            self.record(r, "approved", "");
            return Ok(());
        }
        let mut member = is_member(self.tg, self.ch, r).await.unwrap_or(false);
        if !member {
            tokio::time::sleep(Duration::from_secs(3)).await;
            member = is_member(self.tg, self.ch, r).await.unwrap_or(false);
        }
        if member {
            self.verified.insert(r.user_id);
            self.record(r, "approved_verified", "");
        } else {
            // Approved, but not a member a moment later: they may have left
            // straight away. The admin log settles it at the end.
            self.record(r, "approved_unverified", "");
        }
        Ok(())
    }

    fn park(&mut self, r: &Requester, reason: &str) {
        self.parked.insert(r.user_id, (r.clone(), reason.to_string()));
        self.record(r, "parked", reason);
    }
}

fn emit(app: &AppHandle, ev: JobEvent) {
    let _ = app.emit("job", ev);
}

fn status(app: &AppHandle, text: &str) {
    emit(app, JobEvent::Status { text: text.into() });
}

pub async fn run(
    app: AppHandle,
    tg: Arc<Tg>,
    ch: ChannelRef,
    title: String,
    opts: Options,
    audit_dir: PathBuf,
    cancel: Arc<AtomicBool>,
) {
    if let Err(e) = run_inner(&app, &tg, ch, title, opts, audit_dir, &cancel).await {
        emit(&app, JobEvent::Failed { error: e });
    }
}

async fn run_inner(
    app: &AppHandle,
    tg: &Tg,
    ch: ChannelRef,
    title: String,
    opts: Options,
    audit_dir: PathBuf,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let e = |e: InvocationError| error_name(&e);
    tg.pacer.set_delay_ms(opts.delay_ms);
    tg.pacer.reset_stats();
    let t0 = Instant::now();

    status(app, "Reading the channel…");
    let start = counts(tg, ch).await.map_err(e)?;
    status(app, "Reading every pending request…");
    let (mut queue, total) = pending(tg, ch).await.map_err(e)?;
    if (queue.len() as i64) < total {
        status(app, &format!("Listed {} of the {} requests Telegram counts", queue.len(), total));
    }
    let log_head = admin_log_head(tg, ch).await.map_err(e)?;

    std::fs::create_dir_all(&audit_dir).map_err(|e| e.to_string())?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let audit_path = audit_dir.join(format!("approvals-{}-{stamp}.jsonl", ch.id));
    let audit = File::create(&audit_path).map_err(|e| e.to_string())?;
    emit(
        app,
        JobEvent::Started {
            title: title.clone(),
            start_count: start.members,
            pending: queue.len(),
            audit_path: audit_path.display().to_string(),
        },
    );

    let mut run = Run {
        tg,
        app,
        ch,
        verify_each: opts.verify_each,
        audit,
        approved: BTreeMap::new(),
        verified: HashSet::new(),
        already: 0,
        gone: 0,
        parked: BTreeMap::new(),
    };
    let budget = if opts.limit == 0 { usize::MAX } else { opts.limit as usize };
    let mut attempted = 0usize;
    let mut last_count: Option<i64> = None;
    let mut last_count_at = 0usize;

    let progress = |run: &Run, attempted: usize, remaining: usize, subs: Option<i64>| {
        let mins = t0.elapsed().as_secs_f64() / 60.0;
        emit(
            app,
            JobEvent::Progress {
                approved: run.approved.len(),
                parked: run.parked.len(),
                attempted,
                remaining,
                subscribers: subs,
                start_count: start.members,
                per_minute: if mins > 0.0 { attempted as f64 / mins } else { 0.0 },
                delay_ms: tg.pacer.delay_ms(),
            },
        );
    };

    // Re-read the list until nothing in it can still be acted on: new
    // requests arrive mid-run, and pages shift as requests are approved.
    let mut round = 0;
    'rounds: while attempted < budget && !cancel.load(Ordering::Relaxed) {
        round += 1;
        if round > 1 {
            status(app, "Re-reading the pending list…");
            queue = pending(tg, ch).await.map_err(e)?.0;
        }
        let todo: Vec<Requester> =
            queue.iter().filter(|r| !run.parked.contains_key(&r.user_id)).cloned().collect();
        if todo.is_empty() {
            break;
        }
        status(app, "Approving…");
        for (i, r) in todo.iter().enumerate() {
            if attempted >= budget || cancel.load(Ordering::Relaxed) {
                break 'rounds;
            }
            attempted += 1;
            run.approve(r).await?;
            // The subscriber count every 50 approvals: the running proof.
            if run.approved.len() - last_count_at >= 50 {
                last_count = counts(tg, ch).await.ok().map(|c| c.members).or(last_count);
                last_count_at = run.approved.len();
            }
            progress(&run, attempted, todo.len() - i - 1, last_count);
        }
    }

    // One more go at anything parked: some reasons clear on their own.
    if !run.parked.is_empty() && attempted < budget && !cancel.load(Ordering::Relaxed) {
        status(app, &format!("Retrying {} that could not be approved…", run.parked.len()));
        let still: HashSet<i64> = run.parked.keys().copied().collect();
        for r in pending(tg, ch).await.map_err(e)?.0 {
            if still.contains(&r.user_id) && attempted < budget && !cancel.load(Ordering::Relaxed) {
                attempted += 1;
                run.approve(&r).await?;
                progress(&run, attempted, 0, last_count);
            }
        }
    }

    // ---- Report ----
    status(app, "Waiting for Telegram's subscriber count to settle…");
    let mut end = counts(tg, ch).await.map_err(e)?;
    for _ in 0..5 {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let again = counts(tg, ch).await.map_err(e)?;
        let settled = again.members == end.members;
        end = again;
        if settled {
            break;
        }
    }

    status(app, "Checking the admin log…");
    let log = admin_log_since(tg, ch, log_head).await.map_err(e)?;
    let approved_ids: HashSet<i64> = run.approved.keys().copied().collect();

    // In the admin log as a join by request = it happened, whether or not
    // the user was still there when looked up.
    for id in approved_ids.intersection(&log.by_request) {
        run.verified.insert(*id);
    }
    // Not in the log: one last direct look before calling it unconfirmed.
    let mut unconfirmed = Vec::new();
    let to_check: Vec<Requester> = run
        .approved
        .values()
        .filter(|r| !run.verified.contains(&r.user_id))
        .cloned()
        .collect();
    if !to_check.is_empty() {
        status(app, &format!("Looking up {} approvals the log does not show…", to_check.len()));
    }
    for r in to_check {
        if is_member(tg, ch, &r).await.unwrap_or(false) {
            run.verified.insert(r.user_id);
        } else {
            unconfirmed.push(r.name.clone());
        }
    }
    let missing_from_log: Vec<String> = run
        .approved
        .values()
        .filter(|r| !log.by_request.contains(&r.user_id))
        .map(|r| r.name.clone())
        .collect();

    let expected = start.members + log.by_request.len() as i64 + log.other_joins as i64
        - log.leaves as i64
        - log.removed as i64;
    let report = Report {
        title,
        minutes: t0.elapsed().as_secs_f64() / 60.0,
        stopped_early: cancel.load(Ordering::Relaxed),
        verified: run.verified.len(),
        unverified: unconfirmed,
        already: run.already,
        gone: run.gone,
        parked: run
            .parked
            .values()
            .map(|(r, reason)| Stuck {
                user_id: r.user_id,
                name: r.name.clone(),
                thumb: r.thumb.clone(),
                reason: reason.clone(),
                explanation: explain(reason),
            })
            .collect(),
        joined_by_request: log.by_request.len(),
        ours_in_log: approved_ids.intersection(&log.by_request).count(),
        approved_elsewhere: log.by_request.difference(&approved_ids).count(),
        other_joins: log.other_joins,
        leaves: log.leaves,
        removed: log.removed,
        start_count: start.members,
        end_count: end.members,
        expected_count: expected,
        pending_now: end.pending,
        missing_from_log,
        flood_waits: tg.pacer.flood_waits.load(Ordering::Relaxed),
        flood_seconds: tg.pacer.flood_seconds.load(Ordering::Relaxed),
        final_delay_ms: tg.pacer.delay_ms(),
        audit_path: audit_path.display().to_string(),
    };
    emit(app, JobEvent::Done { report });
    Ok(())
}
