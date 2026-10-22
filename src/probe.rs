//! ICMP probe backend.

use anyhow::{Context, Result};
use std::net::IpAddr;
use std::time::Duration;
use surge_ping::{Client, Config, ICMP, PingIdentifier, PingSequence};

/// Sends ICMP echo requests to one host and reports whether a reply arrived.
pub struct Prober {
    /// Kept alive on purpose: dropping the client closes the socket and stops
    /// the receive task, after which every probe would silently time out.
    _client: Client,
    pinger: surge_ping::Pinger,
}

impl Prober {
    pub async fn new(target: IpAddr, timeout: Duration) -> Result<Prober> {
        let config = match target {
            IpAddr::V4(_) => Config::default(),
            IpAddr::V6(_) => Config::builder().kind(ICMP::V6).build(),
        };
        let client = Client::new(&config).with_context(|| {
            format!(
                "failed to open an ICMP socket for {target} \
                 (on Linux/macOS this needs root or CAP_NET_RAW)"
            )
        })?;

        let ident = PingIdentifier((std::process::id() & 0xffff) as u16);
        let mut pinger = client.pinger(target, ident).await;
        pinger.timeout(timeout);

        Ok(Prober {
            _client: client,
            pinger,
        })
    }

    /// Send one probe. `Ok(Some(rtt))` means a reply came back, `Ok(None)` means
    /// it timed out (a normal, expected outcome when the path is broken), and
    /// `Err` means the probe could not be performed at all.
    pub async fn probe(&mut self, seq: u16, payload: &[u8]) -> Result<Option<Duration>> {
        match self.pinger.ping(PingSequence(seq), payload).await {
            Ok((_packet, rtt)) => Ok(Some(rtt)),
            Err(surge_ping::SurgeError::Timeout { .. }) => Ok(None),
            Err(err) => Err(err).context("probe failed"),
        }
    }
}
