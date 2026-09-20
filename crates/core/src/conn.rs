//! One authenticated connection to a peer: TLS, a `Hello` in each direction,
//! then a reader task feeding frames to the engine and a writer task
//! draining frames the engine sends.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use rustls_pki_types::ServerName;
use tokio::io::{AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch, OwnedSemaphorePermit, Semaphore};
use tokio::time::timeout;
use tokio_rustls::{TlsAcceptor, TlsConnector, TlsStream};
use tracing::debug;

use crate::clock::now_ms;
use crate::config::DeviceKind;
use crate::identity::Identity;
use crate::proto::{encode, Control, Frame, FrameReader, BLOCK_WINDOW, PROTOCOL_VERSION};
use crate::tls::{self, Trusted};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const INBOUND_QUEUE: usize = 64;

#[derive(Clone)]
pub struct Connection {
    /// The TLS-authenticated fingerprint of the other side.
    pub peer_id: String,
    pub peer_name: String,
    pub peer_kind: DeviceKind,
    pub addr: SocketAddr,
    /// Whether this side dialled.
    pub outbound: bool,
    /// Whether `peer_id` was a paired peer when the handshake finished.
    pub trusted: bool,
    /// Control frames: unbounded, so the engine never blocks on a slow
    /// peer while holding its lock, and never starved by blocks.
    tx: mpsc::UnboundedSender<Frame>,
    /// Blocks: bounded, so a peer that requests and never reads stalls its
    /// own requests rather than growing our memory.
    block_tx: mpsc::Sender<Frame>,
    /// Requests being served for this peer, at most `BLOCK_WINDOW`.
    serve_permits: Arc<Semaphore>,
    closer: Arc<watch::Sender<bool>>,
    last_rx_ms: Arc<AtomicI64>,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("peer_id", &self.peer_id)
            .field("addr", &self.addr)
            .field("outbound", &self.outbound)
            .field("trusted", &self.trusted)
            .finish()
    }
}

impl Connection {
    /// Queues a frame. A control frame is queued at once; a block waits
    /// for room in the bounded block queue.
    pub async fn send(&self, frame: Frame) -> Result<()> {
        match frame {
            Frame::Block { .. } => self
                .block_tx
                .send(frame)
                .await
                .map_err(|_| anyhow!("connection to {} is closed", self.peer_id)),
            Frame::Control(_) => self.try_send(frame),
        }
    }

    /// Queues a frame without waiting. A block is refused when its queue
    /// is full.
    pub fn try_send(&self, frame: Frame) -> Result<()> {
        match frame {
            Frame::Block { .. } => self
                .block_tx
                .try_send(frame)
                .map_err(|_| anyhow!("block queue to {} is full or closed", self.peer_id)),
            Frame::Control(_) => self
                .tx
                .send(frame)
                .map_err(|_| anyhow!("connection to {} is closed", self.peer_id)),
        }
    }

    /// A permit to serve one block request, or `None` when the peer
    /// already has `BLOCK_WINDOW` being served.
    pub fn serve_permit(&self) -> Option<OwnedSemaphorePermit> {
        self.serve_permits.clone().try_acquire_owned().ok()
    }

    pub fn close(&self) {
        let _ = self.closer.send(true);
    }

    pub fn is_closed(&self) -> bool {
        *self.closer.borrow() || self.tx.is_closed()
    }

    /// When the last frame arrived from the peer.
    pub fn last_rx_ms(&self) -> i64 {
        self.last_rx_ms.load(Ordering::Relaxed)
    }
}

/// Accepts an incoming TCP connection: TLS, send `hello`, read the peer's.
pub async fn accept(
    identity: &Identity,
    trusted: Trusted,
    tcp: TcpStream,
    hello: Control,
) -> Result<(Connection, mpsc::Receiver<Frame>)> {
    let addr = tcp.peer_addr().context("peer address")?;
    let _ = tcp.set_nodelay(true);
    let acceptor = TlsAcceptor::from(tls::server_config(identity, trusted.clone())?);
    let stream = timeout(HANDSHAKE_TIMEOUT, acceptor.accept(tcp))
        .await
        .context("TLS accept timed out")?
        .context("TLS accept failed")?;
    establish(TlsStream::Server(stream), addr, false, trusted, hello).await
}

