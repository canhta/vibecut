#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use super::*;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

async fn send(write: &mut tokio::io::WriteHalf<tokio::io::DuplexStream>, value: Value) {
    write
        .write_all(format!("{value}\n").as_bytes())
        .await
        .unwrap();
}
async fn response(read: &mut BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>) -> Value {
    let mut line = String::new();
    read.read_line(&mut line).await.unwrap();
    serde_json::from_str(&line).unwrap()
}

#[tokio::test(start_paused = true)]
async fn hung_calls_expire_and_cancelled_queued_mutations_never_execute() {
    let (handler, mut inbox) =
        Inbox::new(Grants::default(), ByteBudget::new(2 << 30).unwrap()).unwrap();
    let (server, client) = tokio::io::duplex(65536);
    let (server_read, server_write) = tokio::io::split(server);
    let task = tokio::spawn(async move {
        handler
            .serve(transport::Framed::new(server_read, server_write))
            .await
            .unwrap()
            .waiting()
            .await
            .unwrap();
    });
    let (read, mut write) = tokio::io::split(client);
    let mut read = BufReader::new(read);
    send(&mut write, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"conformance","version":"1"}}})).await;
    assert!(response(&mut read).await.get("result").is_some());
    send(
        &mut write,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    let mut session = Session::new(
        project::Project::empty(project::ProjectId(1), "initial".into()).unwrap(),
        100,
    )
    .unwrap();
    let initial = session.revision();
    let request = json!({"command":"project.rename","params":{"name":"must not execute"},"base_rev":initial,"idempotency_key":"queued","request_id":"q"});
    send(&mut write, json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"command_run","arguments":request}})).await;
    // Poll until queued, without allowing the engine owner to process it.
    while inbox.receive.is_empty() {
        tokio::task::yield_now().await;
    }
    tokio::time::advance(DEADLINE + Duration::from_secs(1)).await;
    let timeout = response(&mut read).await;
    assert_eq!(
        timeout.pointer("/result/structuredContent/error/code"),
        Some(&json!("timeout_unknown_outcome"))
    );
    inbox.poll(&mut session, |_, _| Ok(())).unwrap();
    assert_eq!(session.revision(), initial);

    let request = json!({"command":"project.rename","params":{"name":"must not execute"},"base_rev":initial,"idempotency_key":"cancelled","request_id":"c"});
    send(&mut write, json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"command_run","arguments":request}})).await;
    while inbox.receive.is_empty() {
        tokio::task::yield_now().await;
    }
    send(
        &mut write,
        json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":3}}),
    )
    .await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    inbox.poll(&mut session, |_, _| Ok(())).unwrap();
    assert_eq!(session.revision(), initial);
    task.abort();
}

#[tokio::test]
async fn partial_frames_survive_receive_cancellation_and_hostile_framing_closes() {
    use rmcp::transport::Transport as _;
    for hostile in [
        b"{\"jsonrpc\":\"2.0\",\"id\":1,\"id\":2,\"method\":\"ping\"}\n".to_vec(),
        vec![b'x'; control::MAX_FRAME_BYTES + 1],
    ] {
        let (server, mut client) = tokio::io::duplex(control::MAX_FRAME_BYTES + 4096);
        let (read, write) = tokio::io::split(server);
        let mut transport = transport::Framed::new(read, write);
        client.write_all(&hostile).await.unwrap();
        assert!(transport.receive().await.is_none());
    }
    let (server, mut client) = tokio::io::duplex(1024);
    let (read, write) = tokio::io::split(server);
    let mut transport = transport::Framed::new(read, write);
    client.write_all(b"{\"jsonrpc\":\"2.0\",").await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(5), transport.receive())
            .await
            .is_err()
    );
    client
        .write_all(b"\"id\":1,\"method\":\"ping\"}\n")
        .await
        .unwrap();
    assert!(transport.receive().await.is_some());
}

#[tokio::test]
async fn message_flood_closes_before_unbounded_sdk_task_creation() {
    use rmcp::transport::Transport as _;
    let (server, mut client) = tokio::io::duplex(65536);
    let (read, write) = tokio::io::split(server);
    let mut transport = transport::Framed::new(read, write);
    let message = b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n";
    for _ in 0..65 {
        client.write_all(message).await.unwrap();
    }
    for _ in 0..64 {
        assert!(transport.receive().await.is_some());
    }
    assert!(transport.receive().await.is_none());
}
