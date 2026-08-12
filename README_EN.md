﻿# VoicePlayer

English | [中文](./README.md)

A lightweight Windows soundboard hotkey tool (similar to Soundpad). Press a global
hotkey to mix sound effects into your microphone so everyone in your game or voice
chat can hear them — while **your normal speech is unaffected**.

- Pure Rust + egui, single exe, small footprint, low memory, fast startup
- Multiple profiles (each profile is a folder); drop audio files in and they auto-appear.
  Any folder on your computer can also be added as a profile.
- Bind a global hotkey per sound, plus a dedicated "stop all" hotkey
- Main-row and numpad digit keys can be bound independently (no interference)
- One-click lock: when locked, hotkeys won't trigger any sound — prevents accidental fire
- Multi-language: Chinese / English, defaults to system language
- Hotkeys use a low-level keyboard hook (WH_KEYBOARD_LL) that never blocks key events,
  so a bound key still types normally
- Hotkeys are monitored on a background thread — works in fullscreen games and when minimized
- Multiple sounds mix simultaneously; repeated key behavior is configurable:
  restart / overlap / toggle
- Optional: autostart, headphone monitoring (hear effects yourself)

## How It Works

Games and Discord capture audio from "microphones." VoicePlayer does its own mixing:

```
Real mic ──┐
           ├─→ VoicePlayer mixes ──→ CABLE Input ──→ Game mic = CABLE Output
Sound file ──┘                      (VB-CABLE virtual sound card)
```

Since mixing happens inside the app, **you don't need VoiceMeeter** — just a single
"pipe" virtual sound card, and the free [VB-CABLE](https://vb-audio.com/Cable/) works.

## Setup

1. Install [VB-CABLE](https://vb-audio.com/Cable/) and restart your computer.
2. Open VoicePlayer, set **Output device** to `CABLE Input (VB-Audio Virtual Cable)`.
3. In your game / Discord, set **microphone** to `CABLE Output (VB-Audio Virtual Cable)`.
4. Click "Open folder", drop mp3 / wav / ogg / flac files in — they auto-appear in the list.
5. Click "Set hotkey" for each sound and press your desired key combination.
6. (Optional) To hear effects yourself, set "Monitor device" to your headphones.

> Note: Your game mic is set to CABLE Output, so **VoicePlayer must stay open** to
> forward your real mic. Closing the app means the game gets no mic audio.
> Enable "Launch on startup" for convenience.

## Data Directory

Config and audio live in `%APPDATA%\VoicePlayer\`:

```
%APPDATA%\VoicePlayer\
├─ config.json
└─ profiles\
   └─ Default\
      ├─ xxx.mp3
      └─ _bindings.json   # hotkey / volume bindings
```

## Building from Source

Requires the Rust toolchain (`rustup`). Windows only (audio uses WASAPI).

```bash
cargo run            # debug run
cargo build --release
```

Releasing: push a `v`-prefixed tag (e.g. `v0.0.3`) and GitHub Actions will
automatically build a portable zip + installer and publish to Releases.

## License

MIT