/// Dials a peer: TCP, TLS, send `hello`, read the peer's.
pub async fn dial(
    identity: &Identity,
    trusted: Trusted,
    addr: SocketAddr,
    hello: Control,
) -> Result<(Connection, mpsc::Receiver<Frame>)> {
    let tcp = timeout(HANDSHAKE_TIMEOUT, TcpStream::connect(addr))
        .await
        .with_context(|| format!("connecting to {addr} timed out"))?
        .with_context(|| format!("connecting to {addr}"))?;
    let _ = tcp.set_nodelay(true);
    let connector = TlsConnector::from(tls::client_config(identity, trusted.clone())?);
    let name = ServerName::try_from(tls::SERVER_NAME).expect("a valid DNS name");
    let stream = timeout(HANDSHAKE_TIMEOUT, connector.connect(name, tcp))
        .await
        .context("TLS connect timed out")?
        .context("TLS connect failed")?;
    establish(TlsStream::Client(stream), addr, true, trusted, hello).await
}

async fn establish(
    stream: TlsStream<TcpStream>,
    addr: SocketAddr,
    outbound: bool,
    trusted: Trusted,
    hello: Control,
) -> Result<(Connection, mpsc::Receiver<Frame>)> {
    let fp = tls::peer_fingerprint(&stream).context("peer presented no certificate")?;
    let is_trusted = trusted(&fp);
    let (rd, mut wr) = tokio::io::split(stream);
    wr.write_all(&encode(&Frame::Control(hello)))
        .await
        .context("sending hello")?;
    let mut reader = FrameReader::new(rd);
    let first = timeout(HANDSHAKE_TIMEOUT, reader.next())
        .await
        .context("waiting for hello timed out")??;
    let (peer_id, peer_name, peer_kind) = match first {
        Some(Frame::Control(Control::Hello {
            id,
            name,
            kind,
            version,
        })) => {
            if version != PROTOCOL_VERSION {
                bail!("peer speaks protocol {version}, this is {PROTOCOL_VERSION}");
            }
            if id != fp {
                bail!("peer claims id {id} but presented certificate {fp}");
            }
            (id, name, kind)
        }
        Some(other) => bail!("expected hello, got {other:?}"),
        None => bail!("peer closed the connection before hello"),
    };

    let (in_tx, in_rx) = mpsc::channel(INBOUND_QUEUE);
    let (out_tx, out_rx) = mpsc::unbounded_channel();
    let (block_tx, block_rx) = mpsc::channel(BLOCK_WINDOW);
    let closer = Arc::new(watch::channel(false).0);
    let last_rx_ms = Arc::new(AtomicI64::new(now_ms()));
    tokio::spawn(read_loop(reader, in_tx, closer.clone(), last_rx_ms.clone()));
    tokio::spawn(write_loop(wr, out_rx, block_rx, closer.clone()));

    Ok((
        Connection {
            peer_id,
            peer_name,
            peer_kind,
            addr,
            outbound,
            trusted: is_trusted,
            tx: out_tx,
            block_tx,
            serve_permits: Arc::new(Semaphore::new(BLOCK_WINDOW)),
            closer,
            last_rx_ms,
        },
        in_rx,
    ))
}

/// Resolves once the connection has been closed from either task or by
/// `Connection::close`. Holds no guard across the await so the tasks stay
/// `Send`.
async fn wait_closed(rx: &mut watch::Receiver<bool>) {
    loop {
        if *rx.borrow_and_update() {
            return;
        }
        if rx.changed().await.is_err() {
            return;
        }
    }
}

async fn read_loop(
    mut reader: FrameReader<ReadHalf<TlsStream<TcpStream>>>,
    in_tx: mpsc::Sender<Frame>,
    closer: Arc<watch::Sender<bool>>,
    last_rx_ms: Arc<AtomicI64>,
) {
    let mut closed = closer.subscribe();
    loop {
        tokio::select! {
            _ = wait_closed(&mut closed) => break,
            res = reader.next() => match res {
                Ok(Some(frame)) => {
                    last_rx_ms.store(now_ms(), Ordering::Relaxed);
                    if in_tx.send(frame).await.is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    debug!("connection read ended: {e}");
                    break;
                }
            }
        }
    }
    let _ = closer.send(true);
}

