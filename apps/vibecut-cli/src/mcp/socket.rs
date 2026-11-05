//! OS-owned authentication: private Unix directory + peer uid, or a protected
//! Windows named-pipe DACL granting only its owner. No client-config secret.
use super::{
    Handler,
    transport::{Framed, frame},
};
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
        ListenerOptions::new()
            .name(name(endpoint)?)
            .mode(0o600)
            .create_tokio()
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
    let slots = Arc::new(Semaphore::new(8));
    let mut tasks = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let Ok(stream) = accepted else { break; };
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

pub(super) async fn proxy(endpoint: &Path) -> io::Result<()> {
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
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
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
        let metadata = std::fs::metadata(endpoint).unwrap();
        assert_eq!(metadata.mode() & 0o777, 0o600);
        assert_eq!(metadata.uid(), rustix::process::geteuid().as_raw());
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
