# Otip — AI-Powered Smart Video Player

**Otip** is a cross-platform video player with an AI-powered "Lookahead Content Moderator" that analyzes upcoming scenes and automatically skips explicitly NSFW content — without modifying the original video file.

## Features

- 🛡️ **Safe Mode** — Full video scan before playback begins
- ⚡ **Instant Play (Zero Trust)** — Start immediately; AI scanning runs asynchronously in the background
- 🎯 **Auto-Skip** — Seamless content filtering that skips flagged segments in real time
- 🔧 **Hardware Acceleration** — GPU-accelerated decoding via libmpv (VAAPI, NVDEC, VideoToolbox)
- 📊 **Smart Timeline** — Color-coded seek bar: gray (unscanned), green (safe), red (explicit)
- 🤖 **Gemini AI Integration** — Uses Google Gemini for frame-by-frame content analysis
- 🎬 **Library Browser** — Scan folders, thumbnails, search, chapter markers
- 🌐 **Network Streams** — Play HTTP/HTTPS URLs directly
- ⌨️ **Keyboard Shortcuts** — Space (play/pause), arrows (seek/volume), F (fullscreen)

## Architecture

```
otip/
├── otip-core/    # Domain types, error hierarchy, config, timeline, scan pipeline
├── otip-video/   # Video engine abstraction (libmpv backend)
├── otip-ai/      # Gemini API client, grid processor, content moderator
├── otip-ui/      # Iced GUI framework (screens, widgets, theme) [scaffold]
├── otip-app/     # Main application binary (3-screen Iced app)
└── src/          # Workspace root redirect
```

## Prerequisites

| Dependency | Required | Purpose |
|-----------|----------|---------|
| **Rust** (1.75+) | ✅ | Build toolchain |
| **ffmpeg** / **ffprobe** | ✅ | Frame extraction & metadata probing |
| **libmpv** (`libmpv2`) | ✅ | Video playback engine |
| **pkg-config** | ✅ | Locating native libraries |
| **Gemini API key** | For AI features | Content moderation scanning |

### Linux (Debian/Ubuntu)

```bash
sudo apt install ffmpeg libmpv-dev pkg-config libvulkan-dev
```

### Linux (Fedora)

```bash
sudo dnf install ffmpeg mpv-libs-devel pkg-config vulkan-loader-devel
```

### macOS

```bash
brew install ffmpeg mpv pkg-config
```

## Building

```bash
# Development build
cargo build -p otip-app

# Release build (optimized, LTO enabled)
cargo build -p otip-app --release

# Run directly
cargo run -p otip-app
```

## Running

```bash
# Launch the app
cargo run -p otip-app

# With debug logging
RUST_LOG=otip=debug cargo run -p otip-app
```

### Setting up AI scanning

1. Launch Otip
2. Open the **AI Panel** (🤖 button)
3. Enter your **Gemini API key**
4. (Optional) Select a model: Gemini 3.8 Flash or Gemini 3.5 Flash Lite
5. (Optional) Enter a custom skip prompt (e.g., "skip ads and explicit content")
6. Open a video → click **Start AI Scan**

## Running Tests

```bash
cargo test --workspace
```

## Configuration

Config is stored at `~/.config/otip/otip/config.toml` (Linux) or the platform-equivalent `ProjectDirs` path.

| Setting | Default | Description |
|---------|---------|-------------|
| `gemini_api_key` | (none) | Google Gemini API key |
| `gemini_model` | `gemini-3.8-flash` | AI model for scanning |
| `scan_frame_interval` | `1` | Seconds between extracted frames |
| `grid_size` | `(2, 2)` | Frames per grid (2×2 = 4 per API call) |
| `frame_resolution` | `(320, 240)` | Frame extraction resolution |

The API key can also be set via the `GEMINI_API_KEY` environment variable.

## License

MIT OR Apache-2.0
