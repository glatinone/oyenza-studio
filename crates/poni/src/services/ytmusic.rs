//! Search YouTube Music from the island and play the pick in the person's
//! own YouTube Music tab in Edge.
//!
//! Search is YouTube Music's own web search (InnerTube, the songs filter),
//! asked without signing in, on a short-lived thread. Playing goes through
//! jx, Kiell's local browser tool, whose Edge extension points the YouTube
//! Music tab already open at the song (no new tab, no focus); without jx the
//! song opens as a link. No cookies are read and no stream is fetched here.

use crate::events::{AppEvent, EventSink};
use crate::services::http::{Client, Method, MtaGuard};
use std::time::Duration;

const USER_AGENT: &str = concat!("OyenzaStudio/", env!("CARGO_PKG_VERSION"));
const SEARCH_URL: &str = "https://music.youtube.com/youtubei/v1/search?prettyPrint=false";
/// The songs filter of YouTube Music's search.
const SONGS: &str = "EgWKAQIIAWoKEAkQBRAKEAMQBA==";
const TIMEOUT: Duration = Duration::from_secs(8);
/// Results shown.
pub const MAX_RESULTS: usize = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Track {
    pub id: String,
    pub title: String,
    /// "Artist • Album • 3:33", as YouTube Music writes it.
    pub line: String,
}

/// Search for `query`; the answer comes as `AppEvent::MusicFound`.
pub fn search(query: String, sink: EventSink) {
    let spawned = std::thread::Builder::new().name("poni-ytmusic".into()).spawn({
        let (query, sink) = (query.clone(), sink.clone());
        move || {
            let got = ask(&query);
            if let Err(e) = &got {
                crate::pwarn!("music search: {e}");
            }
            sink.send(AppEvent::MusicFound { query, got });
        }
    });
    if let Err(e) = spawned {
        sink.send(AppEvent::MusicFound { query, got: Err(format!("no thread ({e})")) });
    }
}

fn ask(query: &str) -> Result<Vec<Track>, String> {
    let _com = MtaGuard::new();
    let client = Client::new(USER_AGENT).map_err(|e| e.to_string())?;
    let body = serde_json::json!({
        "context": {"client": {"clientName": "WEB_REMIX", "clientVersion": "1.20251020.01.00", "hl": "en", "gl": "ID"}},
        "query": query,
        "params": SONGS,
    })
    .to_string();
    let resp = client.send(Method::Post, SEARCH_URL, &[("Origin", "https://music.youtube.com")], Some(&body), TIMEOUT).map_err(|e| e.to_string())?;
    if resp.status != 200 {
        return Err(format!("HTTP {}", resp.status));
    }
    let v: serde_json::Value = serde_json::from_str(&resp.body).map_err(|e| e.to_string())?;
    Ok(parse(&v))
}

/// The songs in a search answer, in order.
pub fn parse(v: &serde_json::Value) -> Vec<Track> {
    let text = |col: &serde_json::Value| -> String {
        col["musicResponsiveListItemFlexColumnRenderer"]["text"]["runs"].as_array().map(|runs| runs.iter().filter_map(|r| r["text"].as_str()).collect()).unwrap_or_default()
    };
    let mut out = Vec::new();
    let tabs = &v["contents"]["tabbedSearchResultsRenderer"]["tabs"];
    let Some(sections) = tabs[0]["tabRenderer"]["content"]["sectionListRenderer"]["contents"].as_array() else { return out };
    for s in sections {
        let Some(items) = s["musicShelfRenderer"]["contents"].as_array() else { continue };
        for it in items {
            let r = &it["musicResponsiveListItemRenderer"];
            let Some(id) = r["playlistItemData"]["videoId"].as_str().filter(|id| valid_id(id)) else { continue };
            let cols = &r["flexColumns"];
            let title = text(&cols[0]);
            if title.is_empty() {
                continue;
            }
            out.push(Track { id: id.to_string(), title, line: text(&cols[1]) });
            if out.len() == MAX_RESULTS {
                return out;
            }
        }
    }
    out
}

pub fn valid_id(id: &str) -> bool {
    id.len() == 11 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Play `id` in the YouTube Music tab (through jx), else open it as a link.
pub fn play(id: String) {
    if !valid_id(&id) {
        return;
    }
    let spawned = std::thread::Builder::new().name("poni-ytmusic-play".into()).spawn(move || match through_jx(&id) {
        Ok(()) => crate::pinfo!("music search: playing in the YouTube Music tab"),
        Err(e) => {
            crate::pinfo!("music search: jx not there ({e}), opening the song");
            crate::shell::open(&format!("https://music.youtube.com/watch?v={id}"));
        }
    });
    if let Err(e) = spawned {
        crate::pwarn!("music search: no thread to play on ({e})");
    }
}

/// jx's daemon: one line of JSON on 127.0.0.1:9556 with its token, one back.
fn through_jx(id: &str) -> Result<(), String> {
    use std::io::{BufRead, BufReader, Write};
    let dir = std::env::var_os("LOCALAPPDATA").ok_or("no LOCALAPPDATA")?;
    let token = std::fs::read_to_string(std::path::Path::new(&dir).join("jx").join("token")).map_err(|_| "no jx token")?;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 9556));
    let mut s = std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(800)).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let msg = serde_json::json!({"token": token.trim(), "sid": "poni", "owner": null, "cpid": null, "steps": ["real", format!("ytm {id}")]});
    s.write_all(format!("{msg}\n").as_bytes()).map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line).map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
    if v["ok"].as_bool() == Some(true) {
        Ok(())
    } else {
        Err(v["steps"].as_array().and_then(|s| s.last()).and_then(|s| s["error"].as_str()).unwrap_or("jx said no").to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_songs_from_a_search_answer() {
        let col = |t: &str| serde_json::json!({"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{"text": t}]}}});
        let item = |id: &str, t: &str| serde_json::json!({"musicResponsiveListItemRenderer": {"playlistItemData": {"videoId": id}, "flexColumns": [col(t), col("Tulus • Manusia • 4:02")]}});
        let v = serde_json::json!({"contents": {"tabbedSearchResultsRenderer": {"tabs": [{"tabRenderer": {"content": {"sectionListRenderer": {"contents": [
            {"musicShelfRenderer": {"contents": [item("E7kHvjvU6JY", "Hati-Hati di Jalan"), item("bad", "Skipped")]}}
        ]}}}}]}}});
        let got = parse(&v);
        assert_eq!(got, vec![Track { id: "E7kHvjvU6JY".into(), title: "Hati-Hati di Jalan".into(), line: "Tulus • Manusia • 4:02".into() }]);
        assert!(parse(&serde_json::json!({})).is_empty());
    }
}
