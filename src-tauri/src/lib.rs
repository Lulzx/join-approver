mod approver;
mod media;
mod session;
mod tg;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use grammers_client::client::{LoginToken, PasswordToken};
use grammers_client::peer::Peer;
use grammers_client::{tl, SignInError};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Mutex;

use approver::{ChannelRef, Cursor};
use media::Cache;
use tg::{error_name, Tg};

#[derive(Clone)]
struct Known {
    ch: ChannelRef,
    title: String,
    photo_id: Option<i64>,
}

struct Job {
    cancel: Arc<AtomicBool>,
    done: tauri::async_runtime::JoinHandle<()>,
}

#[derive(Default)]
struct AppState {
    tg: Mutex<Option<Arc<Tg>>>,
    login: Mutex<Option<LoginToken>>,
    password: Mutex<Option<PasswordToken>>,
    channels: Mutex<HashMap<i64, Known>>,
    /// Where "Show more" picks up, per channel.
    cursors: Mutex<HashMap<i64, Cursor>>,
    job: Mutex<Option<Job>>,
}

fn data_dir(app: &AppHandle) -> PathBuf {
    app.path().app_data_dir().expect("no app data folder on this system")
}

fn cache(app: &AppHandle) -> Cache {
    Cache(data_dir(app).join("cache"))
}

async fn tg(state: &AppState) -> Result<Arc<Tg>, String> {
    state.tg.lock().await.clone().ok_or_else(|| "Not connected yet".to_string())
}

