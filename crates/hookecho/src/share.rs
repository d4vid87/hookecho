//! Position sharing between Hook Echo instances — the phone out chasing, the desktop at home.
//!
//! Two transports, both optional and independent:
//!   * **LAN** — a UDP broadcast on [`PORT`]. Zero configuration, but only reaches devices on the
//!     same network, so it covers "laptop and phone on the house Wi-Fi", not a chase on cellular.
//!   * **Relay** — an HTTP endpoint the user hosts: `POST` your fix, `GET` everyone's. Works
//!     anywhere the phone has data. The endpoint is trusted with your live position, which is why
//!     there is no default one.
//!
//! Both carry the same [`Peer`] JSON, and both feed one channel, so the app treats them alike.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::UdpSocket;
use std::sync::mpsc::{self, Receiver, Sender};

/// Broadcast port. Arbitrary, unassigned, and shared by every Hook Echo on the network.
pub const PORT: u16 = 41777;
/// Drop a peer whose last fix is older than this — a stale dot is worse than no dot.
pub const STALE_SECS: i64 = 300;

/// One shared position. `id` identifies the *device* (a random per-run string) so a peer that
/// renames itself doesn't fork into two dots, and so we can recognise and ignore our own packets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Peer {
    pub id: String,
    pub name: String,
    pub lon: f64,
    pub lat: f64,
    /// Unix seconds when the fix was taken.
    pub ts: i64,
    /// Optional live-stream URL this chaser is broadcasting (empty = none). Old peers that
    /// predate the field simply send nothing.
    #[serde(default)]
    pub video_url: String,
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Drop peers older than [`STALE_SECS`].
pub fn prune(peers: &mut HashMap<String, Peer>, now: i64) {
    peers.retain(|_, p| now - p.ts <= STALE_SECS);
}

/// A running share session: one UDP socket for send + receive, plus the channel every transport
/// delivers into. Dropping it stops broadcasting; the listener thread ends with the process.
pub struct Share {
    /// This run's device id. Packets carrying it are our own and are ignored.
    pub id: String,
    sock: Option<UdpSocket>,
    tx: Sender<Peer>,
    rx: Receiver<Peer>,
}

impl Share {
    /// Bind the broadcast socket and start listening. A bind failure (port taken, no permission)
    /// is not fatal: the relay transport and the rest of the app carry on without LAN.
    pub fn start() -> Self {
        // Android filters out broadcast packets unless a Wi-Fi multicast lock is held.
        crate::platform::hold_multicast_lock();
        let id = format!("{:x}", now() as u64 ^ u64::from(std::process::id()));
        let (tx, rx) = mpsc::channel();
        let sock = bind().inspect(|s| {
            if let Ok(s2) = s.try_clone() {
                let tx2 = tx.clone();
                std::thread::spawn(move || listen(s2, tx2));
            }
        });
        if sock.is_none() {
            log::warn!("position sharing: UDP bind on :{PORT} failed, LAN transport off");
        }
        Self { id, sock, tx, rx }
    }

    /// A sender for other transports (the relay poller) to deliver peers on.
    pub fn sender(&self) -> Sender<Peer> {
        self.tx.clone()
    }

    /// Our own fix, stamped with this session's id. `video_url` is the stream the user chose to
    /// publish alongside their dot (empty = none).
    pub fn me(&self, name: &str, lon: f64, lat: f64, video_url: &str) -> Peer {
        Peer {
            id: self.id.clone(),
            name: name.to_string(),
            lon,
            lat,
            ts: now(),
            video_url: video_url.to_string(),
        }
    }

    /// Push one fix onto the LAN. No-op when the socket never bound.
    pub fn broadcast(&self, me: &Peer) {
        let (Some(sock), Ok(json)) = (&self.sock, serde_json::to_vec(me)) else {
            return;
        };
        // 255.255.255.255 alone is not enough: on a host with several interfaces (docker bridges,
        // a VM bridge, a VPN) the kernel routes it out whichever one it likes, which on this
        // machine was not the Wi-Fi the phone is on. Also aim at the local subnet's own broadcast
        // address, which is unambiguous.
        // ponytail: assumes a /24 for that second target — true of home LANs; the general fix is
        // a getifaddrs dependency, worth it only if someone reports a /16.
        for addr in [subnet_broadcast(), Some("255.255.255.255".to_string())]
            .into_iter()
            .flatten()
        {
            if let Err(e) = sock.send_to(&json, (addr.as_str(), PORT)) {
                log::debug!("position broadcast to {addr} failed: {e}");
            }
        }
    }

