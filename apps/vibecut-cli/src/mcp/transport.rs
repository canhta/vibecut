//! Bounded, strict newline framing before rmcp deserializes protocol messages.
use engine::control::{MAX_FRAME_BYTES, MAX_RESPONSE_BYTES, bounded_json, parse_frame};
use rmcp::{
    service::{RoleServer, RxJsonRpcMessage, TxJsonRpcMessage},
    transport::Transport,
};
use std::{
    collections::BTreeSet,
    io,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    sync::{Mutex, Semaphore},
};

pub(super) struct Framed<R, W> {
    read: BufReader<R>,
    write: Arc<Mutex<W>>,
    window: Instant,
    count: usize,
    pending: Vec<u8>,
    started: Option<Instant>,
    outstanding: Arc<std::sync::Mutex<BTreeSet<String>>>,
    writes: Arc<Semaphore>,
}
impl<R: AsyncRead, W> Framed<R, W> {
    pub fn new(read: R, write: W) -> Self {
        Self {
            read: BufReader::new(read),
            write: Arc::new(Mutex::new(write)),
            window: Instant::now(),
            count: 0,
            pending: Vec::new(),
            started: None,
            outstanding: Arc::new(std::sync::Mutex::new(BTreeSet::new())),
            writes: Arc::new(Semaphore::new(4)),
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

impl<R, W> Transport<RoleServer> for Framed<R, W>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send + 'static,
{
    type Error = io::Error;
    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleServer>,
    ) -> impl Future<Output = io::Result<()>> + Send + 'static {
        let write = self.write.clone();
        let outstanding = self.outstanding.clone();
        let permit = self.writes.clone().try_acquire_owned();
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
        async move {
            let _permit = permit.map_err(|_| io::ErrorKind::WouldBlock)?;
            // The handler has completed. Release its ID before publishing bytes:
            // a client may receive them and reuse the ID before flush wakes us.
            if let Some(id) = response_id {
                outstanding
                    .lock()
                    .map_err(|_| io::ErrorKind::Other)?
                    .remove(&id);
            }
            let result = tokio::time::timeout(Duration::from_secs(5), async {
                let mut write = write.lock().await;
                let mut bytes = bounded_json(&item, MAX_RESPONSE_BYTES)
                    .map_err(|_| io::ErrorKind::InvalidData)?;
                bytes.push(b'\n');
                write.write_all(&bytes).await?;
                write.flush().await
            })
            .await;
            result.map_err(|_| io::ErrorKind::TimedOut)?
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
            self.outstanding.lock().ok()?.remove(&id);
        }
        if let Some(id) = request_id {
            let mut outstanding = self.outstanding.lock().ok()?;
            if id.len() > 128 || outstanding.len() >= 4 || !outstanding.insert(id) {
                return None;
            }
        }
        Some(message)
    }
    async fn close(&mut self) -> io::Result<()> {
        tokio::time::timeout(Duration::from_secs(1), self.write.lock().await.shutdown())
            .await
            .map_err(|_| io::ErrorKind::TimedOut)?
    }
}
