#![allow(clippy::unwrap_used, reason = "loopback shared transport regression")]
use ai::transport::{HttpRequest, HttpTransport, UreqTransport};
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};
#[test]
fn shared_transport_honors_the_declared_request_method() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut byte = [0; 1];
        while !request.ends_with(b"\r\n\r\n") {
            socket.read_exact(&mut byte).unwrap();
            request.extend_from_slice(&byte);
        }
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .unwrap();
        String::from_utf8(request).unwrap()
    });
    let response = UreqTransport::new()
        .send(HttpRequest {
            method: "GET",
            url: format!("http://{address}/fixture"),
            headers: Vec::new(),
            body: Vec::new(),
        })
        .unwrap();
    assert_eq!(response.body, b"ok");
    assert!(server.join().unwrap().starts_with("GET /fixture HTTP/1.1"));
}
