use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde_json::json;

use crate::peer::Listener;

#[test]
fn authenticated_requests_are_dispatched_and_answered() {
    let listener = Listener::bind("127.0.0.1:0", "secret".into()).unwrap();
    let mut stream = TcpStream::connect(listener.addr()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    writeln!(
        stream,
        "{}",
        json!({"key":"secret","match":{"size":42,"name":"file","parents":["dir"]}})
    )
    .unwrap();
    let request = listener
        .requests()
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
    assert_eq!(
        request.command,
        json!({"match":{"size":42,"name":"file","parents":["dir"]}})
    );
    request
        .reply
        .send(json!({"count":1,"rank":1,"path":"/dir/file"}))
        .unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&line).unwrap(),
        json!({"count":1,"rank":1,"path":"/dir/file"})
    );
}

#[test]
fn bad_json_and_wrong_key_are_rejected_without_dispatch() {
    let listener = Listener::bind("127.0.0.1:0", "secret".into()).unwrap();
    for text in ["not json", "{\"key\":\"wrong\",\"state\":{}}"] {
        let mut stream = TcpStream::connect(listener.addr()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        writeln!(stream, "{text}").unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        assert!(
            serde_json::from_str::<serde_json::Value>(&line)
                .unwrap()
                .get("error")
                .is_some()
        );
    }
    assert!(listener.requests().try_recv().is_err());
}
