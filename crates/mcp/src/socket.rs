//! Authenticates local MCP peers with OS-owned access controls.
use super::{Framed, Handler, frame};
#[cfg(unix)]
use interprocess::local_socket::{GenericFilePath, ToFsName};
#[cfg(windows)]
use interprocess::local_socket::{GenericNamespaced, ToNsName};
use interprocess::local_socket::{
    ListenerOptions,
    tokio::{Listener, Stream},
    traits::{
        StreamCommon as _,
        tokio::{Listener as _, Stream as _},
    },
};
use rmcp::ServiceExt;
use std::{io, path::Path, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    sync::Semaphore,
};

fn name(endpoint: &Path) -> io::Result<interprocess::local_socket::Name<'_>> {
    #[cfg(unix)]
    {
        endpoint.to_fs_name::<GenericFilePath>()
    }
    #[cfg(windows)]
    {
        endpoint.as_os_str().to_ns_name::<GenericNamespaced>()
    }
}

pub(super) fn listen(endpoint: &Path) -> io::Result<Listener> {
    #[cfg(unix)]
    {
        #[cfg(not(target_os = "macos"))]
        use interprocess::os::unix::local_socket::ListenerOptionsExt;
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
        let parent = endpoint.parent().ok_or(io::ErrorKind::InvalidInput)?;
        // Never repair a broad or foreign directory: that could hide a spoofed endpoint.
        if !parent.exists() {
            std::fs::DirBuilder::new().mode(0o700).create(parent)?;
        }
        let metadata = std::fs::symlink_metadata(parent)?;
        if !metadata.is_dir()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let options = ListenerOptions::new().name(name(endpoint)?);
        #[cfg(not(target_os = "macos"))]
        let options = options.mode(0o600);
        let listener = options.create_tokio()?;
        // macOS rejects fchmod on an unbound socket. The verified private
        // directory denies foreign traversal during bind/chmod; never change
        // the process-wide umask in this multithreaded application.
        #[cfg(target_os = "macos")]
        std::fs::set_permissions(endpoint, std::fs::Permissions::from_mode(0o600))?;
        Ok(listener)
    }
    #[cfg(windows)]
    {
        use interprocess::os::windows::{
            local_socket::ListenerOptionsExt, security_descriptor::SecurityDescriptor,
        };
        let descriptor = widestring::U16CString::from_str("D:P(A;;GA;;;OW)")
            .map_err(|_| io::ErrorKind::InvalidInput)?;
        let descriptor = SecurityDescriptor::deserialize(&descriptor)?;
        ListenerOptions::new()
            .name(name(endpoint)?)
            .security_descriptor(descriptor)
            .create_tokio()
    }
}

fn authenticate(stream: &Stream) -> io::Result<()> {
    let peer = stream.peer_creds()?;
    #[cfg(unix)]
    if peer.euid() != Some(rustix::process::geteuid().as_raw()) {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    #[cfg(windows)]
    if peer.pid().is_none() {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    Ok(())
}

pub(super) async fn serve(listener: Listener, handler: Handler) {
    serve_accept(|| listener.accept(), handler).await;
}

async fn serve_accept<F, A>(mut accept: F, handler: Handler)
where
    F: FnMut() -> A,
    A: std::future::Future<Output = io::Result<Stream>>,
{
    let slots = Arc::new(Semaphore::new(8));
    let mut tasks = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            accepted = accept() => {
                let stream = match accepted {
                    Ok(stream) => stream,
                    Err(error) => {
                        tracing::warn!(category = ?error.kind(), "control accept failed; retrying");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                };
                if authenticate(&stream).is_err() { continue; }
                let Ok(slot) = slots.clone().try_acquire_owned() else { continue; };
                let handler = handler.clone();
                tasks.spawn(async move {
                    let _slot = slot;
                    let (read, write) = tokio::io::split(stream);
                    if let Ok(Ok(service)) = tokio::time::timeout(Duration::from_secs(5), handler.serve(Framed::new(read, write))).await {
                        let _ = service.waiting().await;
                    }
                });
            }
            _ = tasks.join_next(), if !tasks.is_empty() => {}
        }
    }
}

async fn forward<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    read: R,
    mut write: W,
) -> io::Result<()> {
    let mut read = BufReader::new(read);
    while let Some(bytes) = frame(&mut read).await? {
        tokio::time::timeout(Duration::from_secs(5), async {
            write.write_all(&bytes).await?;
            write.flush().await
        })
        .await
        .map_err(|_| io::ErrorKind::TimedOut)??;
    }
    Ok(())
}

pub async fn proxy(endpoint: &Path) -> io::Result<()> {
    let stream = tokio::time::timeout(Duration::from_secs(5), Stream::connect(name(endpoint)?))
        .await
        .map_err(|_| io::ErrorKind::TimedOut)??;
    authenticate(&stream)?;
    #[cfg(windows)]
    authenticate_server_owner(&stream)?;
    let (read, write) = tokio::io::split(stream);
    // Either EOF closes this connection only; the app and its receipt ledger survive.
    tokio::select! {
        result = forward(tokio::io::stdin(), write) => result,
        result = forward(read, tokio::io::stdout()) => result,
    }
}