async fn write_loop(
    mut wr: WriteHalf<TlsStream<TcpStream>>,
    mut out_rx: mpsc::UnboundedReceiver<Frame>,
    mut block_rx: mpsc::Receiver<Frame>,
    closer: Arc<watch::Sender<bool>>,
) {
    let mut closed = closer.subscribe();
    loop {
        // Control frames first, so a peer streaming blocks still gets
        // pings, index updates and rejects promptly; then a close is
        // honoured only once nothing is waiting.
        match out_rx.try_recv() {
            Ok(frame) => {
                if let Err(e) = wr.write_all(&encode(&frame)).await {
                    debug!("connection write ended: {e}");
                    break;
                }
                continue;
            }
            Err(mpsc::error::TryRecvError::Empty) => {}
            Err(mpsc::error::TryRecvError::Disconnected) => break,
        }
        tokio::select! {
            biased;
            next = out_rx.recv() => match next {
                Some(frame) => {
                    if let Err(e) = wr.write_all(&encode(&frame)).await {
                        debug!("connection write ended: {e}");
                        break;
                    }
                }
                None => break,
            },
            next = block_rx.recv() => match next {
                Some(frame) => {
                    if let Err(e) = wr.write_all(&encode(&frame)).await {
                        debug!("connection write ended: {e}");
                        break;
                    }
                }
                None => break,
            },
            _ = wait_closed(&mut closed) => break,
        }
    }
    while let Ok(frame) = out_rx.try_recv() {
        if wr.write_all(&encode(&frame)).await.is_err() {
            break;
        }
    }
    let _ = wr.shutdown().await;
    let _ = closer.send(true);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    fn hello(identity: &Identity, name: &str) -> Control {
        Control::Hello {
            id: identity.id.clone(),
            name: name.into(),
            kind: DeviceKind::Desktop,
            version: PROTOCOL_VERSION,
        }
    }

    #[tokio::test]
    async fn hello_is_exchanged() {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let a = Identity::load_or_create(dir_a.path()).unwrap();
        let b = Identity::load_or_create(dir_b.path()).unwrap();
        let b_id = b.id.clone();
        let trusts_b: Trusted = Arc::new(move |fp| fp == b_id);
        let trusts_nobody: Trusted = Arc::new(|_| false);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = {
            let a = a.clone();
            tokio::spawn(async move {
                let (tcp, _) = listener.accept().await.unwrap();
                accept(&a, trusts_b, tcp, hello(&a, "A")).await.unwrap()
            })
        };
        let (conn_b, mut rx_b) = dial(&b, trusts_nobody, addr, hello(&b, "B")).await.unwrap();
        let (conn_a, mut rx_a) = server.await.unwrap();

        assert_eq!(conn_a.peer_id, b.id);
        assert_eq!(conn_a.peer_name, "B");
        assert!(conn_a.trusted);
        assert!(!conn_a.outbound);
        assert_eq!(conn_b.peer_id, a.id);
        assert_eq!(conn_b.peer_name, "A");
        assert!(!conn_b.trusted);
        assert!(conn_b.outbound);

        conn_a.send(Frame::Control(Control::Ping)).await.unwrap();
        assert_eq!(
            timeout(Duration::from_secs(2), rx_b.recv()).await.unwrap(),
            Some(Frame::Control(Control::Ping))
        );
        conn_b.send(Frame::Control(Control::Pong)).await.unwrap();
        assert_eq!(
            timeout(Duration::from_secs(2), rx_a.recv()).await.unwrap(),
            Some(Frame::Control(Control::Pong))
        );

        // Serving is bounded per connection.
        let permits: Vec<_> = (0..BLOCK_WINDOW)
            .map(|_| conn_a.serve_permit().expect("a permit within the window"))
            .collect();
        assert!(conn_a.serve_permit().is_none());
        drop(permits);
        assert!(conn_a.serve_permit().is_some());

        conn_a.close();
        assert_eq!(
            timeout(Duration::from_secs(2), rx_b.recv()).await.unwrap(),
            None
        );
        assert!(conn_b.send(Frame::Control(Control::Ping)).await.is_ok() || conn_b.is_closed());
    }
}
