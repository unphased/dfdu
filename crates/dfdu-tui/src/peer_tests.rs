use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde_json::{Value, json};

use crate::peer::Listener;

fn connect(listener: &Listener) -> (TcpStream, BufReader<TcpStream>) {
    let stream = TcpStream::connect(listener.addr()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let reader = BufReader::new(stream.try_clone().unwrap());
    (stream, reader)
}

fn response(reader: &mut BufReader<TcpStream>) -> Value {
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

fn dispatch(listener: &Listener, command: Value, reply: Value) {
    let request = listener
        .requests()
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
    assert_eq!(request.command, command);
    request.reply.send(reply).unwrap();
}

#[test]
fn separate_authentication_allows_multiple_independent_requests() {
    let listener = Listener::bind("127.0.0.1:0", Some("secret".into())).unwrap();
    let (mut stream, mut reader) = connect(&listener);
    writeln!(stream, "{}", json!({"key":"secret"})).unwrap();
    stream.write_all(b"{\"state\":{}}\n").unwrap();
    dispatch(&listener, json!({"state":{}}), json!({"count":1}));
    assert_eq!(response(&mut reader), json!({"count":1}));
    writeln!(
        stream,
        "{}",
        json!({"key":"ordinary field", "match":{"size":42}})
    )
    .unwrap();
    dispatch(
        &listener,
        json!({"key":"ordinary field", "match":{"size":42}}),
        json!({"rank":1}),
    );
    assert_eq!(response(&mut reader), json!({"rank":1}));
}

#[test]
fn no_key_allows_requests_and_recovers_from_bad_json() {
    let listener = Listener::bind("127.0.0.1:0", None).unwrap();
    let (mut stream, mut reader) = connect(&listener);
    writeln!(stream, "not json").unwrap();
    assert_eq!(response(&mut reader), json!({"error":"invalid JSON"}));
    stream.write_all(b"{\"state\":{}}\n").unwrap();
    dispatch(&listener, json!({"state":{}}), json!({"ok":true}));
    assert_eq!(response(&mut reader), json!({"ok":true}));
}

#[test]
fn bad_authentication_is_rejected_without_dispatch() {
    let listener = Listener::bind("127.0.0.1:0", Some("secret".into())).unwrap();
    for text in [
        "not json",
        r#"{"key":"wrong"}"#,
        r#"{"state":{}}"#,
        r#"{"key":"secret","state":{}}"#,
    ] {
        let (mut stream, mut reader) = connect(&listener);
        writeln!(stream, "{text}").unwrap();
        assert_eq!(
            response(&mut reader),
            json!({"error":"authentication failed"})
        );
        let mut line = String::new();
        assert_eq!(reader.read_line(&mut line).unwrap(), 0);
    }
    assert!(listener.requests().try_recv().is_err());
}

#[test]
fn binds_non_loopback_address() {
    let listener = Listener::bind("0.0.0.0:0", None).unwrap();
    assert!(listener.addr().ip().is_unspecified());
}
