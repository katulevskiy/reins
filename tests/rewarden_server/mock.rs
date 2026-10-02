//! A tiny HTTP/1.1 server standing in for GitHub, an asset host or an MCP server: it records every request and
//! answers with whatever the test's handler returns.

use std::sync::{Arc, Mutex};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

#[derive(Clone, Debug)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    /// Lower-cased names.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Recorded {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn new(status: u16, content_type: &str, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            headers: vec![("Content-Type".to_owned(), content_type.to_owned())],
            body: body.into(),
        }
    }

    pub fn json(status: u16, body: &serde_json::Value) -> Self {
        Self::new(status, "application/json", body.to_string())
    }

    pub fn redirect(status: u16, location: &str) -> Self {
        Self {
            status,
            headers: vec![("Location".to_owned(), location.to_owned())],
            body: Vec::new(),
        }
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
}

type Handler = dyn Fn(&Recorded) -> Reply + Send + Sync;

pub struct MockServer {
    pub port: u16,
    requests: Arc<Mutex<Vec<Recorded>>>,
}

impl MockServer {
    pub async fn start(handler: impl Fn(&Recorded) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind mock");
        let port = listener.local_addr().expect("mock addr").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let handler: Arc<Handler> = Arc::new(handler);
        let recorded = Arc::clone(&requests);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let handler = Arc::clone(&handler);
                let recorded = Arc::clone(&recorded);
                tokio::spawn(async move {
                    read_request(stream, &*handler, &recorded).await;
                });
            }
        });
        Self {
            port,
            requests,
        }
    }

    /// `http://127.0.0.1:<port><path>`
    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

async fn read_more(stream: &mut TcpStream, buf: &mut Vec<u8>) -> bool {
    let mut chunk = vec![0u8; 64 * 1024];
    match stream.read(&mut chunk).await {
        Ok(0) | Err(_) => false,
        Ok(n) => {
            buf.extend_from_slice(&chunk[..n]);
            true
        }
    }
}

/// Reads one request (Content-Length or chunked body), answers it and closes the connection.
async fn read_request(mut stream: TcpStream, handler: &Handler, recorded: &Mutex<Vec<Recorded>>) -> Option<()> {
    let mut buf = Vec::new();
    let head_end = loop {
        if let Some(at) = find(&buf, b"\r\n\r\n") {
            break at;
        }
        if !read_more(&mut stream, &mut buf).await {
            return None;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let (method, path) = (first.next()?.to_owned(), first.next()?.to_owned());
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    let mut rest = buf[head_end + 4..].to_vec();
    let get = |name: &str| headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
    let body = if let Some(length) = get("content-length").and_then(|v| v.parse::<usize>().ok()) {
        while rest.len() < length {
            if !read_more(&mut stream, &mut rest).await {
                return None;
            }
        }
        rest.truncate(length);
        rest
    } else if get("transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        let mut body = Vec::new();
        loop {
            let line_end = loop {
                if let Some(at) = find(&rest, b"\r\n") {
                    break at;
                }
                if !read_more(&mut stream, &mut rest).await {
                    return None;
                }
            };
            let size = usize::from_str_radix(String::from_utf8_lossy(&rest[..line_end]).trim(), 16).ok()?;
            rest.drain(..line_end + 2);
            if size == 0 {
                break;
            }
            while rest.len() < size + 2 {
                if !read_more(&mut stream, &mut rest).await {
                    return None;
                }
            }
            body.extend_from_slice(&rest[..size]);
            rest.drain(..size + 2);
        }
        body
    } else {
        Vec::new()
    };
    let request = Recorded {
        method,
        path,
        headers,
        body,
    };
    let reply = handler(&request);
    recorded.lock().unwrap().push(request);
    let extra = reply.headers.iter().map(|(k, v)| [k.as_str(), ": ", v.as_str(), "\r\n"].concat()).collect::<String>();
    let out = format!(
        "HTTP/1.1 {} Mock\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n",
        reply.status,
        reply.body.len()
    );
    stream.write_all(out.as_bytes()).await.ok()?;
    stream.write_all(&reply.body).await.ok()?;
    stream.shutdown().await.ok()?;
    Some(())
}
