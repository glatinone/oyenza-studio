//! Translation lines for lyrics (`poni_core::translate`): one worker thread
//! (`poni-translate`) that the app hands a song's lines once its lyrics are
//! in. The latest song wins: a queued older one is dropped, and an answer for
//! a song that is no longer playing is ignored by the app (`track_key`).
//!
//! Translations are kept on disk next to the lyrics cache
//! (`lyrics\tr-<to>-<sha256 of the lines>.json`), so a song played again
//! shows its translation at once and costs no request.

use crate::events::{AppEvent, EventSink};
use crate::services::http::{Client, MtaGuard};
use poni_core::{sha256, translate};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

const USER_AGENT: &str = concat!("Floaty/", env!("CARGO_PKG_VERSION"));
const TIMEOUT: Duration = Duration::from_secs(8);

struct Job {
    track_key: String,
    lines: Vec<String>,
    to: String,
}

pub struct TranslateService {
    tx: Sender<Job>,
}

impl TranslateService {
    pub fn start(sink: EventSink) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("poni-translate".into())
            .spawn(move || worker(rx, sink))
            .map_err(|e| std::io::Error::other(format!("could not start the translation thread: {e}")))?;
        Ok(Self { tx })
    }

    /// Translate `lines` into `to` for `track_key` and return at once; the
    /// answer comes back as `AppEvent::LyricsTranslation`.
    pub fn request(&self, track_key: &str, lines: Vec<String>, to: &str) {
        let _ = self.tx.send(Job { track_key: track_key.to_string(), lines, to: to.to_string() });
    }
}

fn worker(rx: Receiver<Job>, sink: EventSink) {
    // Declared first so it is dropped last, after the client.
    let _com = MtaGuard::new();
    let mut client: Option<Client> = None;
    while let Ok(mut job) = rx.recv() {
        // Only the newest song matters.
        while let Ok(newer) = rx.try_recv() {
            job = newer;
        }
        let lines = translated(&job, &mut client);
        sink.send(AppEvent::LyricsTranslation { track_key: job.track_key, lines });
    }
}

/// The song's translation, from the disk cache or the network ("" lines when
/// none could be had).
fn translated(job: &Job, client: &mut Option<Client>) -> Vec<String> {
    let path = cache_path(job);
    if let Some(lines) = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok()).filter(|l| l.len() == job.lines.len()) {
        return lines;
    }
    if client.is_none() {
        *client = Client::new(USER_AGENT).map_err(|e| crate::pwarn!("translate: {e}")).ok();
    }
    let Some(c) = client.as_ref() else { return vec![String::new(); job.lines.len()] };
    let texts: Vec<&str> = job.lines.iter().map(String::as_str).collect();
    let fetch = |url: &str| match c.get(url, TIMEOUT) {
        Ok(r) if r.status == 200 => Ok(r.body),
        Ok(r) => Err(format!("translation service answered {}", r.status)),
        Err(e) => Err(e.to_string()),
    };
    match translate::translate_song(&texts, &job.to, fetch) {
        Ok(lines) => {
            let shown = lines.iter().filter(|l| !l.is_empty()).count();
            crate::pinfo!("translate: {shown} of {} lines into {}", lines.len(), job.to);
            if let Ok(json) = serde_json::to_string(&lines) {
                let _ = std::fs::create_dir_all(crate::paths::lyrics_cache_dir());
                let _ = std::fs::write(&path, json);
            }
            lines
        }
        Err(e) => {
            crate::pwarn!("translate: {e}");
            vec![String::new(); job.lines.len()]
        }
    }
}

fn cache_path(job: &Job) -> PathBuf {
    let digest = sha256::sha256(job.lines.join("\n").as_bytes());
    crate::paths::lyrics_cache_dir().join(format!("tr-{}-{}.json", job.to, &sha256::hex(&digest)[..24]))
}
