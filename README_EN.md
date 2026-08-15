# VoicePlayer

English | [中文](./README.md)

A lightweight Windows soundboard hotkey tool (similar to Soundpad). Press a global
hotkey to mix sound effects into your microphone so everyone in your game or voice
chat can hear them — while **your normal speech is unaffected**.

## Features

- **Multi-sound mixing**: multiple sounds play simultaneously without interfering
- **Hotkey binding**: bind a global hotkey per sound, plus a dedicated "stop all" hotkey
- **Main-row / numpad independent binding**: main-row 1 and numpad 1 can be bound to different sounds
- **One-click lock**: when locked, hotkeys won't trigger any sound
- **Non-blocking keys**: bound keys still type normally — no key swallowing
- **Repeat key behavior**: restart / overlap / toggle
- **System audio capture**: mix audio currently playing on your PC (music, video, etc.) into the virtual mic
- **App audio routing**: route a specific app's audio (browser, music player, etc.) into the virtual mic
- **Headphone monitoring**: hear effects yourself
- **Multi-language**: Chinese / English, defaults to system language
- **Autostart**: optionally launch on system startup
- **Window position memory**: restores last window position and size on next launch

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

## System Audio Capture (Two Methods)

Want to send audio currently playing on your PC (music, video, etc.) into the game mic? There are two ways:

### Method 1: System Audio Capture (all system sound)

Best for mixing **all system audio** into the virtual mic at once.

1. Open VoicePlayer, find the "System Audio Capture" section at the bottom
2. Check **"Capture system audio to microphone"**
3. Adjust the "Capture Volume" slider to control how loud the captured audio is
4. Now any sound playing on your PC (music, video, game audio, etc.) will be mixed into the virtual mic

> Note: This captures **all** system sound, including notification sounds. If you only want one app's audio, use Method 2 below.

### Method 2: App Audio Routing (specific app)

Best for sending only **one app's** audio into the virtual mic while leaving other apps unaffected.

1. Open VoicePlayer, find the "App Audio Routing" section at the bottom
2. Click **"Open app volume settings"** — Windows will open the Sound Settings → App Volume page
3. Find the app you want to route (e.g., NetEase Cloud Music, Chrome browser, etc.)
4. Change its **output device** to `CABLE Input (VB-Audio Virtual Cable)`
5. Now only that app's audio goes to the virtual mic; other apps still play through your speakers normally

> For example, if you set NetEase Cloud Music's output to CABLE Input, your teammates will hear your music; meanwhile your browser video audio still plays from your speakers as usual.

## Data Directory

Config and audio live in `%APPDATA%\VoicePlayer\`:

```
%APPDATA%\VoicePlayer\
├─ config.json        # global config
└─ profiles\
   └─ Default\
      ├─ xxx.mp3
      └─ _bindings.json   # hotkey bindings
```

## Building from Source

Requires the Rust toolchain (`rustup`). Windows only (audio uses WASAPI).

```bash
cargo run            # debug run
cargo build --release
```

Releasing: push a `v`-prefixed tag (e.g., `v0.0.3`) and GitHub Actions will
automatically build a portable zip + installer and publish to Releases.

## License

MIT