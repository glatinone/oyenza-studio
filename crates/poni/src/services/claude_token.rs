//! Keeps a Claude Code sign-in fresh for the AI tab.
//!
//! Claude Code's access token lasts about eight hours and only a running
//! Claude Code renews it. A config folder nobody has opened today (a second
//! account) would read "sign-in expired" until then. When the token in a
//! `.credentials.json` has run out, this renews it the way Claude Code does
//! (refresh token, its own client id and scopes) and writes the new tokens
//! back into the same file, so Claude Code picks them up instead of renewing
//! again. Only an expired token is renewed: a Claude Code that is running
//! renews minutes before expiry, so it never meets this one half-way, and it
//! re-reads the file under its own lock before it renews anyway.

use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use studio_usage::{NetFail, Reply, Request};

const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";

/// Refresh tokens the server turned down, per file: never sent again (the
/// person signs in again, which brings a new one).
#[derive(Default)]
pub struct Renewer {
    dead: BTreeMap<PathBuf, String>,
}

impl Renewer {
    /// Renews the sign-in in `path` when its access token has run out.
    /// True when the file now holds a fresh token.
    pub fn renew(&mut self, path: &Path, now_s: i64, fetch: impl FnOnce(&Request) -> Result<Reply, NetFail>) -> bool {
        let Some((doc, refresh, scopes)) = expired(path, now_s) else { return false };
        if self.dead.get(path) == Some(&refresh) {
            return false;
        }
        let body = serde_json::json!({ "grant_type": "refresh_token", "refresh_token": refresh, "client_id": CLIENT_ID, "scope": scopes });
        let reply = match fetch(&Request::post_json(TOKEN_URL, body.to_string())) {
            Ok(r) => r,
            Err(_) => return false,
        };
        if reply.status == 400 || reply.status == 401 {
            crate::pwarn!("usage: claude: the refresh token was turned down; sign in again");
            self.dead.insert(path.to_path_buf(), refresh);
            return false;
        }
        let Some(new) = (reply.status == 200).then(|| serde_json::from_str::<Value>(&reply.body).ok()).flatten() else {
            crate::pwarn!("usage: claude: renewing the sign-in failed (HTTP {})", reply.status);
            return false;
        };
        // Someone (Claude Code) renewed it meanwhile: theirs stays.
        if expired(path, now_s).map(|(_, r, _)| r) != Some(refresh.clone()) {
            return false;
        }
        match save(path, doc, &new, now_s) {
            Ok(()) => {
                crate::pinfo!("usage: claude: sign-in renewed ({})", path.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                true
            }
            Err(e) => {
                crate::pwarn!("usage: claude: couldn't save the renewed sign-in: {e}");
                false
            }
        }
    }
}

/// The file's JSON, refresh token and scopes, when its access token has run
/// out (or does within a minute) and it can be renewed.
fn expired(path: &Path, now_s: i64) -> Option<(Value, String, String)> {
    let doc: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let o = doc.get("claudeAiOauth")?;
    let exp_ms = o.get("expiresAt")?.as_i64()?;
    if exp_ms / 1000 > now_s + 60 {
        return None;
    }
    if o.get("refreshTokenExpiresAt").and_then(Value::as_i64).is_some_and(|t| t / 1000 <= now_s) {
        return None;
    }
    let refresh = o.get("refreshToken")?.as_str()?.trim().to_string();
    let scopes = o.get("scopes").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ")).filter(|s| !s.is_empty())?;
    (!refresh.is_empty()).then_some((doc, refresh, scopes))
}

/// The new tokens into the sign-in, everything else kept, the file replaced
/// in one move.
fn save(path: &Path, mut doc: Value, new: &Value, now_s: i64) -> std::io::Result<()> {
    let bad = || std::io::Error::other("unexpected answer");
    let access = new.get("access_token").and_then(Value::as_str).ok_or_else(bad)?;
    let lasts = new.get("expires_in").and_then(Value::as_i64).ok_or_else(bad)?;
    let o = doc.get_mut("claudeAiOauth").and_then(Value::as_object_mut).ok_or_else(bad)?;
    o.insert("accessToken".into(), access.into());
    o.insert("expiresAt".into(), ((now_s + lasts) * 1000).into());
    if let Some(r) = new.get("refresh_token").and_then(Value::as_str) {
        o.insert("refreshToken".into(), r.into());
    }
    if let Some(r) = new.get("refresh_token_expires_in").and_then(Value::as_i64) {
        o.insert("refreshTokenExpiresAt".into(), ((now_s + r) * 1000).into());
    }
    if let Some(s) = new.get("scope").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) {
        o.insert("scopes".into(), s.split_whitespace().collect::<Vec<_>>().into());
    }
    let tmp = path.with_extension("json.poni-tmp");
    std::fs::write(&tmp, serde_json::to_vec(&doc)?)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn file(name: &str, exp_s: i64) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("poni-claude-token-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".credentials.json");
        let doc = serde_json::json!({
            "claudeAiOauth": {"accessToken": "old", "refreshToken": "r1", "expiresAt": exp_s * 1000, "refreshTokenExpiresAt": (NOW + 86_400) * 1000,
                "scopes": ["user:inference", "user:profile"], "subscriptionType": "max", "rateLimitTier": "default_claude_max_20x"},
            "mcpOAuth": {"x": 1}
        });
        std::fs::write(&path, doc.to_string()).unwrap();
        path
    }

