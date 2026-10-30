//! A tiny HTTP/1.1 file server on 127.0.0.1 for download tests. Supports `Range: bytes=N-`,
//! redirects, and dropping the connection mid-body to simulate a network failure.

#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use sha2::{Digest, Sha256};
use wf_models::{ModelEntry, ModelFile};

#[derive(Clone, Default)]
pub struct Route {
    pub body: Vec<u8>,
    /// Answer range requests with 200 and the whole body, like some CDNs do.
    pub ignore_range: bool,
    /// On the first request only, send this many body bytes and then close the socket.
    pub cut_first_after: Option<usize>,
    /// Answer with a 302 to this path.
    pub redirect_to: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub path: String,
    pub range: Option<String>,
}

#[derive(Default)]
struct State {
    routes: HashMap<String, Route>,
    requests: Vec<Request>,
}

pub struct TestServer {
    pub base: String,
    state: Arc<Mutex<State>>,
}

impl TestServer {
    pub fn start() -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let state = Arc::new(Mutex::new(State::default()));
        let shared = state.clone();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let state = shared.clone();
                thread::spawn(move || serve(stream, &state));
            }
        });
        TestServer { base, state }
    }

    pub fn route(&self, path: &str, route: Route) -> String {
        self.state
            .lock()
            .unwrap()
            .routes
            .insert(path.to_owned(), route);
        format!("{}{path}", self.base)
    }

    /// Serves `body` at `path` and returns the catalog entry describing it.
    pub fn file(&self, path: &str, body: &[u8]) -> ModelFile {
        let route = Route {
            body: body.to_vec(),
            ..Route::default()
        };
        let url = self.route(path, route);
        ModelFile {
            name: path.rsplit('/').next().unwrap().to_owned(),
            url,
            size: body.len() as u64,
            sha256: sha256_hex(body),
        }
    }

    pub fn requests(&self) -> Vec<Request> {
        self.state.lock().unwrap().requests.clone()
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Deterministic, non-repeating-looking bytes.
pub fn payload(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 31 % 251) as u8).collect()
}

pub fn model(id: &str, files: Vec<ModelFile>) -> ModelEntry {
    ModelEntry {
        id: id.to_owned(),
        name: id.to_owned(),
        kind: wf_models::ModelKind::Asr,
        engine: wf_models::Engine::WhisperCpp,
        license: "MIT".into(),
        languages: vec!["*".into()],
        ram_mb: 1,
        candidate: true,
        notes: None,
        files,
    }
}

fn serve(stream: TcpStream, state: &Mutex<State>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
    let mut range = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("range")
        {
            range = Some(value.trim().to_owned());
        }
    }

    let (route, first) = {
        let mut state = state.lock().unwrap();
        let first = !state.requests.iter().any(|r| r.path == path);
        state.requests.push(Request {
            path: path.clone(),
            range: range.clone(),
        });
        (state.routes.get(&path).cloned(), first)
    };
    let mut out = stream;
    let Some(route) = route else {
        let _ = out
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return;
    };
    if let Some(to) = route.redirect_to {
        let _ = write!(
            out,
            "HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        return;
    }

    let total = route.body.len();
    let start = range.filter(|_| !route.ignore_range).and_then(|r| {
        r.strip_prefix("bytes=")?
            .strip_suffix('-')?
            .parse::<usize>()
            .ok()
    });
    let head = match start {
        Some(start) if start >= total => {
            let _ = write!(
                out,
                "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{total}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            return;
        }
        Some(start) => format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{}/{total}\r\nContent-Length: {}\r\n",
            total - 1,
            total - start
        ),
        None => format!("HTTP/1.1 200 OK\r\nContent-Length: {total}\r\n"),
    };
    let body = &route.body[start.unwrap_or(0)..];
    let _ = write!(
        out,
        "{head}Accept-Ranges: bytes\r\nConnection: close\r\n\r\n"
    );
    match route.cut_first_after.filter(|_| first) {
        Some(n) => {
            let _ = out.write_all(&body[..n.min(body.len())]);
            let _ = out.flush();
            let _ = out.shutdown(std::net::Shutdown::Both);
        }
        None => {
            let _ = out.write_all(body);
        }
    }
}
