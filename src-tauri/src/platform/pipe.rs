//! The local IPC transport, behind the platform boundary.
//!
//! D9 makes the control API a named pipe rather than localhost HTTP so the
//! zero-network guarantee is true by construction. Which pipe API that is
//! belongs here, with the rest of the non-portable surface, and not as a
//! `#[cfg(windows)]` in `control.rs` (#20). Both channels of the protocol use
//! it: the control pipe's NDJSON and each viz subscriber's binary frames are
//! byte streams; framing is the caller's.
//!
//! On Windows this is `tokio`'s named-pipe server. The stub for everything
//! else refuses to listen, which is honest: nothing else builds today (O7),
//! and a POSIX socket lands with a port rather than as dead code here.

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

pub use imp::{listen, Listener};

#[cfg(windows)]
mod imp {
    use super::{Conn, ListenOptions};
    use std::io;
    use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

    /// One pipe instance, created and waiting. Windows semantics: an instance
    /// serves one client, so the caller creates the next one after `accept`.
    pub struct Listener(NamedPipeServer);

    /// Create an instance of `name`, ready for a client. Created *before* the
    /// caller replies with the name, so a client never finds nothing listening.
    pub fn listen(name: &str, opts: ListenOptions) -> io::Result<Listener> {
        let mut o = ServerOptions::new();
        if opts.out_buffer > 0 {
            o.out_buffer_size(opts.out_buffer);
        }
        Ok(Listener(o.create(name)?))
    }

    impl Listener {
        /// Wait for a client, then hand the connected stream over.
        pub async fn accept(self) -> io::Result<Conn> {
            self.0.connect().await?;
            Ok(Box::new(self.0))
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{Conn, ListenOptions};
    use std::io;

    pub struct Listener;

    pub fn listen(_name: &str, _opts: ListenOptions) -> io::Result<Listener> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "no local IPC transport on this platform yet (control-api.md names the POSIX socket paths)",
        ))
    }

    impl Listener {
        pub async fn accept(self) -> io::Result<Conn> {
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }
    }
}
