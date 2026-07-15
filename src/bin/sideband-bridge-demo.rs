//! Loopback demo bridge connector.
//!
//! A tiny, dependency-light sidecar that speaks Sideband's bridge JSON-lines
//! protocol (see `src/bridge.rs`) with no external network. It proves the whole
//! bridge pipeline — spawn, handshake, conversation list, inbound + outbound
//! messages, delivery results — end to end, with no accounts or infrastructure.
//!
//! Behaviour:
//!   * On `hello`: report `ready` + `connected`, and emit two seed
//!     conversations ("Demo Bot" and "Echo Room").
//!   * On `send`: echo the text straight back as an inbound message from the
//!     same conversation, then report a successful `send_result`.
//!   * On `login`: no-op (already connected).
//!   * On `shutdown` or stdin EOF: exit.
//!
//! Kept deliberately free of the Sideband crate so it stays a faithful stand-in
//! for a real out-of-process connector (e.g. the future Matrix connector).

use std::io::{BufRead, Write};

fn emit(line: &str) {
    let stdout = std::io::stdout();
    let mut h = stdout.lock();
    let _ = writeln!(h, "{line}");
    let _ = h.flush();
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Bare-bones extraction of a string field's value from a flat JSON object line.
/// The protocol lines are small and machine-generated, so this avoids pulling a
/// JSON crate into the demo binary.
fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\"");
    let idx = line.find(&needle)? + needle.len();
    let rest = &line[idx..];
    let colon = rest.find(':')? + 1;
    let rest = rest[colon..].trim_start();
    let rest = rest.strip_prefix('"')?;
    // Find the closing quote, honoring simple backslash escapes.
    let mut end = 0;
    let bytes = rest.as_bytes();
    while end < bytes.len() {
        match bytes[end] {
            b'\\' => end += 2,
            b'"' => return Some(&rest[..end]),
            _ => end += 1,
        }
    }
    None
}

fn field_i64(line: &str, key: &str) -> Option<i64> {
    let needle = format!("\"{key}\"");
    let idx = line.find(&needle)? + needle.len();
    let rest = &line[idx..];
    let colon = rest.find(':')? + 1;
    let rest = rest[colon..].trim_start();
    let end = rest
        .find(|c: char| !c.is_ascii_digit() && c != '-')
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn seed_conversation(remote_id: &str, title: &str, kind: &str) {
    emit(&format!(
        "{{\"type\":\"conversation\",\"remote_id\":\"{}\",\"title\":\"{}\",\"kind\":\"{}\",\"last_activity_ms\":{}}}",
        json_escape(remote_id),
        json_escape(title),
        json_escape(kind),
        now_ms()
    ));
}

fn main() {
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let msg_type = field(line, "type").unwrap_or("");
        match msg_type {
            "hello" => {
                emit("{\"type\":\"ready\"}");
                emit("{\"type\":\"status\",\"state\":\"connected\"}");
                seed_conversation("demo-bot", "Demo Bot", "dm");
                seed_conversation("echo-room", "Echo Room", "group");
                emit(&format!(
                    "{{\"type\":\"message\",\"remote_id\":\"demo-bot\",\"sender\":\"Demo Bot\",\"text\":\"{}\",\"timestamp_ms\":{}}}",
                    json_escape("Hi! I'm a demo bridge. Send me anything and I'll echo it back."),
                    now_ms()
                ));
            }
            "send" => {
                let outbox_id = field_i64(line, "outbox_id").unwrap_or(0);
                let remote_id = field(line, "remote_id").map(unescape).unwrap_or_default();
                let text = field(line, "text").map(unescape).unwrap_or_default();
                // Echo the message straight back as an inbound reply.
                emit(&format!(
                    "{{\"type\":\"message\",\"remote_id\":\"{}\",\"sender\":\"Echo\",\"text\":\"{}\",\"timestamp_ms\":{}}}",
                    json_escape(&remote_id),
                    json_escape(&format!("echo: {text}")),
                    now_ms()
                ));
                emit(&format!(
                    "{{\"type\":\"send_result\",\"outbox_id\":{outbox_id},\"ok\":true}}"
                ));
            }
            "login" => {
                emit("{\"type\":\"status\",\"state\":\"connected\"}");
            }
            "shutdown" => break,
            _ => {}
        }
    }
}
