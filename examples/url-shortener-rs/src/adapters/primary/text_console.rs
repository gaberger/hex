//! One line of text in, one line of text out.

use crate::ports::{Console, Shortener};

/// A tiny command reader.
///
/// | Input line        | Output                        |
/// |-------------------|-------------------------------|
/// | `shorten <url>`   | `ok <code>` or `err <reason>` |
/// | `resolve <code>`  | `ok <url>`, `none`, or `err`  |
/// | `expire`          | `ok <count>`                  |
/// | anything else     | `err unknown command`         |
///
/// It must never panic on any input, so every branch here is total.
pub struct TextConsole<S: Shortener> {
    app: S,
}

impl<S: Shortener> TextConsole<S> {
    pub fn new(app: S) -> TextConsole<S> {
        TextConsole { app }
    }
}

impl<S: Shortener> Console for TextConsole<S> {
    fn handle(&self, line: &str) -> String {
        let mut parts = line.trim().splitn(2, char::is_whitespace);
        let verb = parts.next().unwrap_or("");
        let argument = parts.next().unwrap_or("").trim();
        match verb {
            "shorten" => self.shorten(argument),
            "resolve" => self.resolve(argument),
            "expire" => format!("ok {}", self.app.expire()),
            _ => "err unknown command".to_string(),
        }
    }
}

impl<S: Shortener> TextConsole<S> {
    fn shorten(&self, argument: &str) -> String {
        if argument.is_empty() {
            return "err missing argument".to_string();
        }
        match self.app.shorten(argument) {
            Ok(done) => format!("ok {}", done.code.as_str()),
            Err(why) => format!("err {why}"),
        }
    }

    fn resolve(&self, argument: &str) -> String {
        if argument.is_empty() {
            return "err missing argument".to_string();
        }
        match self.app.resolve(argument) {
            Ok(Some(url)) => format!("ok {}", url.as_str()),
            Ok(None) => "none".to_string(),
            Err(why) => format!("err {why}"),
        }
    }
}
