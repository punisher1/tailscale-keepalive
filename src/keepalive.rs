//! The keepalive loop.

use anyhow::Result;
use tokio::sync::watch;

use crate::probe::Prober;
use crate::settings::Settings;
use crate::state::{LinkState, ProbeOutcome, Reaction};
use crate::{debug, info, warn};

/// Payload size. With the 8-byte ICMP header this makes a 64-byte echo, the
/// same as `ping`'s default — small enough to ignore, large enough that the
/// packet is not degenerate.
const PAYLOAD_LEN: usize = 56;

/// How often (in probes) to log a running summary.
const STATS_EVERY: u64 = 100;

/// Run the loop until `shutdown` flips to `true`.
pub async fn run(settings: &Settings, mut shutdown: watch::Receiver<bool>) -> Result<()> {
    info!("starting: {}", settings.summary());

    let mut prober = Prober::new(settings.target, settings.timeout).await?;
    let payload = vec![0x5a_u8; PAYLOAD_LEN];
    let mut link = LinkState::new(settings.fail_threshold);
    let mut seq: u16 = 0;

    // Fire the first probe right away rather than waiting a full interval: the
    // mapping we are protecting may already be close to expiring.
    let mut ticker = tokio::time::interval(settings.interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    info!("shutdown requested");
                    break;
                }
            }
            _ = ticker.tick() => {
                if *shutdown.borrow() {
                    info!("shutdown requested");
                    break;
                }
                probe_once(&mut prober, &mut link, &mut seq, &payload, settings).await;
            }
        }
    }

    let (probes, failures) = link.totals();
    info!("stopped after {probes} probes ({failures} without a reply)");
    Ok(())
}

async fn probe_once(
    prober: &mut Prober,
    link: &mut LinkState,
    seq: &mut u16,
    payload: &[u8],
    settings: &Settings,
) {
    let started = std::time::Instant::now();
    let outcome = prober.probe(*seq, payload).await;
    *seq = seq.wrapping_add(1);

    let (state, detail) = match outcome {
        Ok(Some(rtt)) => (
            ProbeOutcome::Reply,
            format!("rtt={:.1}ms", rtt.as_secs_f64() * 1000.0),
        ),
        Ok(None) => (
            ProbeOutcome::Timeout,
            format!("no reply within {}s", settings.timeout.as_secs()),
        ),
        Err(err) => (ProbeOutcome::Timeout, format!("probe error: {err:#}")),
    };

    match link.record(state) {
        Reaction::Ok => {
            debug!(
                "probe ok target={} {detail} elapsed={:?}",
                settings.target,
                started.elapsed()
            );
        }
        Reaction::Recovered { failures } => {
            info!(
                "link recovered: target={} replied after {failures} failed probe(s)",
                settings.target
            );
        }
        Reaction::Quiet => {}
        Reaction::Down { failures } => {
            warn!(
                "link looks down: target={} has not replied to {failures} consecutive probes. \
                 The keepalive cannot fix a genuinely broken path — check `tailscale status`, \
                 `tailscale ping {}`, or the peer's power/network.",
                settings.target, settings.target
            );
        }
        Reaction::StillDown { failures } => {
            warn!(
                "still down: target={} has not replied to {failures} consecutive probes \
                 ({} total failures)",
                settings.target,
                link.totals().1
            );
        }
    }

    let (probes, failures) = link.totals();
    if probes % STATS_EVERY == 0 {
        info!(
            "stats: {probes} probes, {failures} without reply, {} consecutive",
            link.consecutive_failures()
        );
    }
}
