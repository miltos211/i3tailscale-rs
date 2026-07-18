mod tailscale;

use anyhow::Context;
use clap::{Parser, Subcommand};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::Duration;
use tailscale::Status;
use wait_timeout::ChildExt;

/// How long to wait for `tailscale status --json` before giving up. Chosen
/// generously — this is polled every few seconds by the bar, so a genuinely
/// wedged `tailscaled` shouldn't be left to hang forever and pile up
/// abandoned processes.
const TAILSCALE_STATUS_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Parser)]
#[command(name = "i3tailscale", about = "Tailscale status/toggle/peer-picker for i3status-rust")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print "on" if Tailscale is running, print nothing otherwise. Meant
    /// for the i3status-rust `toggle` block's `command_state`, which reads
    /// state by presence of stdout, not by its text — printing the literal
    /// word "off" would be read as "on".
    Status,
    /// Open a rofi picker over the tailnet's peers and copy the chosen
    /// peer's DNS name (or, with Alt+Enter, its Tailscale IPv4 address) to
    /// the clipboard.
    PeerPick,
}

fn fetch_status() -> anyhow::Result<Status> {
    let mut child = Command::new("tailscale")
        .args(["status", "--json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to run `tailscale status --json` — is the Tailscale CLI installed?")?;

    let status = child
        .wait_timeout(TAILSCALE_STATUS_TIMEOUT)
        .context("failed waiting on `tailscale status --json`")?;
    let Some(status) = status else {
        let _ = child.kill();
        let _ = child.wait();
        anyhow::bail!(
            "`tailscale status --json` didn't respond within {TAILSCALE_STATUS_TIMEOUT:?} \
             (tailscaled may be unresponsive)"
        );
    };

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    child.stdout.take().expect("piped stdout").read_to_end(&mut stdout)?;
    child.stderr.take().expect("piped stderr").read_to_end(&mut stderr)?;

    if !status.success() {
        anyhow::bail!(
            "`tailscale status --json` exited with {status}: {}",
            String::from_utf8_lossy(&stderr)
        );
    }
    let json = String::from_utf8(stdout)?;
    Status::from_json(&json).context("failed to parse `tailscale status --json` output")
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

/// rofi's dmenu exit code for `-kb-custom-1` (bound to Alt+Return in
/// `run_peer_pick`'s rofi invocation) — rofi's own convention: 0 = Enter,
/// 10..=28 = custom-1..custom-19.
const ROFI_ALT_ENTER_EXIT_CODE: i32 = 10;

fn run_peer_pick() -> anyhow::Result<()> {
    let status = fetch_status()?;
    let lines = tailscale::format_peer_lines(&status);

    let mut rofi = Command::new("rofi")
        .args([
            "-dmenu",
            "-p",
            "Tailscale hosts",
            "-i",
            "-no-custom",
            "-mesg",
            "Enter: copy DNS name  ·  Alt+Enter: copy IP",
            "-kb-custom-1",
            "Alt+Return",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("failed to run rofi — is it installed?")?;

    // Write on a separate thread rather than blocking on it before reading
    // stdout: a large enough peer list could exceed the pipe buffer, and
    // rofi doesn't keep draining stdin once it's rendering the interactive
    // menu — writing and reading concurrently avoids that deadlock.
    let mut stdin = rofi.stdin.take().expect("piped stdin");
    let writer = std::thread::spawn(move || stdin.write_all(lines.as_bytes()));

    let output = rofi.wait_with_output().context("failed waiting on rofi")?;
    writer
        .join()
        .expect("stdin writer thread panicked")
        .context("failed writing peer list to rofi's stdin")?;

    let choice = String::from_utf8(output.stdout)?;
    let choice = choice.trim();
    if choice.is_empty() {
        return Ok(());
    }
    // Line format is "hostname\tdns_name\tstatus_label" (format_peer_lines);
    // look up by the DNS name field (index 1), not hostname (index 0) — see
    // find_peer_by_dns_name for why.
    let dns_name = choice
        .split('\t')
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("unexpected rofi selection format: '{choice}'"))?;

    let Some(peer) = status.find_peer_by_dns_name(dns_name) else {
        anyhow::bail!("no peer with DNS name '{dns_name}' in tailscale status");
    };

    let value = value_to_copy(peer, output.status.code())?;
    copy_to_clipboard(value)
}

/// Anything other than `ROFI_ALT_ENTER_EXIT_CODE` falls back to the default
/// Enter behavior (copy DNS name).
fn value_to_copy(peer: &tailscale::Peer, rofi_exit_code: Option<i32>) -> anyhow::Result<&str> {
    if rofi_exit_code == Some(ROFI_ALT_ENTER_EXIT_CODE) {
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
    let is_peer_pick = matches!(cli.command, Cmd::PeerPick);
    let result = match cli.command {
        Cmd::Status => run_status(),
        Cmd::PeerPick => run_peer_pick(),
    };
    // `peer-pick` is launched via an i3 keybinding (`exec --no-startup-id`)
    // with no visible terminal, so stderr — where anyhow's default error
    // reporting goes — is never seen. Surface failures via a desktop
    // notification instead of letting them vanish silently. Best-effort:
    // if notify-send itself isn't available, this just falls through to
    // the normal (invisible) stderr path, no further escalation. `status`
    // is deliberately excluded — it's polled every few seconds by the bar,
    // and notifying on every failed poll would be spam, not a UX fix.
    if is_peer_pick {
        if let Err(err) = &result {
            let _ = Command::new("notify-send")
                .args(["i3tailscale", &format!("peer-pick failed: {err}")])
                .status();
        }
    }
    result
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
        let peer = status.find_peer_by_dns_name("has-ip.tailnet.ts.net.").unwrap();

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
        let peer = status.find_peer_by_dns_name("no-ip.tailnet.ts.net.").unwrap();

        assert!(value_to_copy(peer, Some(10)).is_err());
        // Enter still works fine even though this peer has no IP.
        assert_eq!(value_to_copy(peer, Some(0)).unwrap(), "no-ip.tailnet.ts.net.");
    }
}
