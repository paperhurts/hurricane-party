//! The local IPC transport, behind the platform boundary.
//!
//! D9 makes the control API a named pipe rather than localhost HTTP so the
//! zero-network guarantee is true by construction. Which pipe API that is
//! belongs here, with the rest of the non-portable surface, and not as a
//! `#[cfg(windows)]` in `control.rs` (#20). Both channels of the protocol use
//! it: the control pipe's NDJSON and each viz subscriber's binary frames are
//! byte streams; framing is the caller's.
//!
//! On Windows this is `tokio`'s named-pipe server. Where there are Unix
//! domain sockets it is a socket at `hp_control::socket_path` for the name,
//! in `$XDG_RUNTIME_DIR` (#187): the paths control-api.md has named since
//! protocol 1 was frozen (D151). A listener is made once and accepts each
//! client in turn on both.

use tokio::io::{AsyncRead, AsyncWrite};

/// A connected byte stream. `Box<dyn ...>` so the callers name no OS type.
pub trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}
pub type Conn = Box<dyn Stream>;

/// How much the OS may hold on the outbound side before a write waits.
///
/// The viz channel sets this to a couple of frames: control-api.md says a
/// subscriber that cannot keep up gets frames dropped, never buffered, and a
/// 64 KB default would bank a thousand stale frames in the kernel before the
/// writer noticed. The control channel keeps the default (`0`).
#[derive(Debug, Clone, Copy, Default)]
pub struct ListenOptions {
    pub out_buffer: u32,
}

pub use imp::{endpoint, listen, Listener};

#[cfg(windows)]
mod imp {
    use super::{Conn, ListenOptions};
    use std::io;
    use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

    /// The pipe, with an instance always waiting. An instance serves one
    /// client, so the next is created as each one connects, *before* it is
    /// handed over: a client never finds nothing listening.
    pub struct Listener {
        waiting: NamedPipeServer,
        name: String,
        opts: ListenOptions,
    }

    fn instance(name: &str, opts: ListenOptions) -> io::Result<NamedPipeServer> {
        let mut o = ServerOptions::new();
        if opts.out_buffer > 0 {
            o.out_buffer_size(opts.out_buffer);
        }
        o.create(name)
    }

    /// What a client opens: on Windows, the pipe's name itself.
    pub fn endpoint(name: &str) -> String {
        name.to_string()
    }

    /// Create the first instance of `name`, ready for a client. Created
    /// *before* the caller replies with the name.
    pub fn listen(name: &str, opts: ListenOptions) -> io::Result<Listener> {
        Ok(Listener {
            waiting: instance(name, opts)?,
            name: name.to_string(),
            opts,
        })
    }

    impl Listener {
        /// Wait for a client, then hand the connected stream over. An
        /// instance a connect failed on is replaced, so the next call waits
        /// on a good one.
        pub async fn accept(&mut self) -> io::Result<Conn> {
            if let Err(e) = self.waiting.connect().await {
                self.waiting = instance(&self.name, self.opts)?;
                return Err(e);
            }
            let next = instance(&self.name, self.opts)?;
            Ok(Box::new(std::mem::replace(&mut self.waiting, next)))
        }
    }
}

#[cfg(unix)]
mod imp {
    use super::{Conn, ListenOptions};
    use std::io;
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    use std::path::PathBuf;
    use tokio::net::UnixListener;

    /// The socket, bound at its path until it is dropped.
    pub struct Listener {
        socket: UnixListener,
        path: PathBuf,
        opts: ListenOptions,
    }

