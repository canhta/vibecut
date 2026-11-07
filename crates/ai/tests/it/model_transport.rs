#![allow(clippy::unwrap_used, reason = "loopback HTTP fixtures")]
use ai::transport::{
    DownloadTransport, HttpRequest, HttpTransport, UreqTransport, download_redirect,
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};
#[test]
fn shared_client_streams_get_without_the_provider_body_cap_and_keeps_duplicate_headers() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut byte = [0u8; 1];
        while !request.ends_with(b"\r\n\r\n") {
            socket.read_exact(&mut byte).unwrap();
            request.extend_from_slice(&byte);
        }
        let request = String::from_utf8(request).unwrap();
        assert!(request.starts_with("GET /weights HTTP/1.1"));
        assert!(request.to_ascii_lowercase().contains("range: bytes=3-"));
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2097152\r\nETag: \"one\"\r\nETag: \"two\"\r\nConnection: close\r\n\r\n").unwrap();
        for _ in 0..32 {
            socket.write_all(&[7; 65536]).unwrap();
        }
    });
    let mut response = UreqTransport::new()
        .get(
            &format!("http://{address}/weights"),
            &[("Range".into(), "bytes=3-".into())],
        )
        .unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(
        response
            .headers
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case("etag"))
            .count(),
        2
    );
    assert_eq!(
        std::io::copy(&mut response.body, &mut std::io::sink()).unwrap(),
        2097152
    );
    server.join().unwrap();
}
#[test]
fn redirect_policy_allows_signed_https_and_rejects_downgrades_or_credentials() {
    assert_eq!(
        download_redirect(
            "https://models.example/a",
            "https://cdn.example/b?signature=test"
        )
        .unwrap(),
        "https://cdn.example/b?signature=test"
    );
    for location in [
        "http://cdn.example/a",
        "file:///tmp/a",
        "https://user:secret@cdn.example/a",
        "https://cdn.example/a#fragment",
    ] {
        assert!(download_redirect("https://models.example/a", location).is_err());
    }
}

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
