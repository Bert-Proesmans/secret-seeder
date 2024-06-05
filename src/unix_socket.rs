use hyper_util::rt::TokioIo;
use std::path::{Path, PathBuf};
use tokio::net::UnixStream;
use tokio_stream::wrappers::UnixListenerStream;

// Wrapper struct for unix socket paths. The socket path must be unlinked at the end of
// the program, otherwise the next run will panic with E_ADDR_IN_USE.
//
// REF; https://stackoverflow.com/a/40218765
pub struct DeleteOnDrop {
    // ERROR; Important to consider the lifetime of the owned object!
    // It's wrong to _only_ track the path without the listener object itself because
    // that leads to a footgun where the path is unlinked before the listener is shutdown!
    path: PathBuf,
    pub stream: UnixListenerStream,
}

impl DeleteOnDrop {
    pub fn bind(path: impl AsRef<Path>) -> std::io::Result<Self> {
        use tokio::net::UnixListener;

        let path = path.as_ref().to_owned();
        UnixListener::bind(&path)
            .map(|listener| UnixListenerStream::new(listener))
            .map(|stream| DeleteOnDrop { path, stream })
    }
}

impl Drop for DeleteOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path).expect("Failed to remove the provided file path!");
    }
}

impl std::ops::Deref for DeleteOnDrop {
    type Target = UnixListenerStream;

    fn deref(&self) -> &Self::Target {
        &self.stream
    }
}

// WARN; Implementation required to make UnixListenerStream compatible with `Stream`
// See [warp::server::Server::run_incoming]
impl std::ops::DerefMut for DeleteOnDrop {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.stream
    }
}

pub async fn connect_unix_sock_stream(
) -> Result<TokioIo<UnixStream>, Box<dyn std::error::Error + Send + Sync>> {
    let stream = TokioIo::new(UnixStream::connect("/tmp/warp.sock").await.unwrap());

    Ok(stream)
}
