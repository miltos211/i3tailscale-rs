# i3tailscale-rs

A small helper for [i3status-rust](https://github.com/greshake/i3status-rust) that adds Tailscale support to your i3/sway status bar.

## What it does

- Shows whether Tailscale is currently up or down in the bar.
- Click to toggle Tailscale on/off.
- A keybinding opens a searchable [rofi](https://github.com/davatorium/rofi) popup listing your tailnet's peers — pick one to copy its DNS name to the clipboard (or hold Alt while picking to copy its Tailscale IP instead).

## Why

i3status-rust doesn't have Tailscale support built in, and there wasn't an existing i3bar-native equivalent for it (there are similar projects for other bars — see credits below). This started as a small itch-scratch for a personal setup and is being built out from there.

## Credits / prior art

The design leans on the approach from [`mbugert/tailscale-polybar-rofi`](https://github.com/mbugert/tailscale-polybar-rofi) (status display + rofi peer picker, built for Polybar) and [`OmarSkalli/waybar-tailscale`](https://github.com/OmarSkalli/waybar-tailscale) (status + toggle, built for Waybar).
