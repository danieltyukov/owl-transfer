//! Discovery: a UDP broadcast every two seconds saying who we are and which
//! port we listen on, and a receive loop reporting who else said so.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use if_addrs::IfAddr;
use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, trace};

use crate::clock::now_ms;
use crate::config::DeviceKind;

const INTERVAL: Duration = Duration::from_secs(2);
/// A device not heard for this long is no longer nearby.
pub const EXPIRY_MS: i64 = 10_000;
const PACKET_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Advertisement {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    /// The TCP port peers should dial.
    pub port: u16,
}

#[derive(Serialize, Deserialize)]
struct Packet {
    v: u32,
    id: String,
    name: String,
    kind: DeviceKind,
    port: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heard {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    /// The packet's source address with the port from the payload.
    pub addr: SocketAddr,
    pub at_ms: i64,
}

pub struct Beacon {
    me: Arc<Mutex<Advertisement>>,
    port: u16,
    send_task: JoinHandle<()>,
    recv_task: JoinHandle<()>,
}

impl Drop for Beacon {
    fn drop(&mut self) {
        self.send_task.abort();
        self.recv_task.abort();
    }
}

impl Beacon {
    /// Binds `0.0.0.0:port` with address reuse so several instances on one
    /// machine all hear broadcasts, and sends to the limited broadcast
    /// address, every interface's broadcast address and `extra_targets`
    /// (unicast addresses, used by tests where no broadcast route exists).
    pub fn start(
        port: u16,
        me: Advertisement,
        extra_targets: Vec<SocketAddr>,
        tx: mpsc::Sender<Heard>,
    ) -> Result<Beacon> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
            .context("creating the beacon socket")?;
        socket.set_reuse_address(true)?;
        socket.set_broadcast(true)?;
        socket.set_nonblocking(true)?;
        socket
            .bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)).into())
            .with_context(|| format!("binding the beacon to UDP port {port}"))?;
        let std_socket: std::net::UdpSocket = socket.into();
        let socket = Arc::new(UdpSocket::from_std(std_socket)?);
        let bound_port = socket.local_addr()?.port();
        let target_port = if port == 0 { bound_port } else { port };

        let me = Arc::new(Mutex::new(me));
        let send_task = tokio::spawn(send_loop(
            socket.clone(),
            me.clone(),
            target_port,
            extra_targets,
        ));
        let recv_task = tokio::spawn(recv_loop(socket, me.clone(), tx));
        Ok(Beacon {
            me,
            port: bound_port,
            send_task,
            recv_task,
        })
    }

    pub fn update(&self, me: Advertisement) {
        *self.me.lock().expect("beacon advertisement lock") = me;
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

async fn send_loop(
    socket: Arc<UdpSocket>,
    me: Arc<Mutex<Advertisement>>,
    port: u16,
    extra_targets: Vec<SocketAddr>,
) {
    let mut tick = tokio::time::interval(INTERVAL);
    loop {
        tick.tick().await;
        let ad = me.lock().expect("beacon advertisement lock").clone();
        let packet = serde_json::to_vec(&Packet {
            v: PACKET_VERSION,
            id: ad.id,
            name: ad.name,
            kind: ad.kind,
            port: ad.port,
        })
        .expect("beacon packet serialises");

        let mut targets: Vec<SocketAddr> = vec![SocketAddr::from((Ipv4Addr::BROADCAST, port))];
        targets.extend(
            interface_broadcasts()
                .into_iter()
                .map(|ip| SocketAddr::from((ip, port))),
        );
        targets.extend(extra_targets.iter().copied());
        targets.sort();
        targets.dedup();
        for target in targets {
            if let Err(e) = socket.send_to(&packet, target).await {
                // No route to a broadcast address is normal on a machine
                // without a network; the next tick tries again.
                trace!("beacon to {target}: {e}");
            }
        }
    }
}

fn interface_broadcasts() -> Vec<Ipv4Addr> {
    match if_addrs::get_if_addrs() {
        Ok(interfaces) => interfaces
            .into_iter()
            .filter(|i| !i.is_loopback())
            .filter_map(|i| match i.addr {
                IfAddr::V4(v4) => v4.broadcast,
                IfAddr::V6(_) => None,
            })
            .collect(),
        Err(e) => {
            debug!("listing interfaces: {e}");
            Vec::new()
        }
    }
}

async fn recv_loop(socket: Arc<UdpSocket>, me: Arc<Mutex<Advertisement>>, tx: mpsc::Sender<Heard>) {
    let mut buf = vec![0u8; 2048];
    loop {
        let (n, from) = match socket.recv_from(&mut buf).await {
            Ok(x) => x,
            Err(e) => {
                debug!("beacon receive: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let Ok(packet) = serde_json::from_slice::<Packet>(&buf[..n]) else {
            continue;
        };
        if packet.v != PACKET_VERSION {
            continue;
        }
        if packet.id == me.lock().expect("beacon advertisement lock").id {
            continue;
        }
        let heard = Heard {
            id: packet.id,
            name: packet.name,
            kind: packet.kind,
            addr: SocketAddr::new(from.ip(), packet.port),
            at_ms: now_ms(),
        };
        if tx.send(heard).await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn free_udp_port() -> u16 {
        std::net::UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn ad(id: &str, port: u16) -> Advertisement {
        Advertisement {
            id: id.into(),
            name: format!("Device {id}"),
            kind: DeviceKind::Desktop,
            port,
        }
    }

    #[tokio::test]
    async fn two_beacons_on_loopback_hear_each_other() {
        let port_a = free_udp_port();
        let port_b = free_udp_port();
        let loopback = Ipv4Addr::LOCALHOST;
        let (tx_a, mut rx_a) = mpsc::channel(16);
        let (tx_b, mut rx_b) = mpsc::channel(16);
        let _a = Beacon::start(
            port_a,
            ad("aaa", 1111),
            vec![SocketAddr::from((loopback, port_b))],
            tx_a,
        )
        .unwrap();
        let _b = Beacon::start(
            port_b,
            ad("bbb", 2222),
            vec![SocketAddr::from((loopback, port_a))],
            tx_b,
        )
        .unwrap();

        let heard_by_a = tokio::time::timeout(Duration::from_secs(5), rx_a.recv())
            .await
            .expect("a hears something within five seconds")
            .unwrap();
        assert_eq!(heard_by_a.id, "bbb");
        assert_eq!(heard_by_a.name, "Device bbb");
        assert_eq!(heard_by_a.addr, SocketAddr::from((loopback, 2222)));

        let heard_by_b = tokio::time::timeout(Duration::from_secs(5), rx_b.recv())
            .await
            .expect("b hears something within five seconds")
            .unwrap();
        assert_eq!(heard_by_b.id, "aaa");
        assert_eq!(heard_by_b.addr, SocketAddr::from((loopback, 1111)));
    }
}
