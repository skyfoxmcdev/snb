use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct RecordedRequest {
    pub path: String,
    pub headers: HashMap<String, String>,
}

#[derive(Clone)]
struct Route {
    status: u16,
    body: Vec<u8>,
    content_type: &'static str,
}

pub struct TestServer {
    address: SocketAddr,
    routes: Arc<RwLock<HashMap<String, Route>>>,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    shutdown: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl TestServer {
    pub fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let routes: Arc<RwLock<HashMap<String, Route>>> =
            Arc::new(RwLock::new(HashMap::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let shutdown = Arc::new(AtomicBool::new(false));

        let routes_thread = routes.clone();
        let requests_thread = requests.clone();
        let shutdown_thread = shutdown.clone();
        let thread = thread::spawn(move || {
            while !shutdown_thread.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let routes = routes_thread.clone();
                        let requests = requests_thread.clone();
                        thread::spawn(move || handle_connection(stream, routes, requests));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(4));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            address,
            routes,
            requests,
            shutdown,
            thread: Some(thread),
        }
    }

    pub fn set(&self, path: &str, body: impl Into<Vec<u8>>) {
        self.set_with_status(path, 200, body);
    }

    pub fn set_with_status(&self, path: &str, status: u16, body: impl Into<Vec<u8>>) {
        self.routes.write().unwrap().insert(
            path.to_string(),
            Route {
                status,
                body: body.into(),
                content_type: "application/json",
            },
        );
    }

    pub fn set_content_type(&self, path: &str, content_type: &'static str) {
        if let Some(route) = self.routes.write().unwrap().get_mut(path) {
            route.content_type = content_type;
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.address, path)
    }

    pub fn base_url(&self) -> String {
        format!("http://{}/", self.address)
    }

    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(self.address);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

fn handle_connection(
    mut stream: TcpStream,
    routes: Arc<RwLock<HashMap<String, Route>>>,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
    let mut raw = Vec::new();
    let mut buffer = [0u8; 4096];
    while raw.len() < 32 * 1024 {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                raw.extend_from_slice(&buffer[..read]);
                if raw.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            Err(_) => return,
        }
    }
    let text = String::from_utf8_lossy(&raw);
    let mut lines = text.lines();
    let Some(request_line) = lines.next() else {
        return;
    };
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("GET");
    let path = parts.next().unwrap_or("/").to_string();
    if !path.starts_with('/') || method != "GET" {
        return;
    }
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((key, value)) = line.split_once(':') {
            headers.insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    requests.lock().unwrap().push(RecordedRequest {
        path: path.clone(),
        headers: headers.clone(),
    });

    let route = routes.read().unwrap().get(&path).cloned().unwrap_or(Route {
        status: 404,
        body: b"not found".to_vec(),
        content_type: "text/plain",
    });

    let range_start = headers
        .get("range")
        .and_then(|value| value.strip_prefix("bytes="))
        .and_then(|value| value.split('-').next())
        .and_then(|value| value.parse::<usize>().ok());

    let (status, body, content_range) = match range_start {
        Some(start) if start < route.body.len() => (
            206,
            route.body[start..].to_vec(),
            Some(format!(
                "bytes {start}-{}/{}",
                route.body.len().saturating_sub(1),
                route.body.len()
            )),
        ),
        Some(_) => (416, Vec::new(), None),
        None => (route.status, route.body.clone(), None),
    };

    let mut response = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        status,
        if status == 404 { "Not Found" } else { "OK" },
        route.content_type,
        body.len()
    );
    if let Some(range) = content_range {
        response.push_str(&format!("Content-Range: {range}\r\n"));
    }
    response.push_str("\r\n");
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.write_all(&body);
    let _ = stream.flush();
}
