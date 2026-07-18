use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct Status {
    #[serde(rename = "BackendState")]
    pub backend_state: String,
    #[serde(rename = "Self")]
    pub myself: Peer,
    #[serde(rename = "Peer", default)]
    pub peers: HashMap<String, Peer>,
}

#[derive(Debug, Deserialize)]
pub struct Peer {
    #[serde(rename = "HostName", default)]
    pub host_name: String,
    #[serde(rename = "DNSName", default)]
    pub dns_name: String,
    #[serde(rename = "Online", default)]
    pub online: bool,
    /// IPv4 first, then IPv6 (matches Tailscale's own ordering), e.g.
    /// `["100.73.195.116", "fd7a:115c:a1e0::7b35:c376"]`.
    #[serde(rename = "TailscaleIPs", default)]
    pub tailscale_ips: Vec<String>,
}

impl Peer {
    /// The peer's Tailscale IPv4 address, if it has one. Looked up by
    /// actually parsing each entry as IPv4 rather than assuming index 0,
    /// in case ordering ever changes.
    pub fn ipv4(&self) -> Option<&str> {
        self.tailscale_ips
            .iter()
            .find(|ip| ip.parse::<std::net::Ipv4Addr>().is_ok())
            .map(String::as_str)
    }
}

impl Status {
    pub fn from_json(json: &str) -> serde_json::Result<Status> {
        serde_json::from_str(json)
    }

    /// Matches i3status-rust's `toggle` block command_state contract: exactly
    /// "on" or anything else means off.
    pub fn is_running(&self) -> bool {
        self.backend_state == "Running"
    }

    /// Peers sorted by hostname for a stable, predictable picker list.
    pub fn peer_list(&self) -> Vec<(&Peer, bool)> {
        let mut entries: Vec<(&Peer, bool)> = self
            .peers
            .values()
            .map(|p| (p, false))
            .chain(std::iter::once((&self.myself, true)))
            .collect();
        entries.sort_by(|a, b| a.0.host_name.cmp(&b.0.host_name));
        entries
    }

    /// Looks up a peer by DNS name, not hostname — `HostName` is a
    /// self-reported, unenforced field and can collide between peers (two
    /// devices can share a hostname). `DNSName` is the field Tailscale's
    /// MagicDNS actually guarantees unique (deduplicated with a `-1`/`-2`
    /// suffix), so it's the only safe key for resolving exactly the peer
    /// the user picked. Iterates directly rather than going through the
    /// sorted `peer_list()` — this is a lookup, not a display list.
    pub fn find_peer_by_dns_name(&self, dns_name: &str) -> Option<&Peer> {
        self.peers
            .values()
            .chain(std::iter::once(&self.myself))
            .find(|p| p.dns_name == dns_name)
    }
}

