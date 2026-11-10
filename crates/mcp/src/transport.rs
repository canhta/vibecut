//! Bounded, strict newline framing before rmcp deserializes protocol messages.
use engine::control::{MAX_FRAME_BYTES, MAX_RESPONSE_BYTES, bounded_json, parse_frame};
use rmcp::{
    service::{RoleServer, RxJsonRpcMessage, TxJsonRpcMessage},
    transport::Transport,
};
use std::{
    collections::BTreeSet,
    io,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    sync::{mpsc, oneshot},
};

struct Outgoing {
    item: TxJsonRpcMessage<RoleServer>,
    reply: oneshot::Sender<io::Result<()>>,
}

pub struct Framed<R> {
    read: BufReader<R>,
    write: Option<mpsc::Sender<Outgoing>>,
    writer: Option<tokio::task::JoinHandle<io::Result<()>>>,
    window: Instant,
    count: usize,
    pending: Vec<u8>,
    started: Option<Instant>,
    outstanding: BTreeSet<String>,
}
impl<R: AsyncRead> Framed<R> {
    pub fn new<W: AsyncWrite + Unpin + Send + 'static>(read: R, write: W) -> Self {
        let (send, receive) = mpsc::channel(4);
        Self {
            read: BufReader::new(read),
            write: Some(send),
            writer: Some(tokio::spawn(write_frames(write, receive))),
            window: Instant::now(),
            count: 0,
            pending: Vec::new(),
            started: None,
            outstanding: BTreeSet::new(),
        }
    }
}

// One task owns the writer. There is no state lock held across IO or await.
async fn write_frames<W: AsyncWrite + Unpin>(
    mut write: W,
    mut receive: mpsc::Receiver<Outgoing>,
) -> io::Result<()> {
    while let Some(frame) = receive.recv().await {
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            let mut bytes = bounded_json(&frame.item, MAX_RESPONSE_BYTES - 1)
                .map_err(|_| io::ErrorKind::InvalidData)?;
            bytes.push(b'\n');
            write.write_all(&bytes).await?;
            write.flush().await
        })
        .await
        .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))
        .and_then(|result| result);
        let failed = result.is_err();
        let _ = frame.reply.send(result);
        if failed {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
    }
    tokio::time::timeout(Duration::from_secs(1), write.shutdown())
        .await
        .map_err(|_| io::ErrorKind::TimedOut)?
}

impl<R> Drop for Framed<R> {
    fn drop(&mut self) {
        if let Some(writer) = &self.writer {
            writer.abort();
        }
    }
}

pub(super) async fn frame<R: AsyncRead + Unpin>(
    read: &mut BufReader<R>,
) -> io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    read_frame(read, &mut bytes, &mut None).await
}

async fn read_frame<R: AsyncRead + Unpin>(
    read: &mut BufReader<R>,
    bytes: &mut Vec<u8>,
    started: &mut Option<Instant>,
) -> io::Result<Option<Vec<u8>>> {
    loop {
        let available = if let Some(started) = started {
            let remaining = Duration::from_secs(5).saturating_sub(started.elapsed());
            tokio::time::timeout(remaining, read.fill_buf())
                .await
                .map_err(|_| io::ErrorKind::TimedOut)??
        } else {
            read.fill_buf().await?
        };
        if available.is_empty() {
            return if bytes.is_empty() {
                Ok(None)
            } else {
                Err(io::ErrorKind::InvalidData.into())
            };
        }
        let end = available.iter().position(|byte| *byte == b'\n');
        started.get_or_insert_with(Instant::now);
        let count = end.map_or(available.len(), |position| position + 1);
        if count > MAX_FRAME_BYTES.saturating_sub(bytes.len()) {
            return Err(io::ErrorKind::InvalidData.into());
        }
        bytes.extend_from_slice(available.get(..count).ok_or(io::ErrorKind::InvalidData)?);
        read.consume(count);
        if end.is_some() {
            *started = None;
            return Ok(Some(std::mem::take(bytes)));
        }
    }
}

impl<R> Transport<RoleServer> for Framed<R>
where
    R: AsyncRead + Unpin + Send,
{
    type Error = io::Error;
    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleServer>,
    ) -> impl Future<Output = io::Result<()>> + Send + 'static {
        let response_id = match &item {
            rmcp::model::JsonRpcMessage::Response(response) => {
                serde_json::to_string(&response.id).ok()
            }
            rmcp::model::JsonRpcMessage::Error(error) => error
                .id
                .as_ref()
                .and_then(|id| serde_json::to_string(id).ok()),
            _ => None,
        };
        // Release the completed handler's ID before publishing bytes. A client
        // may receive them and reuse the ID before the writer's flush wakes us.
        if let Some(id) = response_id {
            self.outstanding.remove(&id);
        }
        let (reply, receive) = oneshot::channel();
        let queued = self
            .write
            .as_ref()
            .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))
            .and_then(|writer| {
                writer
                    .try_send(Outgoing { item, reply })
                    .map_err(|_| io::Error::from(io::ErrorKind::WouldBlock))
            });
        async move {
            queued?;
            tokio::time::timeout(Duration::from_secs(5), receive)
                .await
                .map_err(|_| io::ErrorKind::TimedOut)?
                .map_err(|_| io::ErrorKind::BrokenPipe)?
        }
    }
    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleServer>> {
        // Closing on hostile framing bounds both memory and parser/task rate;
        // clients recover engine mutation outcomes using the original key.
        let bytes = read_frame(&mut self.read, &mut self.pending, &mut self.started)
            .await
            .ok()??;
        if self.window.elapsed() >= Duration::from_secs(1) {
            self.window = Instant::now();
            self.count = 0;
        }
        self.count += 1;
        if self.count > 64 {
            return None;
        }
        let value = parse_frame(&bytes).ok()?;
        let request_id = value
            .get("method")
            .and_then(|_| value.get("id"))
            .map(serde_json::to_string)
            .transpose()
            .ok()?;
        let cancelled_id = if value
            .get("method")
            .is_some_and(|method| method == "notifications/cancelled")
        {
            value
                .pointer("/params/requestId")
                .and_then(|id| serde_json::to_string(id).ok())
        } else {
            None
        };
        let message = serde_json::from_value(value).ok()?;
        if let Some(id) = cancelled_id {
            self.outstanding.remove(&id);
        }
        if let Some(id) = request_id
            && (id.len() > 128 || self.outstanding.len() >= 4 || !self.outstanding.insert(id))
        {
            return None;
        }
        Some(message)
    }
    async fn close(&mut self) -> io::Result<()> {
        drop(self.write.take());
        let Some(mut writer) = self.writer.take() else {
            return Ok(());
        };
        match tokio::time::timeout(Duration::from_secs(1), &mut writer).await {
            Ok(result) => result.map_err(|_| io::ErrorKind::Other)?,
            Err(_) => {
                writer.abort();
                let _ = writer.await;
                Err(io::ErrorKind::TimedOut.into())
            }
        }
    }
}
