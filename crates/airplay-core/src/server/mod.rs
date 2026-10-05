// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded TCP control service.
//!
//! [`Server::run`] accepts control connections, frames each byte stream into
//! requests with [`crate::control::request_length`], routes every request
//! through a per-connection [`crate::control::ControlSession`] and the
//! [`RouteHandlers`] of that connection, and writes the response
//! [`crate::control::ControlResponse::to_bytes`] produces.
//!
//! # Bounds
//!
//! Every limit the service enforces is a documented port decision, because
//! upstream's control thread has none of them:
//!
//! * A request head is limited to [`crate::wire::DEFAULT_MAX_HEAD`] and a body
//!   to [`crate::control::MAX_CONTROL_BODY`]; both are checked before the bytes
//!   are buffered.
//! * At most [`ServerConfig::max_connections`] connections are served at once.
//!   Like upstream, the listener stops accepting while the limit is reached
//!   instead of rejecting a client, so further connections wait in the kernel
//!   backlog (`upstream/UxPlay/lib/httpd.c:389-402`).
//! * A connection that is idle for [`ServerConfig::idle_timeout`] is closed,
//!   and bytes that continue an unfinished request do not refresh that
//!   deadline. Upstream never closes an idle connection
//!   (`upstream/UxPlay/lib/httpd.c:359-618`), so a stalled client holds one of
//!   its 12 slots until the peer goes away.
//! * A request the service cannot frame is answered once
//!   ([`crate::control::ControlError::to_response`]) and the connection is
//!   closed, because the bytes after a rejected request cannot be trusted.
//!
//! # Documented differences from upstream
//!
//! * Upstream peeks the first eight bytes of every new request and treats
//!   `HTTP/1.1` and `EVENT/1.0` as the response on a *reverse* connection it
//!   opened itself (`upstream/UxPlay/lib/httpd.c:498-556`). The reverse
//!   connection belongs to the event channel work, so the port does not open or
//!   recognise one yet.
//! * Upstream answers a request whose route it does not implement with its
//!   default `200 OK`; the port keeps that for an unmatched URL but answers
//!   `501 Not Implemented` for a route the port has not ported
//!   (see [`crate::control::Route::is_ported`]).
//! * `TCP_NODELAY` is enabled on accepted connections to keep a control reply
//!   from waiting for the delayed-ack timer.

mod connection;
mod handlers;
mod pairing;

use std::io::{Error, ErrorKind, Result};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::time::Duration;

use tokio::net::TcpListener;
use tokio::sync::Semaphore;

use crate::control::RouteHandler;

pub use handlers::{
    AUDIO_OUTPUT_FORMATS, DEFAULT_FEATURES, DEFAULT_HEIGHT, DEFAULT_MAX_FPS, DEFAULT_MODEL,
    DEFAULT_PI, DEFAULT_REFRESH_RATE, DEFAULT_SOURCE_VERSION, DEFAULT_VV, DEFAULT_WIDTH,
    DISPLAY_FEATURES, DISPLAY_UUID, DeviceIdentity, RouteHandlers, STATUS_FLAGS,
};
pub use pairing::{
    MAX_PIN_FAILURES, OsRandom, PairingError, PairingState, PinMode, RandomError, RandomSource,
};

/// Connections upstream serves at the same time
/// (`upstream/UxPlay/lib/httpd.c:154`), which is the default of
/// [`ServerConfig::max_connections`].
pub const MAX_CONNECTIONS: usize = 12;

/// How long a connection may make no progress before it is closed, which is the
/// default of [`ServerConfig::idle_timeout`].
///
/// This is a port decision: upstream has no idle timeout, so the value is chosen
/// to be longer than any pause a client takes between control requests.
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// How a [`Server`] listens and how many clients it serves.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerConfig {
    /// Address to listen on.
    ///
    /// Port `0` binds an ephemeral port, which is what upstream does before it
    /// advertises the assigned port (`upstream/UxPlay/lib/raop.c:613-617`,
    /// `:834-837`); a receiver that is discovered over mDNS does not need a
    /// fixed one.
    pub bind: SocketAddr,
    /// Largest number of connections served at once, at least `1`.
    pub max_connections: usize,
    /// How long a connection may stop making progress before it is closed.
    pub idle_timeout: Duration,
    /// Identity the handlers advertise to clients.
    pub device: DeviceIdentity,
}