#[cfg(windows)]
fn authenticate_server_owner(stream: &Stream) -> io::Result<()> {
    use std::os::windows::io::AsHandle;
    use win_authorization_sys::{ObjectType, SecurityInformation, get_security_info};
    // Compare owners on open kernel handles, avoiding PID lookup/reuse races.
    // A fresh anonymous pipe obtains this process token's default owner from OS.
    let (own, _write) = std::io::pipe()?;
    let Stream::NamedPipe(peer) = stream;
    let owner = |handle| -> io::Result<_> {
        get_security_info(handle, ObjectType::FILE, SecurityInformation::OWNER)
            .and_then(|descriptor| descriptor.to_sddl(SecurityInformation::OWNER))
            .map_err(|_| io::Error::from(io::ErrorKind::PermissionDenied))
    };
    if owner(own.as_handle())? != owner(peer.as_handle())? {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn mcp_socket_refuses_broad_directory_and_uses_owner_permissions() {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
        let root = tempfile::tempdir().unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _enter = runtime.enter();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            listen(&root.path().join("control.sock"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let endpoint = root.path().join("control.sock");
        let _listener = listen(&endpoint).unwrap();
        assert_eq!(
            std::fs::metadata(root.path()).unwrap().mode() & 0o777,
            0o700
        );
        let metadata = std::fs::metadata(&endpoint).unwrap();
        assert_eq!(metadata.mode() & 0o777, 0o600);
        assert_eq!(metadata.uid(), rustix::process::geteuid().as_raw());
        // An alias of a private directory is refused rather than followed.
        let real = root.path().join("real");
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&real)
            .unwrap();
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        assert_eq!(
            listen(&alias.join("control.sock")).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        // A taken name is never replaced or re-permissioned, live socket or stale file.
        assert!(listen(&endpoint).is_err());
        let stale = real.join("stale.sock");
        std::fs::write(&stale, b"").unwrap();
        std::fs::set_permissions(&stale, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(listen(&stale).is_err());
        assert_eq!(std::fs::metadata(&stale).unwrap().mode() & 0o777, 0o644);
        assert_eq!(std::fs::metadata(&endpoint).unwrap().mode() & 0o777, 0o600);
    }

    #[tokio::test(start_paused = true)]
    async fn mcp_accept_error_backs_off_then_serves_same_user() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::AsyncBufReadExt;
        let root = tempfile::tempdir().unwrap();
        let endpoint = if cfg!(windows) {
            std::path::PathBuf::from(format!("vibecut-accept-test-{}", std::process::id()))
        } else {
            root.path().join("private/control.sock")
        };
        let listener = listen(&endpoint).unwrap();
        let (handler, _inbox) = super::super::Inbox::new(
            Default::default(),
            engine::ByteBudget::new(2 << 30).unwrap(),
        )
        .unwrap();
        let attempts = Arc::new(AtomicUsize::new(0));
        let observed = attempts.clone();
        let task = tokio::spawn(async move {
            serve_accept(
                || {
                    let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                    let listener = &listener;
                    async move {
                        if attempt == 0 {
                            Err(io::ErrorKind::Interrupted.into())
                        } else {
                            listener.accept().await
                        }
                    }
                },
                handler,
            )
            .await;
        });
        tokio::task::yield_now().await;
        assert_eq!(observed.load(Ordering::SeqCst), 1);
        tokio::time::advance(Duration::from_millis(99)).await;
        tokio::task::yield_now().await;
        assert_eq!(observed.load(Ordering::SeqCst), 1, "accept must back off");
        assert!(
            !task.is_finished(),
            "an accept error must not stop the listener"
        );
        tokio::time::advance(Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
        assert_eq!(observed.load(Ordering::SeqCst), 2);
        let mut client = Stream::connect(name(&endpoint).unwrap()).await.unwrap();
        authenticate(&client).unwrap();
        client.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{},\"clientInfo\":{\"name\":\"retry-test\",\"version\":\"1\"}}}\n").await.unwrap();
        let mut line = String::new();
        tokio::time::timeout(
            Duration::from_secs(5),
            BufReader::new(client).read_line(&mut line),
        )
        .await
        .unwrap()
        .unwrap();
        let response: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert!(response.get("result").is_some(), "{response}");
        assert!(!task.is_finished());
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
    }

    #[cfg(windows)]
    #[test]
    fn mcp_named_pipe_missing_permission_is_enforced_by_os() {
        use interprocess::os::windows::{
            local_socket::ListenerOptionsExt, security_descriptor::SecurityDescriptor,
        };
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let endpoint =
                std::path::PathBuf::from(format!("vibecut-denied-{}", std::process::id()));
            // A protected empty DACL grants no identity access, including this client.
            let descriptor = widestring::U16CString::from_str("D:P").unwrap();
            let descriptor = SecurityDescriptor::deserialize(&descriptor).unwrap();
            let _listener = ListenerOptions::new()
                .name(name(&endpoint).unwrap())
                .security_descriptor(descriptor)
                .create_tokio()
                .unwrap();
            let error = Stream::connect(name(&endpoint).unwrap()).await.unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        });
    }
}