/// One line per host: "hostname\tdns_name\tstatus_label", for piping into rofi.
pub fn format_peer_lines(status: &Status) -> String {
    status
        .peer_list()
        .into_iter()
        .map(|(peer, is_self)| {
            let label = if is_self {
                "(this device)"
            } else if peer.online {
                "online"
            } else {
                "offline"
            };
            format!("{}\t{}\t{}", peer.host_name, peer.dns_name, label)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic, not pulled from a real tailnet — this repo may end up public.
    const SAMPLE_JSON: &str = r#"{
        "BackendState": "Running",
        "Self": {
            "HostName": "self-device",
            "DNSName": "self-device.tailnet.ts.net.",
            "Online": true
        },
        "Peer": {
            "n1": {
                "HostName": "peer-b",
                "DNSName": "peer-b.tailnet.ts.net.",
                "Online": false
            },
            "n2": {
                "HostName": "peer-a",
                "DNSName": "peer-a.tailnet.ts.net.",
                "Online": true,
                "TailscaleIPs": ["100.64.0.1", "fd7a:115c:a1e0::1"]
            }
        }
    }"#;

    #[test]
    fn parses_real_field_names() {
        let status = Status::from_json(SAMPLE_JSON).expect("valid fixture");
        assert_eq!(status.myself.host_name, "self-device");
        assert_eq!(status.peers.len(), 2);
    }

    #[test]
    fn running_state_detected() {
        let status = Status::from_json(SAMPLE_JSON).unwrap();
        assert!(status.is_running());

        let stopped = SAMPLE_JSON.replace("Running", "Stopped");
        let status = Status::from_json(&stopped).unwrap();
        assert!(!status.is_running());
    }

    #[test]
    fn ipv4_picked_out_of_mixed_ip_list() {
        let status = Status::from_json(SAMPLE_JSON).unwrap();
        let peer_a = status.find_peer_by_dns_name("peer-a.tailnet.ts.net.").unwrap();
        assert_eq!(peer_a.ipv4(), Some("100.64.0.1"));

        // peer-b has no TailscaleIPs in the fixture at all (defaults to empty).
        let peer_b = status.find_peer_by_dns_name("peer-b.tailnet.ts.net.").unwrap();
        assert_eq!(peer_b.ipv4(), None);
    }

    #[test]
    fn peer_list_sorted_by_hostname_and_includes_self() {
        let status = Status::from_json(SAMPLE_JSON).unwrap();
        let names: Vec<&str> = status
            .peer_list()
            .iter()
            .map(|(p, _)| p.host_name.as_str())
            .collect();
        assert_eq!(names, vec!["peer-a", "peer-b", "self-device"]);
    }

    #[test]
    fn find_peer_by_dns_name() {
        let status = Status::from_json(SAMPLE_JSON).unwrap();
        let peer = status
            .find_peer_by_dns_name("peer-a.tailnet.ts.net.")
            .expect("peer-a exists");
        assert_eq!(peer.host_name, "peer-a");
        assert_eq!(peer.dns_name, "peer-a.tailnet.ts.net.");
        assert!(status.find_peer_by_dns_name("nonexistent").is_none());
    }

    // The real bug this was fixed for: HostName isn't guaranteed unique
    // across a tailnet (unlike DNSName, which MagicDNS deduplicates), so
    // looking a peer up by hostname could silently resolve to the wrong
    // device. Two peers sharing a hostname but with distinct DNS names must
    // still resolve correctly when looked up by DNS name.
    #[test]
    fn duplicate_hostnames_resolve_correctly_by_dns_name() {
        const DUPLICATE_HOSTNAME_JSON: &str = r#"{
            "BackendState": "Running",
            "Self": {
                "HostName": "self-device",
                "DNSName": "self-device.tailnet.ts.net."
            },
            "Peer": {
                "n1": {
                    "HostName": "laptop",
                    "DNSName": "laptop.tailnet.ts.net.",
                    "TailscaleIPs": ["100.64.0.1"]
                },
                "n2": {
                    "HostName": "laptop",
                    "DNSName": "laptop-1.tailnet.ts.net.",
                    "TailscaleIPs": ["100.64.0.2"]
                }
            }
        }"#;
        let status = Status::from_json(DUPLICATE_HOSTNAME_JSON).unwrap();

        let first = status.find_peer_by_dns_name("laptop.tailnet.ts.net.").unwrap();
        assert_eq!(first.ipv4(), Some("100.64.0.1"));

        let second = status.find_peer_by_dns_name("laptop-1.tailnet.ts.net.").unwrap();
        assert_eq!(second.ipv4(), Some("100.64.0.2"));
    }

    #[test]
    fn format_peer_lines_labels_self_and_online_state() {
        let status = Status::from_json(SAMPLE_JSON).unwrap();
        let out = format_peer_lines(&status);
        assert_eq!(
            out,
            "peer-a\tpeer-a.tailnet.ts.net.\tonline\n\
             peer-b\tpeer-b.tailnet.ts.net.\toffline\n\
             self-device\tself-device.tailnet.ts.net.\t(this device)"
        );
    }
}