    /// Where the sockets are: `$XDG_RUNTIME_DIR`, which the session makes
    /// for this user alone. Without one (a bare `su`, a cron job), the temp
    /// folder; the socket itself is made this user's alone either way.
    fn dir() -> PathBuf {
        std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|d| d.is_absolute())
            .unwrap_or_else(std::env::temp_dir)
    }

    fn path_of(name: &str) -> PathBuf {
        hp_control::socket_path(name, &dir())
    }

    /// What a client opens: the socket's path.
    pub fn endpoint(name: &str) -> String {
        path_of(name).to_string_lossy().into_owned()
    }

    /// Bind the socket for `name`, ready for clients, before the caller
    /// replies with it. A socket left at the path by a player that did not
    /// shut down (a power cut, a kill) is taken over; one that a running
    /// player answers on is not, and neither is anything there that is not a
    /// socket.
    pub fn listen(name: &str, opts: ListenOptions) -> io::Result<Listener> {
        let path = path_of(name);
        if let Ok(meta) = std::fs::symlink_metadata(&path) {
            if !meta.file_type().is_socket() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{} is there and is not a socket", path.display()),
                ));
            }
            if std::os::unix::net::UnixStream::connect(&path).is_ok() {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    format!("another player is listening at {}", path.display()),
                ));
            }
            std::fs::remove_file(&path)?;
        }
        let socket = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        Ok(Listener { socket, path, opts })
    }

    impl Listener {
        /// Wait for a client, then hand the connected stream over.
        pub async fn accept(&mut self) -> io::Result<Conn> {
            let (stream, _) = self.socket.accept().await?;
            if self.opts.out_buffer > 0 {
                // The kernel doubles it and keeps a floor of its own; a few
                // frames' worth is still what it holds, not 200 KB of them.
                socket2::SockRef::from(&stream)
                    .set_send_buffer_size(self.opts.out_buffer as usize)?;
            }
            Ok(Box::new(stream))
        }
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A name of this test's own, so a player running beside the tests is
    /// not touched.
    fn name(what: &str) -> String {
        format!(r"\\.\pipe\hp-test-{}-{what}", std::process::id())
    }

    fn run<F: std::future::Future>(f: F) -> F::Output {
        tauri::async_runtime::block_on(f)
    }

    #[test]
    fn a_client_reaches_the_socket_at_its_endpoint_and_each_is_accepted() {
        run(async {
            let n = name("echo");
            let mut l = listen(&n, ListenOptions { out_buffer: 4096 }).unwrap();
            for _ in 0..2 {
                let mut c = tokio::net::UnixStream::connect(endpoint(&n)).await.unwrap();
                let mut s = l.accept().await.unwrap();
                s.write_all(b"hi\n").await.unwrap();
                let mut got = [0u8; 3];
                c.read_exact(&mut got).await.unwrap();
                assert_eq!(&got, b"hi\n");
            }
            let path = endpoint(&n);
            drop(l);
            assert!(!std::path::Path::new(&path).exists());
        });
    }

    /// A player killed without shutting down leaves its socket; the next one
    /// takes it over.
    #[test]
    fn a_socket_left_by_a_player_that_died_is_taken_over() {
        run(async {
            let n = name("stale");
            let path = endpoint(&n);
            // std's listener does not remove its path when dropped.
            drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
            assert!(std::path::Path::new(&path).exists());
            let mut l = listen(&n, ListenOptions::default()).unwrap();
            let _c = tokio::net::UnixStream::connect(&path).await.unwrap();
            l.accept().await.unwrap();
        });
    }

    #[test]
    fn a_running_players_socket_is_left_alone() {
        run(async {
            let n = name("live");
            let _first = listen(&n, ListenOptions::default()).unwrap();
            let e = listen(&n, ListenOptions::default()).err().unwrap();
            assert_eq!(e.kind(), std::io::ErrorKind::AddrInUse);
            assert!(std::path::Path::new(&endpoint(&n)).exists());
        });
    }

    #[test]
    fn a_file_that_is_not_a_socket_is_never_removed() {
        run(async {
            let n = name("file");
            let path = endpoint(&n);
            std::fs::write(&path, b"mine").unwrap();
            assert!(listen(&n, ListenOptions::default()).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), b"mine");
            std::fs::remove_file(&path).unwrap();
        });
    }
}