    fn ok(body: &str) -> Result<Reply, NetFail> {
        Ok(Reply { status: 200, body: body.into(), retry_after_s: None })
    }

    #[test]
    fn a_good_token_is_left_alone() {
        let path = file("good", NOW + 3600);
        let mut asked = false;
        assert!(!Renewer::default().renew(&path, NOW, |_| {
            asked = true;
            ok("{}")
        }));
        assert!(!asked);
    }

    #[test]
    fn an_expired_token_is_renewed_and_the_rest_kept() {
        let path = file("expired", NOW - 10);
        let mut sent = String::new();
        assert!(Renewer::default().renew(&path, NOW, |r| {
            sent = r.json.clone().unwrap();
            ok(r#"{"access_token":"new","refresh_token":"r2","expires_in":28800,"scope":"user:inference user:profile"}"#)
        }));
        let sent: Value = serde_json::from_str(&sent).unwrap();
        assert_eq!(sent["refresh_token"], "r1");
        assert_eq!(sent["client_id"], CLIENT_ID);
        assert_eq!(sent["scope"], "user:inference user:profile");
        let doc: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let o = &doc["claudeAiOauth"];
        assert_eq!(o["accessToken"], "new");
        assert_eq!(o["refreshToken"], "r2");
        assert_eq!(o["expiresAt"], (NOW + 28800) * 1000);
        assert_eq!(o["subscriptionType"], "max");
        assert_eq!(doc["mcpOAuth"]["x"], 1);
    }

    #[test]
    fn a_turned_down_refresh_token_is_not_sent_again() {
        let path = file("dead", NOW - 10);
        let mut r = Renewer::default();
        assert!(!r.renew(&path, NOW, |_| Ok(Reply { status: 400, body: "{}".into(), retry_after_s: None })));
        let mut asked = false;
        assert!(!r.renew(&path, NOW, |_| {
            asked = true;
            ok("{}")
        }));
        assert!(!asked);
    }

    /// Against the real server, on the file named by PONI_LIVE_CREDENTIALS:
    /// marks its token run out, then renews it. Run by hand only.
    #[test]
    #[ignore]
    fn live_renew() {
        use crate::services::http::{Client, Method};
        let path = PathBuf::from(std::env::var("PONI_LIVE_CREDENTIALS").unwrap());
        let text = std::fs::read_to_string(&path).unwrap();
        let mut doc: Value = serde_json::from_str(&text).unwrap();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        doc["claudeAiOauth"]["expiresAt"] = ((now - 5) * 1000).into();
        std::fs::write(&path, doc.to_string()).unwrap();
        let client = Client::private("OyenzaStudio-test").unwrap();
        let done = Renewer::default().renew(&path, now, |r| {
            client.send(Method::Post, &r.url, &studio_usage::pace::send_headers(r), r.json.as_deref(), std::time::Duration::from_secs(15))
                .map(|x| Reply { status: x.status, body: x.body, retry_after_s: None })
                .map_err(|e| NetFail::Other(format!("{e:?}")))
        });
        if !done {
            std::fs::write(&path, text).unwrap();
        }
        assert!(done, "renew failed; file restored");
    }
}