impl Default for ServerConfig {
    /// Listens on every interface with an ephemeral port, serves
    /// [`MAX_CONNECTIONS`] connections and closes an idle connection after
    /// [`DEFAULT_IDLE_TIMEOUT`].
    fn default() -> Self {
        Self {
            bind: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)),
            max_connections: MAX_CONNECTIONS,
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
            device: DeviceIdentity::default(),
        }
    }
}

/// A bound control service that has not started accepting yet.
///
/// Binding is separate from [`Server::run`] so a caller can learn the assigned
/// port with [`Server::local_addr`] before it advertises the service.
pub struct Server {
    listener: TcpListener,
    config: ServerConfig,
    slots: Arc<Semaphore>,
}

impl Server {
    /// Binds `config`.
    ///
    /// # Errors
    ///
    /// Returns the bind failure of the address, or [`ErrorKind::InvalidInput`]
    /// when `config.max_connections` is zero.
    pub async fn bind(config: ServerConfig) -> Result<Self> {
        if config.max_connections == 0 {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "max_connections must be at least 1",
            ));
        }
        let listener = TcpListener::bind(config.bind).await?;
        let slots = Arc::new(Semaphore::new(config.max_connections));
        Ok(Self {
            listener,
            config,
            slots,
        })
    }

    /// Address the service listens on, including the assigned port.
    ///
    /// # Errors
    ///
    /// Returns the error the operating system reports for the socket.
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Configuration this service was bound with.
    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    /// Accepts connections until the listener fails.
    ///
    /// `new_handlers` creates the handler set for each accepted connection, so a
    /// caller can hand every client its own [`RouteHandlers`].
    ///
    /// # Errors
    ///
    /// Returns the listener error. A connection that fails while it is served is
    /// not an error of the service: upstream logs it and keeps accepting
    /// (`upstream/UxPlay/lib/httpd.c:359-618`), and so does the port.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use airplay_core::server::{DeviceIdentity, Server, ServerConfig, RouteHandlers};
    ///
    /// # async fn serve() -> std::io::Result<()> {
    /// let config = ServerConfig {
    ///     device: DeviceIdentity::new("aa:bb:cc:dd:ee:ff", "Receiver", Vec::new()),
    ///     ..ServerConfig::default()
    /// };
    /// let server = Server::bind(config).await?;
    /// println!("control service on {}", server.local_addr()?);
    /// server.run(|| RouteHandlers::new(DeviceIdentity::default())).await
    /// # }
    /// ```
    pub async fn run<F, H>(self, mut new_handlers: F) -> Result<()>
    where
        F: FnMut() -> H,
        H: RouteHandler + Send + 'static,
    {
        let idle_timeout = self.config.idle_timeout;
        loop {
            // Taking a slot before accepting is what keeps the connection count
            // bounded: with every slot in use, the listener is not polled and a
            // new connection waits in the backlog.
            let Ok(slot) = Arc::clone(&self.slots).acquire_owned().await else {
                return Ok(());
            };
            let (socket, _peer) = match self.listener.accept().await {
                Ok(accepted) => accepted,
                // A connection can fail between the backlog and `accept`, which
                // says nothing about the next one.
                Err(error) if error.kind() == ErrorKind::ConnectionAborted => continue,
                Err(error) => return Err(error),
            };
            let handlers = new_handlers();
            tokio::spawn(async move {
                let _slot = slot;
                // Upstream logs this failure and drops the connection.
                let _ = connection::serve(socket, handlers, idle_timeout).await;
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bind_rejects_a_useless_connection_limit() {
        let config = ServerConfig {
            max_connections: 0,
            ..ServerConfig::default()
        };
        let error = Server::bind(config).await.err().unwrap();
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }

    #[tokio::test]
    async fn bind_reports_the_assigned_port() {
        let config = ServerConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            ..ServerConfig::default()
        };
        let server = Server::bind(config).await.unwrap();
        assert_ne!(server.local_addr().unwrap().port(), 0);
        assert_eq!(server.config().max_connections, MAX_CONNECTIONS);
    }
}
