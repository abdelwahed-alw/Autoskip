Act as a Senior Rust Systems and UI Engineer. I want to build a smart, cross-platform video player named "Otip". The core feature of Otip is an AI-powered "Lookahead Content Moderator" that analyzes upcoming scenes and automatically skips explicitly NSFW content without modifying the original video file.

I want the application to be written 100% in Rust.

Here is the precise architectural blueprint for the project:

### 1. The UI Layer (Frontend)
*   **Framework:** Use the `Iced` GUI framework (pure Rust, Elm architecture).
*   **Design:** A modern, clean video player interface.
*   **Components:**
    *   Video display area (rendering frames seamlessly).
    *   Play/Pause buttons.
    *   **The Smart Timeline (Crucial):** A custom seek bar with visual color indicators:
        *   Gray: Unscanned/Unknown ahead.
        *   Green: Scanned and safe (Buffer zone).
        *   Red dots/segments: Explicit content detected (to be skipped).
*   **Pre-play Prompt:** When a user opens a video, present two options:
    1.  "Safe Mode": Wait for the full video to be scanned before playing.
    2.  "Instant Play (Zero Trust)": Start immediately; scanning runs asynchronously in the background ahead of the current playtime. Warn the user that manual seeking might jump past the scanned safe zone.

### 2. The Video Engine (Backend/Decoding)
*   **Integration:** Use `libmpv` (via the `mpv` crate) or `GStreamer` integrated into the Iced application to handle video decoding and rendering. 
*   **Frame Extraction:** In the background, extract low-resolution frames (e.g., 320x240) efficiently, utilizing hardware acceleration (e.g., Intel Quick Sync or general VAAPI/DXVA) where possible.
*   **Execution:** Do not save extracted frames to the disk; hold them in memory to feed the AI scanner.

### 3. The AI & Optimization Layer (The Core Logic)
*   **Optimization (Image Gridding):** To save API tokens and speed up network requests, the Rust backend must take 4 extracted frames (representing 4 seconds of video) and stitch them into a single 2x2 grid image.
*   **AI Service:** Use Google's `Gemini 1.5 Flash Lite` (or similar ultra-fast vision model).
*   **The Request:** Send the stitched 2x2 image to the Gemini API (via HTTP/reqwest) with a prompt asking: "Identify which of the 4 quadrants (top-left, top-right, bottom-left, bottom-right) contain explicit NSFW content. Return a JSON array of the bad quadrant numbers."
*   **Reverse Calculation:** Parse the JSON response. If Gemini flags quadrant 3 (bottom-left) as bad in grid #10, the Rust logic must calculate the exact second this corresponds to and log it.

### 4. The Playback Logic (Auto-Skip)
*   The scanner thread communicates with the UI/Player thread via `tokio` channels (`mpsc`).
*   The bad timestamps are kept in memory (or generated into a virtual EDL list).
*   As the player reaches a flagged timestamp (e.g., second 39), it immediately issues a `Seek` command to jump over the bad segment (e.g., to second 40) automatically.

### Constraints & Goals
*   Target OS: Must be easily cross-compiled for Linux (development environment) and Windows.
*   Performance: The background scanning thread must not freeze or block the UI rendering thread.

Please provide the initial project structure (Cargo.toml with necessary dependencies) and the foundational Rust code (`main.rs`) that sets up the `Iced` UI with the Pre-play selection screen and the basic Player state, preparing the hooks for the async video scanning thread.