//! Socket options for the listeners.
//!
//! There is exactly one thing in here and it is load-bearing: **Nagle must be
//! off on every accepted socket.**
//!
//! Nagle's algorithm withholds a small write while earlier small data is still
//! unacknowledged, and the peer's delayed-ACK timer can sit on that
//! acknowledgement for up to 40 ms (`TCP_DELACK_MIN` on Linux). A response
//! small enough to be written in two pieces therefore arrives about 40 ms
//! late, bimodally — sometimes fast, sometimes stalled.
//!
//! In a latency measurement tool that is not a performance nit. The engine's
//! latency probe is `GET /__down?bytes=0`, whose response is exactly the kind
//! of small write that trips this, so the stall is reported to the user as
//! network latency. It is a lie about the one thing the tool exists to
//! measure.
//!
//! Measured on the deployed guest, 25 idle probes each, before the fix:
//!
//! | endpoint                       | mean     | stalled |
//! |--------------------------------|----------|---------|
//! | `http://…:8080/api/health`     |   0.6 ms |    0/25 |
//! | `https://…/api/health`         |  41.9 ms |   24/25 |
//! | `https://…/__down?bytes=0`     |  38.4 ms |   22/25 |
//!
//! Plain HTTP was fine only by luck — hyper happened to emit those small
//! responses as a single write. The TLS path split them, so `axum-server`'s
//! `DefaultAcceptor`, which passes the socket through untouched, left Nagle
//! to do this. Both listeners are pinned here rather than only the one that
//! was observed failing.
//!
//! **This is invisible on loopback**, where the round trip is short enough
//! that the acknowledgement never gets delayed. Every test tier ran against
//! `127.0.0.1` and all of them passed throughout. The guard is therefore the
//! type signature of [`tls_acceptor`]: dropping the `NoDelayAcceptor` changes
//! its return type and the build fails.

use axum_server::accept::NoDelayAcceptor;
use axum_server::tls_rustls::{RustlsAcceptor, RustlsConfig};
use tokio::net::TcpStream;

/// Disables Nagle on an accepted socket. Pass to `Listener::tap_io`.
///
/// A socket that refuses the option is logged and served anyway: a working
/// connection with a possible 40 ms stall beats a refused one.
pub fn set_nodelay(stream: &mut TcpStream) {
    if let Err(e) = stream.set_nodelay(true) {
        tracing::warn!("could not set TCP_NODELAY on an accepted socket: {e}");
    }
}

/// The TLS acceptor, with Nagle disabled underneath it.
///
/// The return type is deliberately concrete. `axum_server::bind_rustls` gives
/// a `RustlsAcceptor<DefaultAcceptor>`, which is the version that shipped the
/// bug; naming `NoDelayAcceptor` here means the mistake cannot be reintroduced
/// without a compile error, which matters because no test on loopback can
/// observe the difference.
pub fn tls_acceptor(config: RustlsConfig) -> RustlsAcceptor<NoDelayAcceptor> {
    RustlsAcceptor::new(config).acceptor(NoDelayAcceptor::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::serve::{Listener, ListenerExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn accepted_sockets_have_nagle_disabled() {
        // Asserts the option on a real accepted socket rather than that the
        // call was written: the property is what the peer experiences.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut wrapped = listener.tap_io(set_nodelay);

        let client = tokio::spawn(async move { TcpStream::connect(addr).await.unwrap() });
        let (accepted, _peer) = wrapped.accept().await;

        assert!(
            accepted.nodelay().unwrap(),
            "TCP_NODELAY must be set on accepted sockets, or small responses \
             can stall ~40 ms waiting for a delayed ACK"
        );
        drop(client.await.unwrap());
    }

    #[tokio::test]
    async fn a_plain_listener_does_not_get_it_for_free() {
        // The control for the test above. Without the wrapper the option is
        // off, which is exactly how the TLS listener shipped — so this proves
        // the wrapper is what makes the difference, not the platform default.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let client = tokio::spawn(async move { TcpStream::connect(addr).await.unwrap() });
        let (accepted, _peer) = listener.accept().await.unwrap();

        assert!(!accepted.nodelay().unwrap());
        drop(client.await.unwrap());
    }
}
