//! A translation line under each lyric line, for people learning the song's
//! language: what the line means in theirs (`Settings::translate_to`).
//!
//! The words come from Google Translate's public endpoint (`client=gtx`, no
//! key). A song goes in a few requests: its distinct lines (a chorus once),
//! newline-joined, cut into batches that keep the URL short. Each answer is
//! split back on its newlines; a batch whose line count doesn't come back
//! the same is asked again a line at a time. A song already in the target
//! language gets no lines at all.
//!
//! Everything here is pure; the worker (`services::translate`) does the
//! fetching and caching.

use crate::urlenc;
use serde_json::Value;

const ENDPOINT: &str = "https://translate.googleapis.com/translate_a/single";
/// Characters of lyrics per request: CJK encodes to 9 URL bytes each, so
/// this keeps a request well under the endpoint's URL limit.
pub const BATCH_CHARS: usize = 600;
/// Requests one song may cost at most (batches plus line-by-line retries).
pub const MAX_REQUESTS: usize = 40;

/// The request URL that translates `text` into `to` (an ISO 639-1 code such
/// as "id", "en", "zh-CN"), the source language detected.
pub fn url(text: &str, to: &str) -> String {
    urlenc::with_query(ENDPOINT, &[("client", "gtx"), ("sl", "auto"), ("tl", to), ("dt", "t"), ("q", text)])
}

/// The translated text and the detected source language from an answer:
/// `[[["translated", "original", ...], ...], null, "ja", ...]`.
pub fn parse(body: &str) -> Option<(String, String)> {
    let v: Value = serde_json::from_str(body).ok()?;
    let segments = v.get(0)?.as_array()?;
    let mut out = String::new();
    for s in segments {
        if let Some(t) = s.get(0).and_then(Value::as_str) {
            out.push_str(t);
        }
    }
    let lang = v.get(2).and_then(Value::as_str).unwrap_or_default().to_string();
    Some((out, lang))
}

/// A line worth translating: it has letters (not "...", "♪" or "(x2)" alone).
pub fn worth(line: &str) -> bool {
    line.chars().any(char::is_alphabetic)
}

/// The same language, ignoring region ("zh-CN" = "zh", "id" = "in").
pub fn same_language(a: &str, b: &str) -> bool {
    let base = |s: &str| {
        let s = s.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
        if s == "in" { "id".to_string() } else { s }
    };
    !a.is_empty() && base(a) == base(b)
}

/// Distinct lines, cut into batches of at most `max` characters (a longer
/// line goes alone).
pub fn batches(lines: &[String], max: usize) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut size = 0;
    for l in lines {
        let n = l.chars().count() + 1;
        match out.last_mut() {
            Some(b) if size + n <= max => {
                b.push(l.clone());
                size += n;
            }
            _ => {
                out.push(vec![l.clone()]);
                size = n;
            }
        }
    }
    out
}

