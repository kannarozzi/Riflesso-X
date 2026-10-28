//! Bonjour advertisement of the `_airplay._tcp` and `_raop._tcp` services.

use anyhow::{Context, Result};
use mdns_sd::{IfKind, ServiceDaemon, ServiceInfo};

use crate::auth::Security;
use crate::consts;
use crate::identity::Identity;

pub struct TxtRecords {
    pub airplay: Vec<(&'static str, String)>,
    pub raop: Vec<(&'static str, String)>,
}

impl TxtRecords {
    /// `pw` tells senders a password or code is needed; `sf` which kind.
    pub fn new(identity: &Identity, features: u64, security: &Security) -> Self {
        let pw = security.required().to_string();
        let pk = hex::encode(identity.public_key());
        let features = consts::features_txt(features);
        let airplay = vec![
            ("deviceid", identity.device_id_string()),
            ("features", features.clone()),
            ("pw", pw.clone()),
            ("flags", "0x4".into()),
            ("model", consts::MODEL.into()),
            ("pk", pk.clone()),
            ("pi", consts::PAIRING_IDENTITY.into()),
            ("srcvers", consts::SOURCE_VERSION.into()),
            ("vv", "2".into()),
        ];
        let raop = vec![
            ("ch", "2".into()),
            ("cn", "0,1,2,3".into()),
            ("da", "true".into()),
            ("et", "0,3,5".into()),
            ("vv", "2".into()),
            ("ft", features),
            ("am", consts::MODEL.into()),
            ("md", "0,1,2".into()),
            ("rhd", "5.6.0.0".into()),
            ("pw", pw),
            ("sf", security.status_flags().into()),
            ("sr", "44100".into()),
            ("ss", "16".into()),
            ("sv", "false".into()),
            ("tp", "UDP".into()),
            ("txtvers", "1".into()),
            ("vs", consts::SOURCE_VERSION.into()),
            ("vn", "65537".into()),
            ("pk", pk),
        ];
        Self { airplay, raop }
    }

    /// Raw DNS TXT wire format, returned to senders that query /info.
    pub fn encode(entries: &[(&'static str, String)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (key, value) in entries {
            let item = format!("{key}={value}");
            let len = item.len().min(255);
            out.push(len as u8);
            out.extend_from_slice(&item.as_bytes()[..len]);
        }
        out
    }
}

pub struct Advertiser {
    daemon: ServiceDaemon,
    fullnames: Vec<String>,
}

impl Advertiser {
    pub fn start(name: &str, identity: &Identity, txt: &TxtRecords, port: u16) -> Result<Self> {
        let daemon = ServiceDaemon::new().context("starting mDNS daemon")?;
        // Our sockets are IPv4-only; advertising IPv6 addresses would make
        // senders try connections we cannot accept.
        daemon.disable_interface(IfKind::IPv6).context("configuring mDNS interfaces")?;

        let host = format!("riflesso-{}.local.", identity.device_id_compact().to_lowercase());
        let services = [
            ("_airplay._tcp.local.", name.to_string(), &txt.airplay),
            ("_raop._tcp.local.", format!("{}@{}", identity.device_id_compact(), name), &txt.raop),
        ];

        let mut fullnames = Vec::new();
        for (ty, instance, entries) in services {
            let props: Vec<(&str, &str)> = entries.iter().map(|(k, v)| (*k, v.as_str())).collect();
            let info = ServiceInfo::new(ty, &instance, &host, "", port, &props[..])
                .with_context(|| format!("building {ty} service"))?
                .enable_addr_auto();
            fullnames.push(info.get_fullname().to_string());
            daemon.register(info).with_context(|| format!("registering {ty}"))?;
            tracing::info!("advertising {instance} on {ty} port {port}");
        }
        Ok(Self { daemon, fullnames })
    }
}

impl Drop for Advertiser {
    fn drop(&mut self) {
        for name in &self.fullnames {
            let _ = self.daemon.unregister(name);
        }
        let _ = self.daemon.shutdown();
    }
}