/// Words a person can act on, for the errors a login can hit.
fn friendly(name: &str) -> String {
    match name {
        "PHONE_NUMBER_INVALID" => "That phone number isn't valid. Include the country code, like +91….".into(),
        "PHONE_NUMBER_BANNED" => "Telegram has banned this phone number.".into(),
        "PHONE_NUMBER_FLOOD" | "FLOOD_WAIT" => "Too many attempts. Try again later.".into(),
        "PHONE_CODE_EXPIRED" => "That code expired. Request a new one.".into(),
        "PHONE_CODE_INVALID" => "That code isn't right.".into(),
        "PASSWORD_HASH_INVALID" => "That password isn't right.".into(),
        "CONNECTION_LOST" => "Can't reach Telegram. Check the internet connection.".into(),
        other => format!("Telegram said: {other}"),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Me {
    name: String,
    username: Option<String>,
    phone: Option<String>,
}

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
enum AuthState {
    SignedOut,
    SignedIn { me: Me },
    #[serde(rename_all = "camelCase")]
    PasswordNeeded { hint: Option<String> },
}

fn me_from(user: &grammers_client::peer::User) -> Me {
    Me {
        name: user.full_name(),
        username: user.username().map(str::to_string),
        phone: user.phone().map(str::to_string),
    }
}

#[tauri::command]
fn platform() -> &'static str {
    std::env::consts::OS
}

#[tauri::command]
async fn auth_state(state: State<'_, AppState>) -> Result<AuthState, String> {
    let tg = tg(&state).await?;
    if !tg.client.is_authorized().await.map_err(|e| friendly(&error_name(&e)))? {
        return Ok(AuthState::SignedOut);
    }
    let me = tg.client.get_me().await.map_err(|e| friendly(&error_name(&e)))?;
    Ok(AuthState::SignedIn { me: me_from(&me) })
}

#[tauri::command]
async fn send_code(state: State<'_, AppState>, phone: String) -> Result<(), String> {
    let tg = tg(&state).await?;
    let phone: String = phone.chars().filter(|c| c.is_ascii_digit() || *c == '+').collect();
    let token = tg
        .client
        .request_login_code(&phone, tg::API_HASH)
        .await
        .map_err(|e| friendly(&error_name(&e)))?;
    *state.login.lock().await = Some(token);
    Ok(())
}

#[tauri::command]
async fn sign_in(state: State<'_, AppState>, code: String) -> Result<AuthState, String> {
    let tg = tg(&state).await?;
    let guard = state.login.lock().await;
    let token = guard.as_ref().ok_or("Request a code first")?;
    let code: String = code.chars().filter(char::is_ascii_digit).collect();
    match tg.client.sign_in(token, &code).await {
        Ok(user) => Ok(AuthState::SignedIn { me: me_from(&user) }),
        Err(SignInError::PasswordRequired(pt)) => {
            let hint = pt.hint().map(str::to_string);
            *state.password.lock().await = Some(pt);
            Ok(AuthState::PasswordNeeded { hint })
        }
        Err(SignInError::InvalidCode) => Err(friendly("PHONE_CODE_INVALID")),
        Err(SignInError::SignUpRequired) => {
            Err("This number has no Telegram account. Sign up in the Telegram app first.".into())
        }
        Err(SignInError::InvalidPassword(_)) => Err(friendly("PASSWORD_HASH_INVALID")),
        Err(SignInError::Other(e)) => Err(friendly(&error_name(&e))),
    }
}

#[tauri::command]
async fn check_password(state: State<'_, AppState>, password: String) -> Result<AuthState, String> {
    let tg = tg(&state).await?;
    let token = state.password.lock().await.take().ok_or("Sign in again")?;
    match tg.client.check_password(token, password.as_bytes()).await {
        Ok(user) => Ok(AuthState::SignedIn { me: me_from(&user) }),
        Err(SignInError::InvalidPassword(pt)) => {
            *state.password.lock().await = Some(pt);
            Err(friendly("PASSWORD_HASH_INVALID"))
        }
        Err(SignInError::Other(e)) => Err(friendly(&error_name(&e))),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
async fn sign_out(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    stop_job(&state).await;
    let old = state.tg.lock().await.take();
    if let Some(old) = old {
        let _ = old.client.sign_out().await;
        old.client.disconnect();
        old.session.wipe();
    }
    state.channels.lock().await.clear();
    state.cursors.lock().await.clear();
    cache(&app).clear();
    let fresh = Tg::connect(&app, data_dir(&app).join("session.json"), Tg::new_pacer());
    *state.tg.lock().await = Some(Arc::new(fresh));
    Ok(())
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ChannelInfo {
    id: i64,
    title: String,
    username: Option<String>,
    is_group: bool,
    members: Option<i64>,
    /// "owner" or "admin": nothing else is listed.
    role: &'static str,
    /// Telegram only lets admins with the invite-users right approve requests.
    can_approve: bool,
    /// Inline preview, shown until the real photo is in.
    thumb: Option<String>,
    has_photo: bool,
}

/// Every channel and supergroup this account created or administers. Admins
/// without the right to approve are listed too, marked, so they don't seem
/// to have vanished.
#[tauri::command]
async fn list_channels(app: AppHandle, state: State<'_, AppState>) -> Result<Vec<ChannelInfo>, String> {
    let tg = tg(&state).await?;
    let mut dialogs = tg.client.iter_dialogs();
    let mut out = Vec::new();
    let mut refs = HashMap::new();
    while let Some(d) = dialogs.next().await.map_err(|e| friendly(&error_name(&e)))? {
        let raw = match d.peer() {
            Peer::Channel(c) => c.raw.clone(),
            Peer::Group(g) => match &g.raw {
                tl::enums::Chat::Channel(c) => c.clone(),
                _ => continue,
            },
            Peer::User(_) => continue,
        };
        let rights = raw.admin_rights.as_ref().map(|tl::enums::ChatAdminRights::Rights(r)| r);
        if raw.left || !(raw.creator || rights.is_some()) {
            continue;
        }
        let Some(access_hash) = raw.access_hash else { continue };
        let can_approve = raw.creator || rights.is_some_and(|r| r.invite_users);
        let photo = match &raw.photo {
            tl::enums::ChatPhoto::Photo(p) => Some(p),
            tl::enums::ChatPhoto::Empty => None,
        };
        refs.insert(
            raw.id,
            Known {
                ch: ChannelRef { id: raw.id, access_hash },
                title: raw.title.clone(),
                photo_id: photo.map(|p| p.photo_id),
            },
        );
        let info = ChannelInfo {
            id: raw.id,
            title: raw.title.clone(),
            username: raw.username.clone(),
            is_group: raw.megagroup,
            members: raw.participants_count.map(i64::from),
            role: if raw.creator { "owner" } else { "admin" },
            can_approve,
            thumb: photo.and_then(|p| p.stripped_thumb.as_deref()).and_then(media::stripped_to_data_url),
            has_photo: photo.is_some(),
        };
        // Usable, and on screen, the moment it's found: a long chat list
        // shouldn't keep every channel waiting for the last one.
        state.channels.lock().await.insert(raw.id, refs[&raw.id].clone());
        let _ = app.emit("channel-found", info.clone());
        out.push(info);
    }
    // The complete set replaces the old one, dropping channels since left.
    *state.channels.lock().await = refs;
    Ok(out)
}

async fn channel(state: &AppState, id: i64) -> Result<Known, String> {
    state
        .channels
        .lock()
        .await
        .get(&id)
        .cloned()
        .ok_or_else(|| "That channel isn't in the list any more. Refresh.".to_string())
}

/// The channel's photo, from disk when already fetched.
#[tauri::command]
async fn channel_photo(app: AppHandle, state: State<'_, AppState>, id: i64) -> Result<Option<String>, String> {
    let k = channel(&state, id).await?;
    let Some(photo_id) = k.photo_id else { return Ok(None) };
    let tg = tg(&state).await?;
    let dir = data_dir(&app).join("cache").join("photos");
    media::photo(&tg, &dir, &format!("c{id}"), k.ch.input_peer(), photo_id)
        .await
        .map(Some)
        .map_err(|e| error_name(&e))
}

/// The signed-in account's photo.
#[tauri::command]
async fn me_photo(app: AppHandle, state: State<'_, AppState>) -> Result<Option<String>, String> {
    let tg = tg(&state).await?;
    let me = tg.client.get_me().await.map_err(|e| error_name(&e))?;
    let Some(photo_id) = me.photo().map(|p| p.photo_id) else { return Ok(None) };
    let dir = data_dir(&app).join("cache").join("photos");
    media::photo(&tg, &dir, "me", tl::enums::InputPeer::PeerSelf, photo_id)
        .await
        .map(Some)
        .map_err(|e| error_name(&e))
}

#[tauri::command]
fn cache_read(app: AppHandle, key: String) -> Option<String> {
    cache(&app).read(&key)
}

#[tauri::command]
fn cache_write(app: AppHandle, key: String, json: String) -> Result<(), String> {
    cache(&app).write(&key, &json)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Stats {
    members: i64,
    pending: i64,
}

#[tauri::command]
async fn channel_stats(state: State<'_, AppState>, id: i64) -> Result<Stats, String> {
    let tg = tg(&state).await?;
    let ch = channel(&state, id).await?.ch;
    let c = approver::counts(&tg, ch).await.map_err(|e| friendly(&error_name(&e)))?;
    Ok(Stats { members: c.members, pending: c.pending })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PendingItem {
    user_id: i64,
    name: String,
    date: i32,
    thumb: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PendingPage {
    telegram_count: i64,
    items: Vec<PendingItem>,
    more: bool,
}

/// One page of who's waiting: the first with `more = false`, the next one
/// after that on every "Show more".
#[tauri::command]
async fn pending_page(state: State<'_, AppState>, id: i64, more: bool) -> Result<PendingPage, String> {
    let tg = tg(&state).await?;
    let ch = channel(&state, id).await?.ch;
    let cursor = if more { state.cursors.lock().await.get(&id).cloned() } else { None };
    if more && cursor.is_none() {
        return Ok(PendingPage { telegram_count: 0, items: vec![], more: false });
    }
    let page = approver::pending_page(&tg, ch, cursor.as_ref())
        .await
        .map_err(|e| friendly(&error_name(&e)))?;
    let has_more = !page.items.is_empty() && page.next.is_some();
    match page.next.filter(|_| has_more) {
        Some(next) => state.cursors.lock().await.insert(id, next),
        None => state.cursors.lock().await.remove(&id),
    };
    Ok(PendingPage {
        telegram_count: page.total,
        items: page
            .items
            .into_iter()
            .map(|r| PendingItem { user_id: r.user_id, name: r.name, date: r.date, thumb: r.thumb })
            .collect(),
        more: has_more,
    })
}

#[tauri::command]
async fn start_approval(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
    options: approver::Options,
) -> Result<(), String> {
    let mut job = state.job.lock().await;
    if job.as_ref().is_some_and(|j| !j.done.inner().is_finished()) {
        return Err("A run is already going".into());
    }
    let tg = tg(&state).await?;
    let Known { ch, title, .. } = channel(&state, id).await?;
    let cancel = Arc::new(AtomicBool::new(false));
    let audit_dir = data_dir(&app).join("audit");
    let done = tauri::async_runtime::spawn(approver::run(
        app.clone(),
        tg,
        ch,
        title,
        options,
        audit_dir,
        Arc::clone(&cancel),
    ));
    *job = Some(Job { cancel, done });
    Ok(())
}

async fn stop_job(state: &AppState) {
    if let Some(job) = state.job.lock().await.as_ref() {
        job.cancel.store(true, Ordering::Relaxed);
    }
}

/// Finishes the user in hand, then writes the report as usual.
#[tauri::command]
async fn stop_approval(state: State<'_, AppState>) -> Result<(), String> {
    stop_job(&state).await;
    Ok(())
}

#[tauri::command]
fn audit_folder(app: AppHandle) -> String {
    data_dir(&app).join("audit").display().to_string()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(AppState::default())
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::async_runtime::block_on(async {
                let tg = Tg::connect(&handle, data_dir(&handle).join("session.json"), Tg::new_pacer());
                *handle.state::<AppState>().tg.lock().await = Some(Arc::new(tg));
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            platform,
            auth_state,
            send_code,
            sign_in,
            check_password,
            sign_out,
            list_channels,
            channel_stats,
            pending_page,
            channel_photo,
            me_photo,
            cache_read,
            cache_write,
            start_approval,
            stop_approval,
            audit_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
