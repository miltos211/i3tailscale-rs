# i3tailscale-rs

![Rust](https://img.shields.io/badge/rust-%23000000.svg?style=for-the-badge&logo=rust&logoColor=white)
![i3](https://img.shields.io/badge/i3-52C0FF?style=for-the-badge&logo=i3&logoColor=black)
![Tailscale](https://img.shields.io/badge/Tailscale-242424?style=for-the-badge&logo=tailscale&logoColor=white)
![Wayland](https://img.shields.io/badge/Wayland-FFBC00?style=for-the-badge&logo=wayland&logoColor=black)
![X11](https://img.shields.io/badge/X11-F28834?style=for-the-badge&logo=xdotorg&logoColor=white)

A small helper for [i3status-rust](https://github.com/greshake/i3status-rust) that adds Tailscale support to your i3/sway status bar.

## Why

i3status-rust doesn't have Tailscale support built in, and there wasn't an existing i3bar-native equivalent for it (there are similar projects for other bars — see credits below). This started as a small itch-scratch for a personal setup and is being built out from there.

## What it does

- Shows whether Tailscale is currently up or down in the bar.
- Click to toggle Tailscale on/off.
- A keybinding opens a searchable [rofi](https://github.com/davatorium/rofi) popup listing your tailnet's peers — pick one to copy its DNS name to the clipboard (or hold Alt while picking to copy its Tailscale IP instead).

## Dependencies

Runtime, regardless of how you get the binary:

- [Tailscale](https://tailscale.com) — the `tailscale` CLI and a running `tailscaled`.
- [i3status-rust](https://github.com/greshake/i3status-rust), with its `toggle` block. Tested against v0.22.0.
- [rofi](https://github.com/davatorium/rofi), for the peer picker.
- A running X11 or Wayland session. Clipboard copy works on either automatically (via [`arboard`](https://docs.rs/arboard))

Build-time, only if building from source:

- Rust, via [rustup](https://rustup.rs).

One-time setup, before it'll actually work:

- Tailscale gates `tailscale up`/`tailscale down` behind its own "operator" permission — separate from, and stricter than, whatever file permissions its socket has. Without this, clicking the toggle block will fail silently (briefly shows an error, then reverts) because `up`/`down` will demand `sudo`, and the bar has no way to prompt you for a password:

  ```
  sudo tailscale set --operator=$USER
  ```

  This is a one-time grant per machine, not something this tool can do for you.

## Example config

i3status-rust `toggle` block, in your `config.toml`:

```toml
[[block]]
block = "toggle"
command_state = "/path/to/i3tailscale status"
command_on = "tailscale up"
command_off = "tailscale down"
icon_on = "net_vpn"      # optional — needs an icon theme that defines this key (e.g. material-nf)
icon_off = "toggle_off"  # optional — same caveat
interval = 5
```

Use an absolute path for `command_state` (pointing at wherever you installed `i3tailscale`, e.g. `~/.local/bin/i3tailscale`) — i3status-rust's exec environment doesn't reliably resolve `PATH` for binaries outside standard system directories. `tailscale`/`rofi` themselves are fine as bare commands in `command_on`/`command_off` above since they're installed via a package manager into a directory that's on `PATH` for virtually any setup (`/usr/bin` or similar); if that's not the case on yours, use their absolute paths too.

There's no generic right-click/click-override hook on i3status-rust blocks (at least not on the version this was tested against), so the peer picker is bound as a plain i3 keybinding instead of a bar click. In your i3 `config`:

```
bindsym $mod+p exec --no-startup-id /path/to/i3tailscale peer-pick
```

Press the bound key, pick a peer from the rofi list — **Enter** copies its Tailscale DNS name, **Alt+Enter** copies its Tailscale IPv4 address.

## Build from source

Requires Rust 1.85+ (this project uses the 2024 edition) — recent enough that some distro-packaged toolchains may be too old; [rustup](https://rustup.rs) always has a current one.

```
git clone https://github.com/miltos211/i3tailscale-rs.git
cd i3tailscale-rs
cargo build --release
```

The binary ends up at `target/release/i3tailscale`.

Cross-compiling (e.g. building on a faster machine for a weaker target device) needs the target's Rust std installed and, if you're compiling from a different OS, a matching linker:

```
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
```

On macOS cross-compiling to Linux, for example, that linker comes from `brew install filosottile/musl-cross/musl-cross`, pointed at from `~/.cargo/config.toml`:

```toml
[target.x86_64-unknown-linux-musl]
linker = "x86_64-linux-musl-gcc"
```

Prebuilt `x86_64-unknown-linux-musl` binaries are also published on [Releases](https://github.com/miltos211/i3tailscale-rs/releases) for every tagged version, if you'd rather skip building it yourself.

## Credits / prior art

The design leans on the approach from [`mbugert/tailscale-polybar-rofi`](https://github.com/mbugert/tailscale-polybar-rofi) (status display + rofi peer picker, built for Polybar) and [`OmarSkalli/waybar-tailscale`](https://github.com/OmarSkalli/waybar-tailscale) (status + toggle, built for Waybar).
