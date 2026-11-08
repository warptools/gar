//! A deliberately dumb static-file HTTP server, standing in for "any static webserver" in tests.
//!
//! It serves the files under one directory on a loopback port picked by the OS:
//! GET only, HTTP/1.1, every response sized by `Content-Length`,
//! connections kept alive until the client closes them or asks for `Connection: close`.
//! 200 for a regular file, 404 for nothing there, 500 for anything else (a directory, an unreadable file).
//!
//! Std-only and free of crate imports, so tests in other crates can pull it in with `#[path]`.

use std::fs;
use std::io::{self, BufRead as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

/// A running server.  Stops accepting when dropped;
/// connections already open are served until their client closes them.
pub struct StaticServer {
	addr: SocketAddr,
	stop: Arc<AtomicBool>,
	acceptor: Option<thread::JoinHandle<()>>,
}

impl StaticServer {
	/// Starts serving `root`, with each connection handled on its own thread.
	pub fn serve(root: &Path) -> io::Result<StaticServer> {
		let listener = TcpListener::bind("127.0.0.1:0")?;
		let addr = listener.local_addr()?;
		let stop = Arc::new(AtomicBool::new(false));
		let root = root.to_owned();
		let acceptor = {
			let stop = stop.clone();
			thread::spawn(move || accept_loop(listener, root, stop))
		};
		Ok(StaticServer {
			addr,
			stop,
			acceptor: Some(acceptor),
		})
	}

	/// The server's base URL, with no trailing slash: `http://127.0.0.1:{port}`.
	pub fn url(&self) -> String {
		format!("http://{}", self.addr)
	}
}

impl Drop for StaticServer {
	/// The acceptor is blocked in `accept`, so a connection of our own is what wakes it to see the stop flag.
	/// If even that connect fails, the acceptor is left behind rather than joined forever.
	fn drop(&mut self) {
		self.stop.store(true, Ordering::SeqCst);
		if TcpStream::connect(self.addr).is_ok() {
			if let Some(acceptor) = self.acceptor.take() {
				let _ = acceptor.join();
			}
		}
	}
}

fn accept_loop(listener: TcpListener, root: PathBuf, stop: Arc<AtomicBool>) {
	for conn in listener.incoming() {
		if stop.load(Ordering::SeqCst) {
			return;
		}
		let Ok(conn) = conn else {
			continue;
		};
		let root = root.clone();
		// Connection errors (a client hanging up mid-response, say) end that connection and nothing else.
		thread::spawn(move || serve_connection(conn, &root));
	}
}

fn serve_connection(stream: TcpStream, root: &Path) -> io::Result<()> {
	let mut reader = io::BufReader::new(stream.try_clone()?);
	let mut writer = stream;
	loop {
		let mut request_line = String::new();
		if reader.read_line(&mut request_line)? == 0 {
			return Ok(()); // The client closed between requests.
		}
		let mut close = false;
		loop {
			let mut header = String::new();
			if reader.read_line(&mut header)? == 0 {
				return Ok(());
			}
			let header = header.trim_end().to_ascii_lowercase();
			if header.is_empty() {
				break;
			}
			if header.starts_with("connection:") && header.contains("close") {
				close = true;
			}
		}
		respond(&mut writer, root, &request_line, close)?;
		if close {
			return Ok(());
		}
	}
}

fn respond(w: &mut TcpStream, root: &Path, request_line: &str, close: bool) -> io::Result<()> {
	let mut parts = request_line.split_whitespace();
	let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
		return respond_status(w, 400, "Bad Request", close);
	};
	if method != "GET" {
		return respond_status(w, 405, "Method Not Allowed", close);
	}
	let Some(rel) = target.strip_prefix('/') else {
		return respond_status(w, 400, "Bad Request", close);
	};
	if rel.split('/').any(|seg| seg == "..") {
		return respond_status(w, 400, "Bad Request", close);
	}
	let path = root.join(rel);
	let (mut file, len) = match fs::metadata(&path) {
		Ok(meta) if meta.is_file() => match fs::File::open(&path) {
			Ok(file) => (file, meta.len()),
			Err(_) => return respond_status(w, 500, "Internal Server Error", close),
		},
		Err(e) if e.kind() == io::ErrorKind::NotFound => {
			return respond_status(w, 404, "Not Found", close);
		},
		_ => return respond_status(w, 500, "Internal Server Error", close),
	};
	write_head(w, 200, "OK", len, close)?;
	io::copy(&mut file, w)?;
	w.flush()
}

/// A response whose body is just the status line's reason, as plain text.
fn respond_status(w: &mut TcpStream, code: u16, reason: &str, close: bool) -> io::Result<()> {
	write_head(w, code, reason, reason.len() as u64, close)?;
	w.write_all(reason.as_bytes())?;
	w.flush()
}

fn write_head(w: &mut TcpStream, code: u16, reason: &str, len: u64, close: bool) -> io::Result<()> {
	let connection = if close { "Connection: close\r\n" } else { "" };
	let head = format!("HTTP/1.1 {code} {reason}\r\nContent-Length: {len}\r\n{connection}\r\n");
	w.write_all(head.as_bytes())
}