/// Every lyric line's translation into `to` ("" where there is none), with
/// `fetch` answering one request URL with the response body. Err only when
/// nothing could be fetched at all; a song already in `to` is Ok with no lines.
pub fn translate_song(lines: &[&str], to: &str, mut fetch: impl FnMut(&str) -> Result<String, String>) -> Result<Vec<String>, String> {
    let mut distinct: Vec<String> = Vec::new();
    for l in lines.iter().map(|l| l.trim()).filter(|l| worth(l)) {
        if !distinct.iter().any(|d| d == l) {
            distinct.push(l.to_string());
        }
    }
    let mut known: Vec<(String, String)> = Vec::new();
    let mut requests = 0;
    let mut ask = |text: &str, requests: &mut usize| -> Result<(String, String), String> {
        *requests += 1;
        let body = fetch(&url(text, to))?;
        parse(&body).ok_or_else(|| "translation: an answer that doesn't parse".to_string())
    };
    for batch in batches(&distinct, BATCH_CHARS) {
        if requests >= MAX_REQUESTS {
            break;
        }
        let (text, lang) = match ask(&batch.join("\n"), &mut requests) {
            Ok(v) => v,
            Err(e) if known.is_empty() => return Err(e),
            Err(_) => continue,
        };
        if same_language(&lang, to) {
            // The song is in the reader's own language: nothing to show.
            return Ok(vec![String::new(); lines.len()]);
        }
        let back: Vec<&str> = text.split('\n').collect();
        if back.len() == batch.len() {
            known.extend(batch.into_iter().zip(back.into_iter().map(|t| t.trim().to_string())));
            continue;
        }
        // Lines merged or split on the way: ask again one line at a time.
        for line in batch {
            if requests >= MAX_REQUESTS {
                break;
            }
            if let Ok((t, _)) = ask(&line, &mut requests) {
                known.push((line, t.trim().to_string()));
            }
        }
    }
    Ok(lines
        .iter()
        .map(|l| {
            let l = l.trim();
            match known.iter().find(|(o, _)| o == l) {
                // A line that comes back as itself (a name, "la la la") adds nothing.
                Some((o, t)) if !t.is_empty() && !t.eq_ignore_ascii_case(o) => t.clone(),
                _ => String::new(),
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(text: &str, lang: &str) -> String {
        serde_json::json!([[[text, "orig", null, null, 3]], null, lang]).to_string()
    }

    #[test]
    fn parses_the_segments_and_the_language() {
        let body = r#"[[["aku cinta kamu\n","我爱你\n",null,null,3],["malam itu","夜",null,null,3]],null,"zh-CN",null]"#;
        assert_eq!(parse(body), Some(("aku cinta kamu\nmalam itu".to_string(), "zh-CN".to_string())));
        assert_eq!(parse("not json"), None);
    }

    #[test]
    fn url_carries_target_and_encoded_text() {
        let u = url("我爱你", "id");
        assert!(u.starts_with("https://translate.googleapis.com/translate_a/single?"));
        assert!(u.contains("tl=id") && u.contains("client=gtx") && u.contains("q=%E6%88%91"));
    }

    #[test]
    fn languages_match_without_region() {
        assert!(same_language("zh-CN", "zh"));
        assert!(same_language("in", "id"));
        assert!(!same_language("ja", "id"));
        assert!(!same_language("", "id"));
    }

    #[test]
    fn batches_stay_small_and_keep_order() {
        let lines: Vec<String> = ["aaaa", "bbbb", "cccc"].iter().map(|s| s.to_string()).collect();
        assert_eq!(batches(&lines, 10), vec![vec!["aaaa".to_string(), "bbbb".to_string()], vec!["cccc".to_string()]]);
        assert_eq!(batches(&lines, 3).len(), 3);
    }

    #[test]
    fn a_song_translates_line_for_line_with_its_chorus_once() {
        let lines = ["我爱你", "", "Te quiero", "我爱你", "♪"];
        let mut asked = Vec::new();
        let out = translate_song(&lines, "id", |u| {
            asked.push(u.to_string());
            Ok(answer("aku cinta kamu\naku mencintaimu", "zh-CN"))
        })
        .unwrap();
        assert_eq!(asked.len(), 1, "one batch, the chorus once");
        assert_eq!(out, vec!["aku cinta kamu", "", "aku mencintaimu", "aku cinta kamu", ""]);
    }

    #[test]
    fn a_song_in_the_readers_language_gets_nothing() {
        let out = translate_song(&["aku cinta kamu"], "id", |_| Ok(answer("aku cinta kamu", "id"))).unwrap();
        assert_eq!(out, vec![String::new()]);
    }

    #[test]
    fn a_batch_that_comes_back_merged_is_asked_line_by_line() {
        let mut n = 0;
        let out = translate_song(&["uno", "dos"], "id", |_| {
            n += 1;
            Ok(match n {
                1 => answer("satu dua", "es"),
                2 => answer("satu", "es"),
                _ => answer("dua", "es"),
            })
        })
        .unwrap();
        assert_eq!(out, vec!["satu", "dua"]);
    }

    #[test]
    fn offline_is_an_error_and_names_stay_bare() {
        assert!(translate_song(&["hola"], "id", |_| Err("offline".into())).is_err());
        let out = translate_song(&["Yoasobi"], "id", |_| Ok(answer("Yoasobi", "ja"))).unwrap();
        assert_eq!(out, vec![String::new()]);
    }
}
