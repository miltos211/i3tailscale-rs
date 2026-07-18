mod tailscale;

use clap::{Parser, Subcommand};
use std::io::Write;
use std::process::{Command, Stdio};
use tailscale::Status;

#[derive(Parser)]
#[command(name = "i3tailscale", about = "Tailscale status/toggle/peer-picker for i3status-rust")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print "on" if Tailscale is running, "off" otherwise. Meant for the
    /// i3status-rust `toggle` block's `command_state`.
    Status,
    /// Open a rofi picker over the tailnet's peers and copy the chosen
    /// peer's DNS name to the clipboard.
    PeerPick,
}

fn fetch_status() -> anyhow::Result<Status> {
    let output = Command::new("tailscale")
        .args(["status", "--json"])
        .output()?;
    if !output.status.success() {
        anyhow::bail!(
            "`tailscale status --json` exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let json = String::from_utf8(output.stdout)?;
    Ok(Status::from_json(&json)?)
}

/// i3status-rust's `toggle` block reads `command_state` by presence of
/// (trimmed) stdout, not by its text: any non-empty output means "on",
/// empty/whitespace-only output means "off" — content and exit code are
/// both ignored. So this must print nothing at all when off, never the
/// literal string "off" (confirmed empirically against the real block:
/// `echo off` and `echo on` both register as "on").
fn status_output(status: &Status) -> Option<&'static str> {
    status.is_running().then_some("on")
}

fn run_status() -> anyhow::Result<()> {
    let status = fetch_status()?;
    if let Some(text) = status_output(&status) {
        println!("{text}");
    }
    Ok(())
}

fn run_peer_pick() -> anyhow::Result<()> {
    let status = fetch_status()?;
    let lines = tailscale::format_peer_lines(&status);

    let mut rofi = Command::new("rofi")
        .args([
            "-dmenu",
            "-p",
            "Tailscale hosts",
            "-i",
            "-mesg",
            "Enter: copy DNS name  ·  Alt+Enter: copy IP",
            "-kb-custom-1",
            "Alt+Return",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    rofi.stdin
        .take()
        .expect("piped stdin")
        .write_all(lines.as_bytes())?;
    let output = rofi.wait_with_output()?;

    let choice = String::from_utf8(output.stdout)?;
    let choice = choice.trim();
    if choice.is_empty() {
        return Ok(());
    }
    let host_name = choice
        .split('\t')
        .next()
        .expect("split always yields at least one item");

    let Some(peer) = status.find_peer(host_name) else {
        anyhow::bail!("no peer named '{host_name}' in tailscale status");
    };

    let value = value_to_copy(peer, output.status.code())?;
    copy_to_clipboard(value)
}

/// rofi dmenu exit codes: 0 = Enter (normal accept), 10 = custom-1 (bound
/// to Alt+Return in `run_peer_pick`). Anything else falls back to the
/// default Enter behavior (copy DNS name).
fn value_to_copy(peer: &tailscale::Peer, rofi_exit_code: Option<i32>) -> anyhow::Result<&str> {
    if rofi_exit_code == Some(10) {
        peer.ipv4().ok_or_else(|| {
            anyhow::anyhow!("peer '{}' has no Tailscale IPv4 address", peer.host_name)
        })
    } else {
        Ok(&peer.dns_name)
    }
}

/// On Linux (X11 and Wayland alike), clipboard content is owned by whichever
/// process set it and vanishes the instant that process exits, unless it
/// stays alive to keep serving paste requests. `.wait()` blocks until
/// something else takes over the clipboard, so this process briefly acts as
/// its own clipboard daemon instead of the copy disappearing before anyone
/// can paste it (confirmed missing in testing: without this, `peer-pick`
/// selected a peer successfully but nothing ended up on the clipboard).
fn copy_to_clipboard(text: &str) -> anyhow::Result<()> {
    let mut clipboard = arboard::Clipboard::new()?;
    #[cfg(target_os = "linux")]
    {
        use arboard::SetExtLinux;
        clipboard.set().wait().text(text)?;
    }
    #[cfg(not(target_os = "linux"))]
    {
        clipboard.set_text(text)?;
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Cmd::Status => run_status(),
        Cmd::PeerPick => run_peer_pick(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status_subcommand() {
        let cli = Cli::parse_from(["i3tailscale", "status"]);
        assert!(matches!(cli.command, Cmd::Status));
    }

    #[test]
    fn parses_peer_pick_subcommand() {
        let cli = Cli::parse_from(["i3tailscale", "peer-pick"]);
        assert!(matches!(cli.command, Cmd::PeerPick));
    }

    #[test]
    fn rejects_unknown_subcommand() {
        let result = Cli::try_parse_from(["i3tailscale", "bogus"]);
        assert!(result.is_err());
    }

    // Synthetic, not real tailnet data — matches the convention in tailscale.rs.
    const SAMPLE_JSON: &str = r#"{
        "BackendState": "Running",
        "Self": {
            "HostName": "self-device",
            "DNSName": "self-device.tailnet.ts.net.",
            "Online": true
        },
        "Peer": {
            "n1": {
                "HostName": "has-ip",
                "DNSName": "has-ip.tailnet.ts.net.",
                "Online": true,
                "TailscaleIPs": ["100.64.0.5", "fd7a:115c:a1e0::5"]
            },
            "n2": {
                "HostName": "no-ip",
                "DNSName": "no-ip.tailnet.ts.net.",
                "Online": false
            }
        }
    }"#;

    // This is the exact bug that made it to a real device before being
    // caught: the toggle block's `command_state` treats ANY non-empty
    // stdout as "on" (see `run_status`'s doc comment), so printing the
    // literal text "off" is wrong — it must print nothing at all.
    #[test]
    fn status_output_is_none_when_stopped_not_the_text_off() {
        let running = Status::from_json(SAMPLE_JSON).unwrap();
        assert_eq!(status_output(&running), Some("on"));

        let stopped = Status::from_json(&SAMPLE_JSON.replace("Running", "Stopped")).unwrap();
        assert_eq!(status_output(&stopped), None);
    }

    // This is the other real bug: Alt+Enter originally copied the hostname
    // instead of the IP address.
    #[test]
    fn enter_copies_dns_name_alt_enter_copies_ip() {
        let status = Status::from_json(SAMPLE_JSON).unwrap();
        let peer = status.find_peer("has-ip").unwrap();

        // Plain Enter (exit code 0) and anything unexpected both fall back
        // to the default: copy DNS name.
        assert_eq!(value_to_copy(peer, Some(0)).unwrap(), "has-ip.tailnet.ts.net.");
        assert_eq!(value_to_copy(peer, None).unwrap(), "has-ip.tailnet.ts.net.");

        // Alt+Enter (rofi custom-1, exit code 10) copies the IPv4 address.
        assert_eq!(value_to_copy(peer, Some(10)).unwrap(), "100.64.0.5");
    }

    #[test]
    fn alt_enter_errors_when_peer_has_no_ip() {
        let status = Status::from_json(SAMPLE_JSON).unwrap();
        let peer = status.find_peer("no-ip").unwrap();

        assert!(value_to_copy(peer, Some(10)).is_err());
        // Enter still works fine even though this peer has no IP.
        assert_eq!(value_to_copy(peer, Some(0)).unwrap(), "no-ip.tailnet.ts.net.");
    }
}