    /// Fold everything received since the last call into `peers` (newest wins), then drop the
    /// stale ones. Returns true if anything changed, so the caller can repaint.
    pub fn drain(&self, peers: &mut HashMap<String, Peer>) -> bool {
        let mut changed = false;
        while let Ok(p) = self.rx.try_recv() {
            if p.id == self.id {
                continue; // our own broadcast, echoed back by the network
            }
            if peers.get(&p.id).is_none_or(|old| old.ts <= p.ts) {
                peers.insert(p.id.clone(), p);
                changed = true;
            }
        }
        let before = peers.len();
        prune(peers, now());
        changed || peers.len() != before
    }
}

/// This host's subnet broadcast address, e.g. `192.168.1.255`. Found by asking the routing table
/// which local address would be used to reach the internet — connecting a UDP socket sends no
/// packets — then replacing the last octet.
fn subnet_broadcast() -> Option<String> {
    let probe = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    probe.connect(("1.1.1.1", 53)).ok()?;
    let ip = match probe.local_addr().ok()? {
        std::net::SocketAddr::V4(a) => *a.ip(),
        std::net::SocketAddr::V6(_) => return None,
    };
    let [a, b, c, _] = ip.octets();
    Some(format!("{a}.{b}.{c}.255"))
}

fn bind() -> Option<UdpSocket> {
    let sock = UdpSocket::bind(("0.0.0.0", PORT)).ok()?;
    sock.set_broadcast(true).ok()?;
    Some(sock)
}

fn listen(sock: UdpSocket, tx: Sender<Peer>) {
    let mut buf = [0u8; 2048];
    loop {
        let Ok((n, _from)) = sock.recv_from(&mut buf) else {
            return;
        };
        match serde_json::from_slice::<Peer>(&buf[..n]) {
            Ok(p) => {
                if tx.send(p).is_err() {
                    return; // app dropped the session
                }
            }
            // Some other program on the port, or a truncated packet. Ignore it.
            Err(e) => log::debug!("share packet ignored: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(id: &str, ts: i64) -> Peer {
        Peer {
            id: id.into(),
            name: "chaser".into(),
            lon: -97.5,
            lat: 35.4,
            ts,
            video_url: String::new(),
        }
    }

    #[test]
    fn subnet_broadcast_ends_in_255() {
        // Skipped rather than failed on a machine with no route out.
        if let Some(b) = subnet_broadcast() {
            assert!(b.ends_with(".255"), "{b}");
            assert_eq!(b.split('.').count(), 4);
        }
    }

    #[test]
    fn peer_json_roundtrips() {
        let p = peer("abc", 1_700_000_000);
        let back: Peer = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn prune_drops_only_stale() {
        let now = 1_000_000;
        let mut peers = HashMap::from([
            ("fresh".to_string(), peer("fresh", now - 10)),
            ("old".to_string(), peer("old", now - STALE_SECS - 1)),
        ]);
        prune(&mut peers, now);
        assert_eq!(peers.keys().collect::<Vec<_>>(), vec!["fresh"]);
    }

    #[test]
    fn drain_ignores_own_packets_and_keeps_newest() {
        let s = Share::start();
        let tx = s.sender();
        tx.send(Peer {
            id: s.id.clone(),
            ..peer("x", now())
        })
        .unwrap();
        tx.send(peer("other", now() - 60)).unwrap();
        tx.send(Peer {
            lat: 40.0,
            ..peer("other", now())
        })
        .unwrap();
        let mut peers = HashMap::new();
        assert!(s.drain(&mut peers));
        assert_eq!(peers.len(), 1);
        assert_eq!(peers["other"].lat, 40.0);
    }
}
