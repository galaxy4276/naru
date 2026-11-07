//! The OSC sequences Naru acts on, collected from a pane's output stream.

use serde::Serialize;
use vte::{Parser, Perform};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Osc {
    /// `OSC 777;notify;<title>;<body>`: agent plugin events and naru CLI requests, which name
    /// their channel in the title (`warp://cli-agent`, `naru://agent`, `naru://view-open`).
    Notify { title: String, body: String },
    /// `OSC 133;<mark>[;<exit code>]` from the shell integration.
    Prompt {
        mark: String,
        exit_code: Option<i32>,
    },
    /// `OSC 633;E;<command line>`: what the shell is about to run, as typed.
    Command { line: String },
    /// `OSC 7;file://<host><path>`.
    Cwd { path: String },
    /// `OSC 0` or `OSC 2`.
    Title { title: String },
}

#[derive(Default)]
pub struct OscCollector {
    parser: Parser,
    found: Found,
}

impl OscCollector {
    /// Feeds raw output; sequences split across calls are completed by a later call.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Osc> {
        self.parser.advance(&mut self.found, bytes);
        std::mem::take(&mut self.found.0)
    }
}

#[derive(Default)]
struct Found(Vec<Osc>);

impl Perform for Found {
    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        // vte splits at every ';', so a value that may hold one is joined back together.
        let joined = |rest: &[&[u8]]| String::from_utf8_lossy(&rest.join(&b';')).into_owned();
        let osc = match params {
            [b"777", b"notify", title, body @ ..] => Osc::Notify {
                title: String::from_utf8_lossy(title).into_owned(),
                body: joined(body),
            },
            [b"133", mark, rest @ ..] => Osc::Prompt {
                mark: String::from_utf8_lossy(mark).into_owned(),
                exit_code: rest
                    .first()
                    .and_then(|code| std::str::from_utf8(code).ok()?.parse().ok()),
            },
            [b"633", b"E", rest @ ..] => Osc::Command { line: joined(rest) },
            [b"7", rest @ ..] => match file_url_path(&joined(rest)) {
                Some(path) => Osc::Cwd { path },
                None => return,
            },
            [b"0" | b"2", rest @ ..] => Osc::Title {
                title: joined(rest),
            },
            _ => return,
        };
        self.0.push(osc);
    }
}

fn file_url_path(url: &str) -> Option<String> {
    let rest = url.strip_prefix("file://")?;
    let path = &rest[rest.find('/')?..];
    percent_decode(path)
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
#[path = "osc_tests.rs"]
mod tests;
