//! The Telegram connection: one client, one pace for every call it makes, and
//! a retry policy that waits out rate limits in full and says so.

use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use grammers_client::client::{ClientConfiguration, RetryContext, RetryPolicy};
use grammers_client::sender::RpcError;
use grammers_client::{tl, Client, InvocationError, SenderPool};
use rand::Rng;
use tauri::{AppHandle, Emitter};

use crate::session::FileSession;

include!(concat!(env!("OUT_DIR"), "/api_keys.rs"));

/// Longest rate-limit wait slept out automatically. Anything longer is
/// reported instead: an hour-plus wait is better decided by a person.
const MAX_FLOOD_WAIT: u32 = 3600;
const MAX_TRANSIENT_RETRIES: u32 = 5;
/// Ceiling for the pace after repeated backoffs.
const MAX_DELAY_MS: u64 = 30_000;

/// Spaces out every call the app makes, and slows down for good after every
/// rate-limit wait Telegram imposes.
pub struct Pacer {
    delay_ms: AtomicU64,
    last: tokio::sync::Mutex<Instant>,
    pub flood_waits: AtomicU32,
    pub flood_seconds: AtomicU64,
}

impl Pacer {
    fn new(delay_ms: u64) -> Self {
        Self {
            delay_ms: AtomicU64::new(delay_ms),
            last: tokio::sync::Mutex::new(Instant::now() - Duration::from_secs(60)),
            flood_waits: AtomicU32::new(0),
            flood_seconds: AtomicU64::new(0),
        }
    }

    pub fn delay_ms(&self) -> u64 {
        self.delay_ms.load(Ordering::Relaxed)
    }

    pub fn set_delay_ms(&self, ms: u64) {
        self.delay_ms.store(ms.clamp(200, MAX_DELAY_MS), Ordering::Relaxed);
    }

    pub fn reset_stats(&self) {
        self.flood_waits.store(0, Ordering::Relaxed);
        self.flood_seconds.store(0, Ordering::Relaxed);
    }

    async fn wait(&self) {
        let mut last = self.last.lock().await;
        let jitter = rand::rng().random_range(0.9..1.2);
        let gap = Duration::from_millis((self.delay_ms() as f64 * jitter) as u64);
        let since = last.elapsed();
        if since < gap {
            tokio::time::sleep(gap - since).await;
        }
        *last = Instant::now();
    }

    fn flooded(&self, seconds: u32) {
        self.flood_waits.fetch_add(1, Ordering::Relaxed);
        self.flood_seconds.fetch_add(seconds as u64, Ordering::Relaxed);
        let slower = (self.delay_ms() as f64 * 1.5) as u64;
        self.delay_ms.store(slower.min(MAX_DELAY_MS), Ordering::Relaxed);
    }
}

#[derive(Clone, serde::Serialize)]
pub struct FloodEvent {
    pub seconds: u32,
    pub delay_ms: u64,
}

/// Rate limits are slept out in full (and announced); Telegram's own hiccups
/// and dropped connections get a few retries with backoff; anything else is an
/// answer, and goes back to the caller.
struct Policy {
    pacer: Arc<Pacer>,
    app: AppHandle,
}

impl RetryPolicy for Policy {
    fn should_retry(&self, ctx: &RetryContext) -> ControlFlow<(), Duration> {
        let n = ctx.fail_count.get();
        match &ctx.error {
            InvocationError::Rpc(RpcError { code: 420, value: Some(seconds), .. })
                if *seconds <= MAX_FLOOD_WAIT && n <= 10 =>
            {
                self.pacer.flooded(*seconds);
                let _ = self.app.emit(
                    "flood",
                    FloodEvent { seconds: *seconds, delay_ms: self.pacer.delay_ms() },
                );
                let pad = rand::rng().random_range(1000..3000);
                ControlFlow::Continue(Duration::from_secs(*seconds as u64) + Duration::from_millis(pad))
            }
            InvocationError::Rpc(RpcError { code, .. })
                if (*code >= 500 || *code < 0) && n <= MAX_TRANSIENT_RETRIES =>
            {
                ControlFlow::Continue(Duration::from_secs(2 * n as u64))
            }
            InvocationError::Io(_) | InvocationError::Transport(_) | InvocationError::Dropped
                if n <= MAX_TRANSIENT_RETRIES => {
                ControlFlow::Continue(Duration::from_secs(n as u64))
            }
            _ => ControlFlow::Break(()),
        }
    }
}

pub struct Tg {
    pub client: Client,
    pub session: Arc<FileSession>,
    pub pacer: Arc<Pacer>,
}

impl Tg {
    pub fn connect(app: &AppHandle, session_path: PathBuf, pacer: Arc<Pacer>) -> Self {
        let session = Arc::new(FileSession::open(session_path));
        let SenderPool { runner, handle, .. } = SenderPool::new(Arc::clone(&session), API_ID);
        let client = Client::with_configuration(
            handle,
            ClientConfiguration {
                retry_policy: Box::new(Policy { pacer: Arc::clone(&pacer), app: app.clone() }),
                ..Default::default()
            },
        );
        tauri::async_runtime::spawn(runner.run());
        Self { client, session, pacer }
    }

    pub fn new_pacer() -> Arc<Pacer> {
        Arc::new(Pacer::new(1500))
    }

    /// Waits for this call's turn, for requests made outside `call`
    /// (file downloads).
    pub async fn pace(&self) {
        self.pacer.wait().await;
    }

    /// Every raw call goes through here, so all of them share one pace.
    pub async fn call<R: tl::RemoteCall>(&self, request: &R) -> Result<R::Return, InvocationError> {
        self.pacer.wait().await;
        self.client.invoke(request).await
    }
}

/// Telegram's name for an error ("USER_CHANNELS_TOO_MUCH"), or a plain
/// description when it is not one of Telegram's.
pub fn error_name(e: &InvocationError) -> String {
    match e {
        InvocationError::Rpc(rpc) => rpc.name.clone(),
        InvocationError::Io(_) | InvocationError::Transport(_) | InvocationError::Dropped => {
            "CONNECTION_LOST".into()
        }
        other => other.to_string(),
    }
}
