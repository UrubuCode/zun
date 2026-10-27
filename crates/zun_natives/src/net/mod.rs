//! `net` group of `zun:core` — blocking HTTP.
//!
//! `httpFetch(method, url, reqHeaders, body)` performs the request
//! synchronously and returns the whole response as one blob:
//!
//! ```text
//! <status>\n<statusText>\n<Name: Value lines joined by \n>\n\n<body>
//! ```
//!
//! The body is everything after the FIRST `\n\n`. A transport failure (DNS,
//! connect, TLS, timeout) returns `__ZUN_FETCH_ERR__\n<message>`. A non-2xx
//! status is a normal response, not an error (matching the web `fetch`).
//!
//! Deferred: a `net` capability gate, true non-blocking I/O on the event loop.

use crate::{NativeArg, NativeError, NativeFn, NativeTy::*, NativeValue, Sink};
use std::io::Read;

/// First line of the blob when the request failed at the transport level.
const ERR_SENTINEL: &str = "__ZUN_FETCH_ERR__";

fn error_blob(message: &str) -> String {
    format!("{ERR_SENTINEL}\n{}", message.replace('\n', " "))
}

fn response_blob(response: ureq::Response) -> String {
    let status = response.status();
    let status_text = response.status_text().to_string();
    let mut head = String::new();
    for name in response.headers_names() {
        if let Some(value) = response.header(&name) {
            if !head.is_empty() {
                head.push('\n');
            }
            head.push_str(&name);
            head.push_str(": ");
            head.push_str(&value.replace('\n', " "));
        }
    }
    let mut bytes = Vec::new();
    if let Err(e) = response.into_reader().take(64 * 1024 * 1024).read_to_end(&mut bytes) {
        return error_blob(&format!("failed to read response body: {e}"));
    }
    let body = String::from_utf8_lossy(&bytes);
    format!("{status}\n{status_text}\n{head}\n\n{body}")
}

fn http_fetch(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let method = args[0].as_str().to_ascii_uppercase();
    let url = args[1].as_str();
    let headers = args[2].as_str();
    let body = args[3].as_str();

    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(60))
        .build();
    let mut request = agent.request(&method, url);
    for line in headers.split('\n') {
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim();
            if !name.is_empty() {
                request = request.set(name, value.trim());
            }
        }
    }
    let result = if body.is_empty() {
        request.call()
    } else {
        request.send_string(body)
    };
    let blob = match result {
        Ok(response) => response_blob(response),
        // A non-2xx status is still a response.
        Err(ureq::Error::Status(_, response)) => response_blob(response),
        Err(ureq::Error::Transport(t)) => error_blob(&t.to_string()),
    };
    Ok(NativeValue::Str(blob))
}

/// The `net` group of `zun:core`.
pub const MODULE: crate::Module = crate::Module {
    name: "net",
    functions: &[NativeFn {
        name: "httpFetch",
        params: &[Str, Str, Str, Str],
        ret: Str,
        imp: http_fetch,
    }],
};
