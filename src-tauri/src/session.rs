//! The login, kept as a small JSON file in the app's data folder.
//!
//! Only the datacenter list (which carries the auth keys) and the home
//! datacenter are written to disk; peers and update state live in memory,
//! because the app reads every access hash it needs from Telegram's replies.

use std::fmt;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use grammers_session::types::{
    ChannelState, DcOption, PeerId, PeerInfo, UpdateState, UpdatesState,
};
use grammers_session::{BoxFuture, Session, SessionData};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct OnDisk {
    home_dc: i32,
    dc_options: Vec<DcOption>,
}

#[derive(Debug)]
pub struct SessionError(String);

impl std::error::Error for SessionError {}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "session: {}", self.0)
    }
}

pub struct FileSession {
    path: PathBuf,
    data: Mutex<SessionData>,
}

impl FileSession {
    pub fn open(path: PathBuf) -> Self {
        let mut data = SessionData::default();
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(saved) = serde_json::from_str::<OnDisk>(&text) {
                data.home_dc = saved.home_dc;
                for dc in saved.dc_options {
                    data.dc_options.insert(dc.id, dc);
                }
            }
        }
        Self { path, data: Mutex::new(data) }
    }

    fn data(&self) -> Result<MutexGuard<'_, SessionData>, SessionError> {
        self.data.lock().map_err(|_| SessionError("lock poisoned".into()))
    }

    /// Written to a temporary file and renamed over the old one, so a crash
    /// mid-write never leaves half a login behind.
    fn save(&self, data: &SessionData) -> Result<(), SessionError> {
        let mut dcs: Vec<DcOption> = data.dc_options.values().cloned().collect();
        dcs.sort_by_key(|d| d.id);
        let json = serde_json::to_vec(&OnDisk { home_dc: data.home_dc, dc_options: dcs })
            .map_err(|e| SessionError(e.to_string()))?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| SessionError(e.to_string()))?;
        }
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, json).map_err(|e| SessionError(e.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp, &self.path).map_err(|e| SessionError(e.to_string()))
    }

    /// Forget the login, on disk and in memory.
    pub fn wipe(&self) {
        let _ = std::fs::remove_file(&self.path);
        if let Ok(mut data) = self.data.lock() {
            *data = SessionData::default();
        }
    }
}

impl Session for FileSession {
    type Error = SessionError;

    fn home_dc_id(&self) -> Result<i32, SessionError> {
        Ok(self.data()?.home_dc)
    }

    fn set_home_dc_id(&self, dc_id: i32) -> BoxFuture<'_, Result<(), SessionError>> {
        Box::pin(async move {
            let mut data = self.data()?;
            data.home_dc = dc_id;
            self.save(&data)
        })
    }

    fn dc_option(&self, dc_id: i32) -> Result<Option<DcOption>, SessionError> {
        Ok(self.data()?.dc_options.get(&dc_id).cloned())
    }

    fn set_dc_option(&self, dc_option: &DcOption) -> BoxFuture<'_, Result<(), SessionError>> {
        let dc_option = dc_option.clone();
        Box::pin(async move {
            let mut data = self.data()?;
            data.dc_options.insert(dc_option.id, dc_option);
            self.save(&data)
        })
    }

    fn peer(&self, peer: PeerId) -> BoxFuture<'_, Result<Option<PeerInfo>, SessionError>> {
        Box::pin(async move { Ok(self.data()?.peer_infos.get(&peer).cloned()) })
    }

    fn cache_peer(&self, peer: &PeerInfo) -> BoxFuture<'_, Result<(), SessionError>> {
        let peer = peer.clone();
        Box::pin(async move {
            self.data()?
                .peer_infos
                .entry(peer.id())
                .or_insert_with(|| peer.clone())
                .extend_info(&peer);
            Ok(())
        })
    }

    fn updates_state(&self) -> BoxFuture<'_, Result<UpdatesState, SessionError>> {
        Box::pin(async move { Ok(self.data()?.updates_state.clone()) })
    }

    fn set_update_state(&self, update: UpdateState) -> BoxFuture<'_, Result<(), SessionError>> {
        Box::pin(async move {
            let mut data = self.data()?;
            match update {
                UpdateState::All(state) => data.updates_state = state,
                UpdateState::Primary { pts, date, seq } => {
                    data.updates_state.pts = pts;
                    data.updates_state.date = date;
                    data.updates_state.seq = seq;
                }
                UpdateState::Secondary { qts } => data.updates_state.qts = qts,
                UpdateState::Channel { id, pts } => {
                    data.updates_state.channels.retain(|c| c.id != id);
                    data.updates_state.channels.push(ChannelState { id, pts });
                }
            }
            Ok(())
        })
    }
}
