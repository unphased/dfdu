//! Local line-delimited JSON peer transport. Commands run on the UI thread.
use std::io::{self, BufRead, BufReader, Read, Write};
#[cfg(test)]
use std::net::SocketAddr;
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
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
    pub fn bind(addr: &str, key: impl Into<Option<String>>) -> io::Result<Self> {
        let key = key.into();
        let listener = TcpListener::bind(addr)?;
        #[cfg(test)]
        let addr = listener.local_addr()?;
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
                        std::thread::spawn(move || serve(stream, key.as_deref(), &tx));
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

fn serve(mut stream: TcpStream, key: Option<&str>, requests: &Sender<Request>) {
    if stream.set_nonblocking(false).is_err() {
        return;
    }
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));
    let Ok(read_stream) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(read_stream);
    let mut authenticated = key.is_none();
    loop {
        let mut line = Vec::new();
        let Ok(n) = (&mut reader).take(65537).read_until(b'\n', &mut line) else {
            break;
        };
        if n == 0 {
            break;
        }
        if n > 65536 || !line.ends_with(b"\n") {
            let _ = send(&mut stream, &json!({"ok":false,"error":"invalid request"}));
            break;
        }
        let parsed = serde_json::from_slice::<Value>(&line);
        if !authenticated {
            if parsed.as_ref().is_ok_and(|value| {
                value.as_object().is_some_and(|object| {
                    object.len() == 1 && object.get("key").and_then(Value::as_str) == key
                })
            }) {
                authenticated = true;
                continue;
            }
            let _ = send(
                &mut stream,
                &json!({"ok":false,"error":"authentication failed"}),
            );
            break;
        }
        let Ok(Value::Object(object)) = parsed else {
            if send(&mut stream, &json!({"ok":false,"error":"invalid JSON"})).is_err() {
                break;
            }
            continue;
        };
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

/// A single worker owns the connection, keeping network waits off the UI thread.
pub(crate) struct Outgoing {
    pub queries: Sender<Value>,
    pub replies: Receiver<String>,
}

impl Outgoing {
    pub fn start(addr: String, key: Option<String>) -> Self {
        let (queries, incoming) = unbounded::<Value>();
        let (outgoing, replies) = unbounded();
        std::thread::spawn(move || {
            let mut connection: Option<(TcpStream, BufReader<TcpStream>)> = None;
            while let Ok(mut query) = incoming.recv() {
                while let Ok(newer) = incoming.try_recv() {
                    query = newer;
                }
                let reused = connection.is_some();
                let mut retry = reused;
                let result = loop {
                    let result = (|| -> io::Result<Value> {
                        if connection.is_none() {
                            let mut stream = connect(&addr)?;
                            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                            stream.set_write_timeout(Some(Duration::from_secs(2)))?;
                            if let Some(key) = &key {
                                send(&mut stream, &json!({"key":key}))?;
                            }
                            let reader = BufReader::new(stream.try_clone()?);
                            connection = Some((stream, reader));
                        }
                        let (stream, reader) = connection.as_mut().unwrap();
                        send(stream, &query)?;
                        let mut line = Vec::new();
                        let n = (&mut *reader)
                            .take(1_048_577)
                            .read_until(b'\n', &mut line)?;
                        if n == 0 {
                            return Err(io::Error::new(
                                io::ErrorKind::UnexpectedEof,
                                "peer closed connection",
                            ));
                        }
                        if n > 1_048_576 || !line.ends_with(b"\n") {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "invalid peer reply",
                            ));
                        }
                        serde_json::from_slice(&line).map_err(io::Error::other)
                    })();
                    if retry
                        && result.as_ref().is_err_and(|error| {
                            matches!(
                                error.kind(),
                                io::ErrorKind::UnexpectedEof
                                    | io::ErrorKind::BrokenPipe
                                    | io::ErrorKind::ConnectionReset
                                    | io::ErrorKind::ConnectionAborted
                                    | io::ErrorKind::NotConnected
                            )
                        })
                    {
                        connection = None;
                        retry = false;
                    } else {
                        break result;
                    }
                };
                let status = match result {
                    Ok(reply) => format!("peer: {reply}"),
                    Err(e) => {
                        connection = None;
                        format!("peer: {e}")
                    }
                };
                let _ = outgoing.send(status);
            }
        });
        Self { queries, replies }
    }
}

fn connect(addr: impl ToSocketAddrs) -> io::Result<TcpStream> {
    let mut last_error = None;
    for addr in addr.to_socket_addrs()? {
        match TcpStream::connect_timeout(&addr, Duration::from_secs(2)) {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| io::Error::other("invalid peer address")))
}

fn send(stream: &mut TcpStream, value: &Value) -> io::Result<()> {
    serde_json::to_writer(&mut *stream, value)?;
    stream.write_all(b"\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_tries_later_addresses() {
        let unavailable = TcpListener::bind("127.0.0.1:0").unwrap();
        let unavailable_addr = unavailable.local_addr().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let available_addr = listener.local_addr().unwrap();
        drop(unavailable);
        let stream = connect(&[unavailable_addr, available_addr][..]).unwrap();
        assert_eq!(stream.peer_addr().unwrap(), available_addr);
    }

    fn reply_status(reply: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            stream.write_all(&reply).unwrap();
        });
        let outgoing = Outgoing::start(addr.to_string(), None);
        outgoing.queries.send(json!({"test": true})).unwrap();
        let status = outgoing
            .replies
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        server.join().unwrap();
        status
    }

    #[test]
    fn outgoing_accepts_complete_reply() {
        assert_eq!(
            reply_status(b"{\"ok\":true}\n".to_vec()),
            "peer: {\"ok\":true}"
        );
    }

    #[test]
    fn outgoing_rejects_oversized_and_unterminated_replies() {
        assert!(reply_status(vec![b'x'; 1_048_577]).contains("invalid peer reply"));
        assert!(reply_status(b"{\"ok\":true}".to_vec()).contains("invalid peer reply"));
    }
}
