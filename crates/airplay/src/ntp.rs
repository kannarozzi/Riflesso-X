//! AirPlay 1 timing: the receiver periodically queries the sender's timing
//! port with NTP-style packets (type 0x52 with the marker bit set).

use std::net::SocketAddr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

const INTERVAL: Duration = Duration::from_secs(3);
const SECONDS_1900_TO_1970: u64 = 2_208_988_800;

pub async fn start(remote: SocketAddr) -> std::io::Result<(u16, JoinHandle<()>)> {
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    let port = socket.local_addr()?.port();
    let task = tokio::spawn(async move {
        let mut buf = [0u8; 128];
        loop {
            let mut request = [0u8; 32];
            request[..4].copy_from_slice(&[0x80, 0xD2, 0x00, 0x07]);
            let sent = ntp_now();
            request[24..32].copy_from_slice(&sent.to_be_bytes());
            if let Err(e) = socket.send_to(&request, remote).await {
                tracing::debug!("timing request to {remote} failed: {e}");
            }

            let deadline = tokio::time::sleep(INTERVAL);
            tokio::pin!(deadline);
            loop {
                tokio::select! {
                    _ = &mut deadline => break,
                    received = socket.recv_from(&mut buf) => {
                        let Ok((n, _)) = received else { break };
                        if n >= 32 && buf[1] & 0x7F == 0x53 {
                            let t4 = ntp_now();
                            let t2 = u64::from_be_bytes(buf[16..24].try_into().unwrap());
                            let t3 = u64::from_be_bytes(buf[24..32].try_into().unwrap());
                            let offset = (t2 as i128 - sent as i128 + t3 as i128 - t4 as i128) / 2;
                            tracing::trace!("clock offset to sender: {:.3} ms", offset as f64 * 1000.0 / 4_294_967_296.0);
                        }
                    }
                }
            }
        }
    });
    Ok((port, task))
}

/// Current time as a 64-bit NTP timestamp (32.32 fixed point since 1900).
fn ntp_now() -> u64 {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = now.as_secs() + SECONDS_1900_TO_1970;
    let frac = (u64::from(now.subsec_nanos()) << 32) / 1_000_000_000;
    (secs << 32) | frac
}
