//! Local line-delimited JSON peer transport. Commands run on the UI thread.
use std::io::{self, BufRead, BufReader, Read, Write};
#[cfg(test)]
use std::net::SocketAddr;
use std::net::{TcpListener, TcpStream};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, unbounded};
use serde_json::{Value, json};

pub(crate) struct Request {
    pub command: Value,
    pub reply: Sender<Value>,
}

pub(crate) struct Listener {
    #[cfg(test)]
    addr: SocketAddr,
    requests: Receiver<Request>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Listener {
    pub fn bind(addr: &str, key: String) -> io::Result<Self> {
        if key.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "peer key must not be empty",
            ));
        }
        let listener = TcpListener::bind(addr)?;
        let addr = listener.local_addr()?;
        if !addr.ip().is_loopback() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "peer listener must use a loopback address",
            ));
        }
        listener.set_nonblocking(true)?;
        let (tx, requests) = unbounded();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let tx = tx.clone();
                        let key = key.clone();
                        std::thread::spawn(move || serve(stream, &key, &tx));
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(50))
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            #[cfg(test)]
            addr,
            requests,
            stop,
            thread: Some(thread),
        })
    }

    #[cfg(test)]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
    pub fn requests(&self) -> &Receiver<Request> {
        &self.requests
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(mut stream: TcpStream, key: &str, requests: &Sender<Request>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));
    let Ok(read_stream) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(read_stream);
    loop {
        let mut line = Vec::new();
        let Ok(n) = (&mut reader).take(65537).read_until(b'\n', &mut line) else {
            break;
        };
        if n == 0 {
            break;
        }
        if n > 65536 || !line.ends_with(b"\n") {
            let _ = send(&mut stream, &json!({"error":"invalid request"}));
            break;
        }
        let Ok(Value::Object(mut object)) = serde_json::from_slice::<Value>(&line) else {
            if send(&mut stream, &json!({"error":"invalid JSON"})).is_err() {
                break;
            }
            continue;
        };
        if object.remove("key").as_ref().and_then(Value::as_str) != Some(key) {
            let _ = send(&mut stream, &json!({"error":"authentication failed"}));
            break;
        }
        let (reply, response) = crossbeam_channel::bounded(1);
        if requests
            .send(Request {
                command: Value::Object(object),
                reply,
            })
            .is_err()
        {
            break;
        }
        let Ok(value) = response.recv_timeout(Duration::from_secs(30)) else {
            break;
        };
        if send(&mut stream, &value).is_err() {
            break;
        }
    }
}

fn send(stream: &mut TcpStream, value: &Value) -> io::Result<()> {
    serde_json::to_writer(&mut *stream, value)?;
    stream.write_all(b"\n")
}
