// A one-shot HTTP server for adapter tests: it records the request it gets and
// answers with a canned response, so every wire format is exercised end to end
// without a key or the network.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;

#[derive(Debug)]
pub struct Captured {
    pub request_line: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Captured {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).expect("request body is JSON")
    }
}

/// Starts the server; returns its base URL and the channel the request arrives on.
pub fn serve_once(status: u16, body: &str) -> (String, mpsc::Receiver<Captured>) {
    serve_sequence(vec![(status, body.to_string())])
}

/// Answers one request per canned response, in order: an agent's tool loop.
pub fn serve_sequence(responses: Vec<(u16, String)>) -> (String, mpsc::Receiver<Captured>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for (status, body) in responses {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            let mut headers = Vec::new();
            let mut length = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let line = line.trim_end();
                if line.is_empty() {
                    break;
                }
                if let Some((k, v)) = line.split_once(':') {
                    if k.eq_ignore_ascii_case("content-length") {
                        length = v.trim().parse().unwrap_or(0);
                    }
                    headers.push((k.trim().to_string(), v.trim().to_string()));
                }
            }
            let mut buf = vec![0u8; length];
            reader.read_exact(&mut buf).unwrap();
            let _ = tx.send(Captured {
                request_line: request_line.trim_end().to_string(),
                headers,
                body: String::from_utf8_lossy(&buf).into_owned(),
            });
            let mut stream = stream;
            let reason = if status < 300 { "OK" } else { "Error" };
            write!(
                stream,
                "HTTP/1.1 {status} {reason}
content-type: application/json
content-length: {}
connection: close

{body}",
                body.len()
            )
            .unwrap();
        }
    });
    (base, rx)
}

pub fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tauri::async_runtime::block_on(f)
}

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap()
}
