/**
 * ModernVideoPlayer — Vanilla JS HTML5 Video Player
 * Features:
 *  - Custom controls (play/pause center + bar, progress with buffered & tooltip, time, volume, fullscreen)
 *  - Captions / subtitles with multi-language menu (using <track>)
 *  - Settings menu: playback speed (0.5x-2x), quality selector (simulated source switch)
 *  - Playlist with Next/Prev, autoplay next, loop/repeat
 *  - Chapters / markers on progress bar
 *  - Loop toggle, Picture-in-Picture, keyboard shortcuts, mobile gestures (tap, double-tap seek)
 *  - Accessibility: ARIA, keyboard navigation, focus styles, contrast
 *  - Auto-hide controls after inactivity, dark theme
 *
 * Usage:
 *   const player = new ModernVideoPlayer(document.querySelector('#mv-player'), {
 *     playlist: [ ... ],
 *     autoplayNext: true,
 *     chapters: [ { time: 0, title: "Intro"}, ... ]
 *   });
 *
 * All labels/tooltips are in English. Comments in English.
 */

class ModernVideoPlayer {
  constructor(container, options = {}) {
    if (!container) throw new Error("ModernVideoPlayer: container element required");
    this.container = container;
    this.options = options;

    // Query core elements
    this.video = container.querySelector("video");
    if (!this.video) throw new Error("ModernVideoPlayer: <video> element not found inside container");

    this.centerPlayBtn = container.querySelector("[data-mv='center-play']");
    this.playBtn = container.querySelector("[data-mv='play']");
    this.prevBtn = container.querySelector("[data-mv='prev']");
    this.nextBtn = container.querySelector("[data-mv='next']");
    this.progressWrap = container.querySelector("[data-mv='progress-wrap']");
    this.progress = container.querySelector("[data-mv='progress']");
    this.progressFilled = container.querySelector("[data-mv='progress-filled']");
    this.progressBuffered = container.querySelector("[data-mv='progress-buffered']");
    this.tooltip = container.querySelector("[data-mv='tooltip']");
    this.tooltipTime = container.querySelector("[data-mv='tooltip-time']");
    this.tooltipChapter = container.querySelector("[data-mv='tooltip-chapter']");
    this.timeCurrent = container.querySelector("[data-mv='time-current']");
    this.timeTotal = container.querySelector("[data-mv='time-total']");
    this.muteBtn = container.querySelector("[data-mv='mute']");
    this.volumeSlider = container.querySelector("[data-mv='volume']");
    this.volumeGroup = container.querySelector("[data-mv='volume-group']");
    this.fullscreenBtn = container.querySelector("[data-mv='fullscreen']");
    this.pipBtn = container.querySelector("[data-mv='pip']");
    this.loopBtn = container.querySelector("[data-mv='loop']");
    this.captionsBtn = container.querySelector("[data-mv='captions']");
    this.settingsBtn = container.querySelector("[data-mv='settings']");
    this.settingsMenu = container.querySelector("[data-mv='settings-menu']");
    this.captionsMenu = container.querySelector("[data-mv='captions-menu']");
    this.helpBtn = container.querySelector("[data-mv='help-btn']");
    this.helpOverlay = container.querySelector("[data-mv='help']");
    this.spinner = container.querySelector("[data-mv='spinner']");
    this.captionsCue = container.querySelector("[data-mv='captions-cue']");

    // Playlist elements (optional)
    this.playlistEl = document.querySelector("[data-mv='playlist']");
    this.playlistListEl = document.querySelector("[data-mv='playlist-list']");
    this.autoplayToggle = document.querySelector("[data-mv='autoplay-toggle']");

    // Gesture zones
    this.tapZoneLeft = container.querySelector("[data-mv='tap-left']");
    this.tapZoneRight = container.querySelector("[data-mv='tap-right']");
    this.tapZoneCenter = container.querySelector("[data-mv='tap-center']");
    this.feedbackLeft = container.querySelector("[data-mv='feedback-left']");
    this.feedbackRight = container.querySelector("[data-mv='feedback-right']");

    // State
    this.playlist = options.playlist || null;
    this.currentIndex = 0;
    this.autoplayNext = options.autoplayNext ?? true;
    this.isLoopPlaylist = options.loop ?? false;
    this.chapters = options.chapters || [];
    this.qualities = options.qualities || null; // for single video
    this.currentQuality = null;
    this.isDragging = false;
    this.wasPlayingBeforeDrag = false;
    this.lastVolume = options.startVolume ?? 1;
    this.hideControlsTimer = null;
    this.lastTapTime = 0;
    this.tapCountLeft = 0;
    this.tapCountRight = 0;
    this.captionsEnabled = false;
    this.activeCaptionsTrack = null;

    // Icons (SVG strings)
    this.icons = {
      play: `<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M8 5.14v14l11-7-11-7z"/></svg>`,
      pause: `<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M6 19h4V5H6v14zm8-14v14h4V5h-4z"/></svg>`,
      volumeHigh: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true"><path d="M11 5L6 9H2v6h4l5 4V5z"/><path d="M15.54 8.46a5 5 0 010 7.08"/><path d="M17.8 6.2a8 8 0 010 11.6"/></svg>`,
      volumeLow: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true"><path d="M11 5L6 9H2v6h4l5 4V5z"/><path d="M15.54 8.46a5 5 0 010 7.08"/></svg>`,
      volumeMute: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true"><path d="M11 5L6 9H2v6h4l5 4V5z"/><path d="M23 9l-6 6"/><path d="M17 9l6 6"/></svg>`,
      fullscreenEnter: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true"><path d="M8 3H5a2 2 0 00-2 2v3"/><path d="M16 3h3a2 2 0 012 2v3"/><path d="M8 21H5a2 2 0 01-2-2v-3"/><path d="M16 21h3a2 2 0 002-2v-3"/></svg>`,
      fullscreenExit: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true"><path d="M8 8H5V5"/><path d="M16 8h3V5"/><path d="M8 16H5v3"/><path d="M16 16h3v3"/></svg>`,
      pip: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" aria-hidden="true"><rect x="3" y="5" width="18" height="14" rx="2"/><rect x="11" y="11" width="8" height="6" rx="1" fill="currentColor" stroke="none"/></svg>`,
      loop: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true"><path d="M17 1l4 4-4 4"/><path d="M3 11V9a4 4 0 014-4h14"/><path d="M7 23l-4-4 4-4"/><path d="M21 13v2a4 4 0 01-4 4H3"/></svg>`,
      captions: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" aria-hidden="true"><rect x="3" y="6" width="18" height="12" rx="2"/><path d="M7 12a2 2 0 012-2h1"/><path d="M7 12a2 2 0 002 2h1"/><path d="M13 12a2 2 0 012-2h1"/><path d="M13 12a2 2 0 002 2h1"/></svg>`,
      settings: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" aria-hidden="true"><circle cx="12" cy="12" r="3"/><path d="M12 1v2"/><path d="M12 21v2"/><path d="M4.22 4.22l1.42 1.42"/><path d="M18.36 18.36l1.42 1.42"/><path d="M1 12h2"/><path d="M21 12h2"/><path d="M4.22 19.78l1.42-1.42"/><path d="M18.36 5.64l1.42-1.42"/></svg>`,
      next: `<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M6 18l8.5-6L6 6v12zM16 6v12h2V6h-2z"/></svg>`,
      prev: `<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M6 6h2v12H6zM9.5 12L18 6v12l-8.5-6z"/></svg>`,
      rewind10: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" aria-hidden="true"><path d="M9 8l-5 4 5 4"/><path d="M20 12a8 8 0 11-8-8"/><text x="11.5" y="15.5" font-size="7" font-weight="700" fill="currentColor" stroke="none" text-anchor="middle">10</text></svg>`,
      forward10: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" aria-hidden="true"><path d="M15 8l5 4-5 4"/><path d="M4 12a8 8 0 108-8"/><text x="12.5" y="15.5" font-size="7" font-weight="700" fill="currentColor" stroke="none" text-anchor="middle">10</text></svg>`,
      close: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="M18 6L6 18"/><path d="M6 6l12 12"/></svg>`,
      check: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" aria-hidden="true"><path d="M5 13l4 4L19 7"/></svg>`,
    };

    this.init();
  }

  // ========= Initialization =========
  init() {
    // Setup video attributes
    this.video.controls = false;
    this.video.preload = "metadata";
    this.video.crossOrigin = "anonymous";

    // Apply initial volume
    if (this.options.startVolume !== undefined) {
      this.video.volume = Math.max(0, Math.min(1, this.options.startVolume));
    }
    this.lastVolume = this.video.volume || 1;

    // If playlist provided, load first item
    if (this.playlist && this.playlist.length > 0) {
      this.loadPlaylistItem(this.currentIndex, false);
    } else if (this.options.src) {
      // Single video mode: set src/qualities/chapters/tracks from options
      this.setSingleVideoSource(this.options);
    }

    // Build dynamic UI bits
    this.buildSettingsMenu();
    this.buildCaptionsMenu();
    if (this.chapters.length) this.renderChapterMarkers();
    if (this.playlist) this.renderPlaylist();

    this.updateVolumeUI();
    this.updatePlayButtonUI();
    this.updateLoopUI();
    this.updateCaptionsButtonUI();
    this.updatePipSupport();
    this.updateTimeDisplay(0, 0);

    this.bindEvents();
    this.resetHideTimer();

    // Make container focusable for keyboard shortcuts
    if (!this.container.hasAttribute("tabindex")) {
      this.container.setAttribute("tabindex", "0");
    }
    this.container.setAttribute("role", "region");
    this.container.setAttribute("aria-label", "Video player");

    // Initial captions track handling: disable all by default unless track.default
    this.initCaptions();
  }

  setSingleVideoSource(opts) {
    // Qualities: array of { label, src }
    if (opts.qualities && opts.qualities.length) {
      this.qualities = opts.qualities;
      this.currentQuality = opts.qualities.find(q => q.default) || opts.qualities[0];
      this.video.src = this.currentQuality.src;
    } else if (opts.src) {
      this.video.src = opts.src;
    }
    if (opts.poster) this.video.poster = opts.poster;
    if (opts.chapters) this.chapters = opts.chapters;
    if (opts.tracks) this.attachTracks(opts.tracks);
  }

  attachTracks(tracks) {
    // Remove existing tracks
    this.video.querySelectorAll("track").forEach(t => t.remove());
    tracks.forEach(t => {
      const trackEl = document.createElement("track");
      trackEl.kind = "subtitles";
      trackEl.label = t.label;
      trackEl.srclang = t.lang || t.srclang || "en";
      trackEl.src = t.src;
      if (t.default) trackEl.default = true;
      this.video.appendChild(trackEl);
    });
  }

  initCaptions() {
    // Wait for tracks to load
    const tracks = this.video.textTracks;
    if (!tracks) return;
    // Disable all initially; enable default if any
    let hasDefault = false;
    for (let i = 0; i < tracks.length; i++) {
      if (tracks[i].mode === "showing") hasDefault = true;
    }
    if (!hasDefault) {
      for (let i = 0; i < tracks.length; i++) {
        tracks[i].mode = "hidden";
      }
      this.captionsEnabled = false;
    } else {
      this.captionsEnabled = true;
      for (let i = 0; i < tracks.length; i++) {
        if (tracks[i].mode === "showing") this.activeCaptionsTrack = tracks[i];
      }
    }
    this.updateCaptionsButtonUI();
    this.buildCaptionsMenu();
  }

  // ========= Build Menus =========
  buildSettingsMenu() {
    if (!this.settingsMenu) return;
    // Determine available qualities
    let qualities = this.qualities;
    // If playlist, use current item's qualities
    if (this.playlist && this.playlist[this.currentIndex]?.qualities) {
      qualities = this.playlist[this.currentIndex].qualities;
    }
    if (!qualities && this.playlist) {
      // gather from playlist? fallback generic
      qualities = null;
    }

    const speeds = [0.25, 0.5, 0.75, 1, 1.25, 1.5, 1.75, 2];
    const currentSpeed = this.video.playbackRate;

    let qualityHTML = "";
    if (qualities && qualities.length) {
      qualityHTML = `
        <div class="mv-menu__section">
          <div class="mv-menu__label">Quality</div>
          ${qualities.map(q => `
            <button class="mv-menu__item ${this.currentQuality?.label === q.label ? "is-selected" : ""}" data-quality="${q.label}">
              <span>${q.label}</span>
              <span class="mv-menu__item-check" aria-hidden="true"></span>
            </button>
          `).join("")}
        </div>
      `;
    } else {
      // Still show simulated qualities for demo if none provided
      const demoQualities = ["Auto", "1080p", "720p", "480p", "360p"];
      const cur = this.currentQuality?.label || "Auto";
      qualityHTML = `
        <div class="mv-menu__section">
          <div class="mv-menu__label">Quality</div>
          ${demoQualities.map(l => `
            <button class="mv-menu__item ${cur===l ? "is-selected":""}" data-quality-demo="${l}">
              <span>${l}</span>
              <span class="mv-menu__item-check" aria-hidden="true"></span>
            </button>
          `).join("")}
        </div>
      `;
    }

    this.settingsMenu.innerHTML = `
      <div class="mv-menu__header">
        <div class="mv-menu__title">
          <span>${this.icons.settings}</span> Settings
        </div>
        <button class="mv-menu__close" data-mv="close-settings" aria-label="Close settings">${this.icons.close}</button>
      </div>
      <div class="mv-menu__section">
        <div class="mv-menu__label">Playback speed</div>
        ${speeds.map(s => `
          <button class="mv-menu__item ${currentSpeed===s ? "is-selected":""}" data-speed="${s}">
            <span>${s===1 ? "Normal" : s + "x"}</span>
            <span class="mv-menu__item-check" aria-hidden="true"></span>
          </button>
        `).join("")}
      </div>
      ${qualityHTML}
    `;

    // Bind menu item clicks
    this.settingsMenu.querySelectorAll("[data-speed]").forEach(btn => {
      btn.addEventListener("click", () => {
        const speed = parseFloat(btn.dataset.speed);
        this.setPlaybackSpeed(speed);
        this.closeAllMenus();
      });
    });
    this.settingsMenu.querySelectorAll("[data-quality]").forEach(btn => {
      btn.addEventListener("click", () => {
        const label = btn.dataset.quality;
        const q = qualities.find(x => x.label === label);
        if (q) this.setQuality(q);
        this.closeAllMenus();
      });
    });
    this.settingsMenu.querySelectorAll("[data-quality-demo]").forEach(btn => {
      btn.addEventListener("click", () => {
        const label = btn.dataset.qualityDemo;
        // Simulate quality switch (no real source change for demo)
        this.currentQuality = { label };
        // Show toast-like feedback via badge or console
        this.showTemporaryBadge(label);
        this.buildSettingsMenu(); // re-render to update selection
        this.closeAllMenus();
      });
    });
    const closeBtn = this.settingsMenu.querySelector("[data-mv='close-settings']");
    if (closeBtn) closeBtn.addEventListener("click", () => this.closeAllMenus());
  }

  buildCaptionsMenu() {
    if (!this.captionsMenu) return;
    const tracks = Array.from(this.video.textTracks || []);
    // Also check <track> elements for labels before loading
    const trackElements = Array.from(this.video.querySelectorAll("track"));
    let items = [];
    if (tracks.length) {
      items = tracks.map((t, idx) => ({
        label: t.label || trackElements[idx]?.label || `Track ${idx+1}`,
        lang: t.language || trackElements[idx]?.srclang || "",
        track: t,
        idx
      }));
    }

    // If no tracks, show empty state
    if (!items.length) {
      this.captionsMenu.innerHTML = `
        <div class="mv-menu__header">
          <div class="mv-menu__title">${this.icons.captions} Captions</div>
          <button class="mv-menu__close" data-mv="close-captions" aria-label="Close captions menu">${this.icons.close}</button>
        </div>
        <div class="mv-menu__section">
          <div style="padding:10px; font-size:13px; color: var(--mv-text-muted);">No captions available for this video.</div>
        </div>
      `;
      const closeBtn = this.captionsMenu.querySelector("[data-mv='close-captions']");
      if (closeBtn) closeBtn.addEventListener("click", () => this.closeAllMenus());
      return;
    }

    this.captionsMenu.innerHTML = `
      <div class="mv-menu__header">
        <div class="mv-menu__title">${this.icons.captions} Captions</div>
        <button class="mv-menu__close" data-mv="close-captions" aria-label="Close captions menu">${this.icons.close}</button>
      </div>
      <div class="mv-menu__section">
        <button class="mv-menu__item ${!this.captionsEnabled ? "is-selected":""}" data-caption-off="1">
          <span>Off</span>
          <span class="mv-menu__item-check" aria-hidden="true"></span>
        </button>
        ${items.map(it => `
          <button class="mv-menu__item ${this.captionsEnabled && this.activeCaptionsTrack===it.track ? "is-selected":""}" data-caption-idx="${it.idx}">
            <span>${it.label} ${it.lang ? `<small style="opacity:.6; margin-left:6px;">${it.lang}</small>` : ""}</span>
            <span class="mv-menu__item-check" aria-hidden="true"></span>
          </button>
        `).join("")}
      </div>
    `;

    this.captionsMenu.querySelector("[data-caption-off]")?.addEventListener("click", () => {
      this.disableCaptions();
      this.closeAllMenus();
    });
    this.captionsMenu.querySelectorAll("[data-caption-idx]").forEach(btn => {
      btn.addEventListener("click", () => {
        const idx = parseInt(btn.dataset.captionIdx, 10);
        this.enableCaptions(idx);
        this.closeAllMenus();
      });
    });
    const closeBtn = this.captionsMenu.querySelector("[data-mv='close-captions']");
    if (closeBtn) closeBtn.addEventListener("click", () => this.closeAllMenus());
  }

  renderChapterMarkers() {
    if (!this.progress) return;
    // Remove existing markers
    this.progress.querySelectorAll(".mv-chapter-marker").forEach(m => m.remove());
    const duration = this.video.duration || 0;
    if (!duration || !this.chapters.length) return;
    this.chapters.forEach(ch => {
      const pct = (ch.time / duration) * 100;
      if (pct < 0 || pct > 100) return;
      const marker = document.createElement("button");
      marker.className = "mv-chapter-marker";
      marker.style.left = pct + "%";
      marker.setAttribute("aria-label", `Jump to chapter: ${ch.title}`);
      marker.title = ch.title;
      marker.addEventListener("click", (e) => {
        e.stopPropagation();
        this.video.currentTime = ch.time;
      });
      marker.addEventListener("mouseenter", () => {
        // Show tooltip with chapter title
        if (this.tooltipChapter) this.tooltipChapter.textContent = ch.title;
      });
      this.progress.appendChild(marker);
    });
  }

  renderPlaylist() {
    if (!this.playlistListEl || !this.playlist) return;
    const frag = document.createDocumentFragment();
    this.playlistListEl.innerHTML = "";
    this.playlist.forEach((item, idx) => {
      const btn = document.createElement("button");
      btn.className = "mv-playlist__item" + (idx === this.currentIndex ? " is-active is-playing" : "");
      btn.setAttribute("aria-label", `Play: ${item.title}`);
      btn.innerHTML = `
        <div class="mv-playlist__thumb">
          <img src="${item.poster || ""}" alt="" loading="lazy" onerror="this.style.display='none'">
          ${item.duration ? `<span class="mv-playlist__thumb-duration">${item.duration}</span>` : ""}
        </div>
        <div class="mv-playlist__item-main">
          <div class="mv-playlist__item-title">${item.title}</div>
          ${item.subtitle ? `<div class="mv-playlist__item-subtitle">${item.subtitle}</div>` : ""}
        </div>
      `;
      btn.addEventListener("click", () => this.loadPlaylistItem(idx, true));
      frag.appendChild(btn);
    });
    this.playlistListEl.appendChild(frag);

    // Update meta count
    const metaEl = document.querySelector("[data-mv='playlist-meta']");
    if (metaEl) metaEl.textContent = `${this.playlist.length} videos`;

    // Prev/Next buttons state
    this.updatePlaylistNavButtons();
  }

  updatePlaylistNavButtons() {
    if (!this.prevBtn || !this.nextBtn) return;
    if (!this.playlist) {
      this.prevBtn.disabled = true;
      this.nextBtn.disabled = true;
      return;
    }
    // If loop disabled, disable at ends; if loop enabled, always enabled
    if (this.isLoopPlaylist) {
      this.prevBtn.disabled = false;
      this.nextBtn.disabled = false;
    } else {
      this.prevBtn.disabled = this.currentIndex === 0;
      this.nextBtn.disabled = this.currentIndex === this.playlist.length - 1;
    }
  }

  // ========= Event Binding =========
  bindEvents() {
    // Play/Pause buttons
    this.centerPlayBtn?.addEventListener("click", () => this.togglePlay());
    this.playBtn?.addEventListener("click", () => this.togglePlay());
    // Video click toggles play (but not when dragging menus)
    this.video.addEventListener("click", () => this.togglePlay());

    // Next/Prev
    this.prevBtn?.addEventListener("click", () => this.prev());
    this.nextBtn?.addEventListener("click", () => this.next());

    // Mute & volume
    this.muteBtn?.addEventListener("click", () => this.toggleMute());
    if (this.volumeSlider) {
      this.volumeSlider.addEventListener("input", (e) => this.setVolume(parseFloat(e.target.value)));
      // Expand on focus
      this.volumeSlider.addEventListener("focus", () => this.volumeGroup?.classList.add("is-expanded"));
      this.volumeSlider.addEventListener("blur", () => this.volumeGroup?.classList.remove("is-expanded"));
    }
    this.volumeGroup?.addEventListener("mouseenter", () => this.volumeGroup.classList.add("is-expanded"));
    this.volumeGroup?.addEventListener("mouseleave", () => {
      if (document.activeElement !== this.volumeSlider) this.volumeGroup.classList.remove("is-expanded");
    });

    // Fullscreen
    this.fullscreenBtn?.addEventListener("click", () => this.toggleFullscreen());
    document.addEventListener("fullscreenchange", () => this.onFullscreenChange());
    document.addEventListener("webkitfullscreenchange", () => this.onFullscreenChange());

    // PiP
    this.pipBtn?.addEventListener("click", () => this.togglePip());
    this.video.addEventListener("enterpictureinpicture", () => this.updatePipButton(true));
    this.video.addEventListener("leavepictureinpicture", () => this.updatePipButton(false));

    // Loop
    this.loopBtn?.addEventListener("click", () => this.toggleLoop());

    // Captions
    this.captionsBtn?.addEventListener("click", (e) => {
      e.stopPropagation();
      const isOpen = this.captionsMenu?.classList.contains("is-open");
      this.closeAllMenus();
      if (!isOpen) this.openMenu(this.captionsMenu);
      else this.updateCaptionsButtonUI();
    });

    // Settings
    this.settingsBtn?.addEventListener("click", (e) => {
      e.stopPropagation();
      const isOpen = this.settingsMenu?.classList.contains("is-open");
      this.closeAllMenus();
      if (!isOpen) {
        this.buildSettingsMenu();
        this.openMenu(this.settingsMenu);
      }
    });

    // Help (keyboard ?)
    this.helpBtn?.addEventListener("click", () => this.toggleHelp());
    this.helpOverlay?.querySelector("[data-mv='help-close']")?.addEventListener("click", () => this.closeHelp());

    // Close menus when clicking outside
    document.addEventListener("click", (e) => {
      if (!this.container.contains(e.target)) {
        this.closeAllMenus();
      } else {
        // If click is inside container but not on menu toggles, close menus if clicking elsewhere
        const isMenu = e.target.closest(".mv-menu");
        const isToggle = e.target.closest("[data-mv='settings']") || e.target.closest("[data-mv='captions']");
        if (!isMenu && !isToggle) this.closeAllMenus();
      }
    });

    // Autoplay toggle in playlist panel
    this.autoplayToggle?.addEventListener("change", (e) => {
      this.autoplayNext = e.target.checked;
    });

    // Video events
    this.video.addEventListener("loadedmetadata", () => this.onLoadedMetadata());
    this.video.addEventListener("timeupdate", () => this.onTimeUpdate());
    this.video.addEventListener("progress", () => this.onProgress());
    this.video.addEventListener("waiting", () => this.onWaiting());
    this.video.addEventListener("playing", () => this.onPlaying());
    this.video.addEventListener("play", () => this.onPlay());
    this.video.addEventListener("pause", () => this.onPause());
    this.video.addEventListener("ended", () => this.onEnded());
    this.video.addEventListener("volumechange", () => this.onVolumeChange());
    this.video.addEventListener("ratechange", () => this.onRateChange());
    this.video.addEventListener("error", (e) => this.onError(e));
    this.video.addEventListener("loadeddata", () => this.onProgress());

    // TextTracks cue change for custom display (optional)
    if (this.video.textTracks) {
      for (let i = 0; i < this.video.textTracks.length; i++) {
        const track = this.video.textTracks[i];
        track.addEventListener?.("cuechange", () => this.onCueChange(track));
      }
      // Also rebuild captions menu when tracks added (after load)
      this.video.addEventListener("loadedmetadata", () => {
        // Re-attach cuechange listeners for any new tracks
        for (let i = 0; i < this.video.textTracks.length; i++) {
          const track = this.video.textTracks[i];
          if (!track._mvBound) {
            track.addEventListener("cuechange", () => this.onCueChange(track));
            track._mvBound = true;
          }
        }
        this.buildCaptionsMenu();
      });
    }

    // Progress bar interactions
    this.setupProgressEvents();

    // Keyboard shortcuts
    this.container.addEventListener("keydown", (e) => this.handleKeyboard(e));
    // Also global when player is hovered/focused? Use container focus only for accessibility, but also allow document-level when player in viewport and user presses shortcuts
    // We keep container focus requirement for most keys, but allow Space/Mute etc when video is playing and player is hovered

    // Gestures & auto-hide
    this.setupGestures();
    this.setupAutoHide();

    // Timeupdate throttling not needed; native is ~250ms
  }

  // ========= Video Event Handlers =========
  onLoadedMetadata() {
    const d = this.video.duration;
    this.updateTimeDisplay(this.video.currentTime, d);
    this.onProgress();
    if (this.chapters.length) this.renderChapterMarkers();
    // If playlist item has its own chapters, update
    const currentItem = this.playlist?.[this.currentIndex];
    if (currentItem?.chapters) {
      this.chapters = currentItem.chapters;
      this.renderChapterMarkers();
    }
  }

  onTimeUpdate() {
    if (this.isDragging) return;
    const current = this.video.currentTime;
    const duration = this.video.duration || 0;
    this.updateTimeDisplay(current, duration);
    this.updateProgressFilled(current, duration);
  }

  onProgress() {
    const video = this.video;
    if (!video.buffered || !video.buffered.length || !video.duration) {
      if (this.progressBuffered) this.progressBuffered.style.width = "0%";
      return;
    }
    try {
      // Find the buffered end that contains currentTime or the furthest
      let bufferedEnd = 0;
      for (let i = 0; i < video.buffered.length; i++) {
        const end = video.buffered.end(i);
        if (end > bufferedEnd) bufferedEnd = end;
      }
      const pct = (bufferedEnd / video.duration) * 100;
      if (this.progressBuffered) this.progressBuffered.style.width = Math.min(100, pct) + "%";
    } catch (e) {
      // Some browsers may throw if not ready
    }
  }

  onWaiting() {
    this.container.classList.add("mv-player--loading");
  }
  onPlaying() {
    this.container.classList.remove("mv-player--loading");
  }
  onPlay() {
    this.container.classList.add("mv-player--playing");
    this.container.classList.remove("mv-player--paused");
    this.updatePlayButtonUI();
    this.resetHideTimer();
  }
  onPause() {
    this.container.classList.remove("mv-player--playing");
    this.container.classList.add("mv-player--paused");
    this.updatePlayButtonUI();
    this.showControls();
  }
  onEnded() {
    this.container.classList.remove("mv-player--playing");
    this.updatePlayButtonUI();
    this.showControls();
    if (this.isLoopPlaylist && this.playlist && this.playlist.length === 1) {
      // single video loop handled by video.loop
    }
    if (this.playlist && this.autoplayNext) {
      this.next(true); // autoplay next
    } else if (this.video.loop) {
      this.video.play();
    }
  }
  onVolumeChange() {
    this.updateVolumeUI();
  }
  onRateChange() {
    this.updateSettingsBadge();
  }
  onError(e) {
    console.warn("Video error:", e);
    this.container.classList.remove("mv-player--loading");
  }
  onFullscreenChange() {
    const isFs = !!(document.fullscreenElement === this.container || document.webkitFullscreenElement === this.container);
    this.container.classList.toggle("is-fullscreen", isFs);
    if (this.fullscreenBtn) {
      this.fullscreenBtn.setAttribute("aria-label", isFs ? "Exit fullscreen (F)" : "Enter fullscreen (F)");
      this.fullscreenBtn.innerHTML = isFs ? this.icons.fullscreenExit : this.icons.fullscreenEnter;
    }
  }
  onCueChange(track) {
    // Show custom cue if desired; native cues already shown by browser.
    // We use this to update our custom cue overlay for styled captions (optional).
    if (track.mode !== "showing") {
      if (this.activeCaptionsTrack === track) {
        if (this.captionsCue) {
          this.captionsCue.style.opacity = "0";
          this.captionsCue.textContent = "";
        }
      }
      return;
    }
    const cues = track.activeCues;
    if (cues && cues.length > 0) {
      const text = Array.from(cues).map(c => c.text).join("\n");
      if (this.captionsCue) {
        this.captionsCue.textContent = text;
        this.captionsCue.style.opacity = text ? "1" : "0";
      }
    } else {
      if (this.captionsCue) {
        this.captionsCue.style.opacity = "0";
        this.captionsCue.textContent = "";
      }
    }
  }

  // ========= UI Updaters =========
  updatePlayButtonUI() {
    const isPlaying = !this.video.paused && !this.video.ended;
    const icon = isPlaying ? this.icons.pause : this.icons.play;
    const label = isPlaying ? "Pause (Space)" : "Play (Space)";
    if (this.playBtn) {
      this.playBtn.innerHTML = icon;
      this.playBtn.setAttribute("aria-label", label);
      this.playBtn.setAttribute("aria-pressed", String(isPlaying));
    }
    if (this.centerPlayBtn) {
      this.centerPlayBtn.innerHTML = icon;
      this.centerPlayBtn.setAttribute("aria-label", label);
      this.centerPlayBtn.dataset.state = isPlaying ? "pause" : "play";
    }
  }

  updateTimeDisplay(current, duration) {
    const fmtCurrent = this.formatTime(current);
    const fmtTotal = this.formatTime(duration || this.video.duration || 0);
    if (this.timeCurrent) this.timeCurrent.textContent = fmtCurrent;
    if (this.timeTotal) this.timeTotal.textContent = fmtTotal;
  }

  updateProgressFilled(current, duration) {
    if (!this.progressFilled) return;
    const pct = duration ? (current / duration) * 100 : 0;
    this.progressFilled.style.width = Math.max(0, Math.min(100, pct)) + "%";
  }

  updateVolumeUI() {
    const vol = this.video.volume;
    const muted = this.video.muted || vol === 0;
    let icon = this.icons.volumeHigh;
    if (muted) icon = this.icons.volumeMute;
    else if (vol < 0.5) icon = this.icons.volumeLow;

    if (this.muteBtn) {
      this.muteBtn.innerHTML = icon;
      this.muteBtn.setAttribute("aria-label", muted ? "Unmute (M)" : "Mute (M)");
      this.muteBtn.setAttribute("aria-pressed", String(muted));
    }
    if (this.volumeSlider) {
      this.volumeSlider.value = muted ? 0 : vol;
      // Update slider fill via background gradient
      const pct = (muted ? 0 : vol) * 100;
      this.volumeSlider.style.background = `linear-gradient(to right, var(--mv-accent) 0%, var(--mv-accent) ${pct}%, rgba(255,255,255,.22) ${pct}%, rgba(255,255,255,.22) 100%)`;
    }
  }

  updateLoopUI() {
    const isLoop = this.video.loop || this.isLoopPlaylist;
    if (this.loopBtn) {
      this.loopBtn.classList.toggle("is-active", isLoop);
      this.loopBtn.setAttribute("aria-pressed", String(isLoop));
      this.loopBtn.setAttribute("aria-label", isLoop ? "Disable loop" : "Enable loop");
    }
  }

  updateCaptionsButtonUI() {
    if (!this.captionsBtn) return;
    const enabled = this.captionsEnabled;
    this.captionsBtn.classList.toggle("is-active", enabled);
    this.captionsBtn.setAttribute("aria-pressed", String(enabled));
    this.captionsBtn.setAttribute("aria-label", enabled ? "Hide captions (C)" : "Show captions (C)");
  }

  updatePipSupport() {
    if (!this.pipBtn) return;
    const supported = document.pictureInPictureEnabled && !this.video.disablePictureInPicture;
    if (!supported) {
      this.pipBtn.style.display = "none";
      this.pipBtn.setAttribute("aria-hidden", "true");
    } else {
      this.pipBtn.style.display = "";
    }
  }

  updatePipButton(isInPip) {
    if (!this.pipBtn) return;
    this.pipBtn.classList.toggle("is-active", isInPip);
    this.pipBtn.setAttribute("aria-pressed", String(isInPip));
  }

  updateSettingsBadge() {
    // Show speed badge on settings button if not 1x
    if (!this.settingsBtn) return;
    let badge = this.settingsBtn.querySelector(".mv-badge");
    const rate = this.video.playbackRate;
    if (rate !== 1) {
      if (!badge) {
        badge = document.createElement("span");
        badge.className = "mv-badge";
        this.settingsBtn.appendChild(badge);
      }
      badge.textContent = rate + "x";
    } else {
      badge?.remove();
    }
  }

  showTemporaryBadge(text) {
    this.showTemporaryTooltip(text);
    this.updateSettingsBadge();
  }

  showTemporaryTooltip(text) {
    // Simple toast near settings button
    const toast = document.createElement("div");
    toast.textContent = text;
    toast.style.cssText = "position:absolute; right:12px; bottom:70px; background:rgba(20,20,20,.92); color:#fff; padding:8px 12px; border-radius:8px; font-size:13px; font-weight:600; z-index:12; border:1px solid rgba(255,255,255,.08); box-shadow:0 8px 24px rgba(0,0,0,.4);";
    this.container.appendChild(toast);
    setTimeout(() => {
      toast.style.transition = "opacity .2s ease, transform .2s ease";
      toast.style.opacity = "0";
      toast.style.transform = "translateY(6px)";
      setTimeout(() => toast.remove(), 220);
    }, 1200);
  }

  formatTime(sec) {
    if (!isFinite(sec) || isNaN(sec) || sec < 0) sec = 0;
    const h = Math.floor(sec / 3600);
    const m = Math.floor((sec % 3600) / 60);
    const s = Math.floor(sec % 60);
    if (h > 0) {
      return `${h}:${String(m).padStart(2,"0")}:${String(s).padStart(2,"0")}`;
    }
    return `${m}:${String(s).padStart(2,"0")}`;
  }

  // ========= Controls Actions =========
  togglePlay() {
    if (this.video.paused || this.video.ended) {
      const p = this.video.play();
      if (p && p.catch) p.catch(() => {}); // autoplay may be blocked
    } else {
      this.video.pause();
    }
  }

  seek(time) {
    if (!isFinite(time)) return;
    const d = this.video.duration || 0;
    this.video.currentTime = Math.max(0, Math.min(time, d || time));
  }

  seekBy(delta) {
    this.seek(this.video.currentTime + delta);
    this.showControls();
    this.resetHideTimer();
  }

  setVolume(v) {
    const vol = Math.max(0, Math.min(1, v));
    this.video.volume = vol;
    this.video.muted = vol === 0;
    if (vol > 0) this.lastVolume = vol;
    this.updateVolumeUI();
  }

  toggleMute() {
    if (this.video.muted || this.video.volume === 0) {
      this.video.muted = false;
      this.video.volume = this.lastVolume || 0.8;
    } else {
      this.lastVolume = this.video.volume || 0.8;
      this.video.muted = true;
    }
  }

  async toggleFullscreen() {
    const isFs = !!(document.fullscreenElement || document.webkitFullscreenElement);
    try {
      if (!isFs) {
        if (this.container.requestFullscreen) await this.container.requestFullscreen();
        else if (this.container.webkitRequestFullscreen) await this.container.webkitRequestFullscreen();
        else if (this.video.webkitEnterFullscreen) this.video.webkitEnterFullscreen(); // iOS Safari
      } else {
        if (document.exitFullscreen) await document.exitFullscreen();
        else if (document.webkitExitFullscreen) await document.webkitExitFullscreen();
      }
    } catch (e) {
      console.warn("Fullscreen error:", e);
    }
  }

  async togglePip() {
    try {
      if (document.pictureInPictureElement === this.video) {
        await document.exitPictureInPicture();
      } else if (document.pictureInPictureEnabled && this.video.requestPictureInPicture) {
        await this.video.requestPictureInPicture();
      }
    } catch (e) {
      console.warn("PiP error:", e);
    }
  }

  toggleLoop() {
    // If playlist exists, toggle playlist loop; otherwise toggle video.loop
    if (this.playlist && this.playlist.length > 1) {
      this.isLoopPlaylist = !this.isLoopPlaylist;
      this.video.loop = false;
    } else {
      this.video.loop = !this.video.loop;
      this.isLoopPlaylist = this.video.loop;
    }
    this.updateLoopUI();
    this.updatePlaylistNavButtons();
  }

  setPlaybackSpeed(rate) {
    this.video.playbackRate = rate;
    this.showTemporaryBadge(rate + "x");
    this.buildSettingsMenu();
  }

  setQuality(q) {
    if (!q || !q.src) return;
    const wasPlaying = !this.video.paused;
    const currentTime = this.video.currentTime;
    this.currentQuality = q;
    this.video.src = q.src;
    // Re-attach tracks if needed, then seek
    this.video.addEventListener("loadedmetadata", () => {
      this.video.currentTime = currentTime;
      if (wasPlaying) this.video.play().catch(()=>{});
    }, { once: true });
    this.video.load();
    this.buildSettingsMenu();
    this.showTemporaryBadge(q.label);
  }

  enableCaptions(idx) {
    const tracks = this.video.textTracks;
    if (!tracks || !tracks[idx]) return;
    for (let i = 0; i < tracks.length; i++) tracks[i].mode = "hidden";
    tracks[idx].mode = "showing";
    this.captionsEnabled = true;
    this.activeCaptionsTrack = tracks[idx];
    this.updateCaptionsButtonUI();
    this.buildCaptionsMenu();
  }

  disableCaptions() {
    const tracks = this.video.textTracks;
    if (!tracks) return;
    for (let i = 0; i < tracks.length; i++) tracks[i].mode = "hidden";
    this.captionsEnabled = false;
    this.activeCaptionsTrack = null;
    if (this.captionsCue) {
      this.captionsCue.style.opacity = "0";
      this.captionsCue.textContent = "";
    }
    this.updateCaptionsButtonUI();
    this.buildCaptionsMenu();
  }

  toggleCaptions() {
    if (this.captionsEnabled) this.disableCaptions();
    else {
      const tracks = this.video.textTracks;
      if (tracks && tracks.length) this.enableCaptions(0);
    }
  }

  // ========= Progress Events =========
  setupProgressEvents() {
    if (!this.progressWrap) return;

    const getTimeFromEvent = (clientX) => {
      const rect = this.progress.getBoundingClientRect();
      const x = Math.max(0, Math.min(clientX - rect.left, rect.width));
      const pct = rect.width ? x / rect.width : 0;
      const duration = this.video.duration || 0;
      return { pct, time: pct * duration, rect, x };
    };

    const updateTooltip = (clientX) => {
      const { pct, time, rect, x } = getTimeFromEvent(clientX);
      const formatted = this.formatTime(time);
      if (this.tooltipTime) this.tooltipTime.textContent = formatted;
      // Find chapter for that time
      let chapterTitle = "";
      if (this.chapters.length) {
        // Find chapter that contains time or closest before
        let found = null;
        for (let i = this.chapters.length -1; i >=0; i--) {
          if (time >= this.chapters[i].time) { found = this.chapters[i]; break; }
        }
        if (found) chapterTitle = found.title;
      }
      if (this.tooltipChapter) {
        this.tooltipChapter.textContent = chapterTitle;
        this.tooltipChapter.style.display = chapterTitle ? "block" : "none";
      }
      if (this.tooltip) {
        this.tooltip.style.left = x + "px";
        this.tooltip.classList.add("is-visible");
        // Keep tooltip inside progress bounds (clamp)
        const tooltipRect = this.tooltip.getBoundingClientRect();
        // No need to clamp left style; CSS transform handles centering, but we can adjust if near edges
        // Simple clamp: ensure not overflow outside container
        // (tooltip is positioned with translateX(-50%), we adjust left to keep visible)
        const progressRect = this.progress.getBoundingClientRect();
        const containerRect = this.container.getBoundingClientRect();
        // If tooltip would overflow left/right, adjust
        // We keep it simple: if x < 40, shift; if x > rect.width -40, shift
      }
    };

    const onPointerMove = (e) => {
      const clientX = e.touches ? e.touches[0].clientX : e.clientX;
      if (this.isDragging) {
        const { time } = getTimeFromEvent(clientX);
        this.updateTimeDisplay(time, this.video.duration);
        this.updateProgressFilled(time, this.video.duration);
      } else {
        updateTooltip(clientX);
      }
    };

    const onPointerEnter = (e) => {
      const clientX = e.touches ? e.touches[0].clientX : e.clientX;
      updateTooltip(clientX);
    };

    const onPointerLeave = () => {
      if (!this.isDragging && this.tooltip) this.tooltip.classList.remove("is-visible");
    };

    // Mouse events
    this.progressWrap.addEventListener("mouseenter", onPointerEnter);
    this.progressWrap.addEventListener("mousemove", onPointerMove);
    this.progressWrap.addEventListener("mouseleave", onPointerLeave);

    // Touch events for tooltip (show on touch move)
    this.progressWrap.addEventListener("touchstart", (e) => {
      onPointerEnter(e);
    }, { passive: true });
    this.progressWrap.addEventListener("touchmove", onPointerMove, { passive: true });
    this.progressWrap.addEventListener("touchend", () => {
      setTimeout(() => this.tooltip?.classList.remove("is-visible"), 800);
    });

    // Dragging / seeking via Pointer Events
    const onPointerDown = (e) => {
      if (e.button !== undefined && e.button !== 0) return; // only left click
      this.isDragging = true;
      this.wasPlayingBeforeDrag = !this.video.paused;
      this.progressWrap.classList.add("is-dragging");
      this.progressWrap.setPointerCapture?.(e.pointerId);
      const clientX = e.clientX;
      const { time } = getTimeFromEvent(clientX);
      this.updateTimeDisplay(time, this.video.duration);
      this.updateProgressFilled(time, this.video.duration);
      updateTooltip(clientX);
      e.preventDefault();
    };

    const onPointerMoveDrag = (e) => {
      if (!this.isDragging) return;
      const clientX = e.clientX;
      const { time } = getTimeFromEvent(clientX);
      this.updateTimeDisplay(time, this.video.duration);
      this.updateProgressFilled(time, this.video.duration);
      updateTooltip(clientX);
    };

    const onPointerUp = (e) => {
      if (!this.isDragging) return;
      this.isDragging = false;
      this.progressWrap.classList.remove("is-dragging");
      const clientX = e.clientX;
      const { time } = getTimeFromEvent(clientX);
      this.seek(time);
      if (this.wasPlayingBeforeDrag) this.video.play().catch(()=>{});
      this.tooltip?.classList.remove("is-visible");
      // Release capture if needed
      try { this.progressWrap.releasePointerCapture?.(e.pointerId); } catch {}
    };

    this.progressWrap.addEventListener("pointerdown", onPointerDown);
    window.addEventListener("pointermove", onPointerMoveDrag);
    window.addEventListener("pointerup", onPointerUp);
    // Fallback for touch without pointer events? Pointer events cover touch in modern browsers
    // Click to seek (if not dragging, also handled by pointerup)
    this.progressWrap.addEventListener("click", (e) => {
      // Avoid double handling when dragging
      if (this.isDragging) return;
      const { time } = getTimeFromEvent(e.clientX);
      this.seek(time);
    });

    // Keyboard accessibility for progress: allow arrow keys when focused
    this.progressWrap.setAttribute("tabindex", "0");
    this.progressWrap.setAttribute("role", "slider");
    this.progressWrap.setAttribute("aria-label", "Seek");
    this.progressWrap.setAttribute("aria-valuemin", "0");
    this.progressWrap.setAttribute("aria-valuemax", "100");
    this.progressWrap.addEventListener("keydown", (e) => {
      const step = 5; // seconds
      if (e.key === "ArrowLeft" || e.key === "ArrowDown") {
        e.preventDefault();
        this.seekBy(-step);
      } else if (e.key === "ArrowRight" || e.key === "ArrowUp") {
        e.preventDefault();
        this.seekBy(step);
      } else if (e.key === "Home") {
        e.preventDefault(); this.seek(0);
      } else if (e.key === "End") {
        e.preventDefault(); this.seek(this.video.duration || 0);
      }
    });
    // Update aria-valuenow on timeupdate
    this.video.addEventListener("timeupdate", () => {
      const pct = this.video.duration ? (this.video.currentTime / this.video.duration) * 100 : 0;
      this.progressWrap.setAttribute("aria-valuenow", String(Math.round(pct)));
      this.progressWrap.setAttribute("aria-valuetext", `${this.formatTime(this.video.currentTime)} of ${this.formatTime(this.video.duration)}`);
    });
  }

  // ========= Playlist Logic =========
  loadPlaylistItem(index, autoplay = true) {
    if (!this.playlist || index < 0 || index >= this.playlist.length) return;
    this.currentIndex = index;
    const item = this.playlist[index];

    // Preserve volume/muted/loop/rate? Keep them
    const wasMuted = this.video.muted;
    const wasVolume = this.video.volume;
    const wasLoop = this.video.loop;
    const wasRate = this.video.playbackRate;

    // Qualities for this item
    if (item.qualities && item.qualities.length) {
      this.qualities = item.qualities;
      this.currentQuality = item.qualities.find(q => q.default) || item.qualities[0];
      this.video.src = this.currentQuality.src;
    } else if (item.src) {
      this.qualities = null;
      this.currentQuality = null;
      this.video.src = item.src;
    }

    if (item.poster) this.video.poster = item.poster;
    else this.video.removeAttribute("poster");

    // Chapters
    this.chapters = item.chapters || [];
    // Tracks
    if (item.tracks) {
      this.attachTracks(item.tracks);
    } else {
      // remove old tracks if no tracks for new item
      this.video.querySelectorAll("track").forEach(t => t.remove());
    }

    // Reset captions state
    this.captionsEnabled = false;
    this.activeCaptionsTrack = null;
    this.updateCaptionsButtonUI();
    this.buildCaptionsMenu();

    // Restore settings after src change
    this.video.muted = wasMuted;
    this.video.volume = wasVolume;
    this.video.playbackRate = wasRate;
    // Loop handled separately for playlist

    this.video.load();
    this.renderPlaylist();
    this.buildSettingsMenu();
    this.renderChapterMarkers();

    // Update URL hash for sharing? Optional
    // Update playlist UI selection
    this.updatePlaylistNavButtons();

    if (autoplay) {
      // Wait for canplay then play
      const playOnReady = () => this.video.play().catch(()=>{});
      this.video.addEventListener("canplay", playOnReady, { once: true });
      // Fallback: try immediate
      this.video.play().catch(()=>{});
    }

    // Announce for screen readers? Use aria-live
    this.announce(`Now playing: ${item.title}`);
  }

  next(isAutoplay = false) {
    if (!this.playlist) return;
    let nextIndex = this.currentIndex + 1;
    if (nextIndex >= this.playlist.length) {
      if (this.isLoopPlaylist) nextIndex = 0;
      else {
        if (isAutoplay) return; // do not loop if not enabled
        nextIndex = this.playlist.length - 1;
      }
    }
    this.loadPlaylistItem(nextIndex, true);
  }

  prev() {
    if (!this.playlist) return;
    let prevIndex = this.currentIndex - 1;
    if (prevIndex < 0) {
      if (this.isLoopPlaylist) prevIndex = this.playlist.length - 1;
      else prevIndex = 0;
    }
    // If current time > 3s, restart current video instead (YouTube behavior)
    if (this.video.currentTime > 3 && !this.isLoopPlaylist) {
      this.seek(0);
      return;
    }
    this.loadPlaylistItem(prevIndex, true);
  }

  announce(msg) {
    let live = document.getElementById("mv-aria-live");
    if (!live) {
      live = document.createElement("div");
      live.id = "mv-aria-live";
      live.setAttribute("aria-live", "polite");
      live.setAttribute("aria-atomic", "true");
      live.style.cssText = "position:absolute; width:1px; height:1px; overflow:hidden; clip:rect(0,0,0,0); white-space:nowrap;";
      document.body.appendChild(live);
    }
    live.textContent = msg;
    setTimeout(() => live.textContent = "", 1000);
  }

  // ========= Keyboard =========
  handleKeyboard(e) {
    // Ignore if typing in input/textarea/contenteditable
    const target = e.target;
    if (target.closest && target.closest("input, textarea, select, [contenteditable='true']")) return;
    // Ignore if modifier keys (except Shift for ?)
    if (e.ctrlKey || e.altKey || e.metaKey) return;

    // Help toggle with ?
    if (e.key === "?" || (e.key === "/" && e.shiftKey)) {
      e.preventDefault();
      this.toggleHelp();
      return;
    }
    if (e.key === "Escape") {
      if (this.helpOverlay?.classList.contains("is-open")) {
        this.closeHelp();
        e.preventDefault();
        return;
      }
      if (this.settingsMenu?.classList.contains("is-open") || this.captionsMenu?.classList.contains("is-open")) {
        this.closeAllMenus();
        e.preventDefault();
        return;
      }
      if (document.fullscreenElement === this.container) {
        this.toggleFullscreen();
        e.preventDefault();
        return;
      }
    }

    // Only handle shortcuts when player is focused or hovered? For accessibility we handle when container has focus
    const isFocused = this.container.contains(document.activeElement) || this.container.matches(":hover") || this.container === document.activeElement;
    // If not focused and video not in viewport, ignore. Allow global when playing? We'll require focus to avoid hijacking page.
    if (!isFocused && !this.container.contains(document.activeElement)) {
      // Still allow Space when video is focused? Check if video element is activeElement
      if (document.activeElement !== this.video && document.activeElement !== this.container) return;
    }

    switch (e.key.toLowerCase()) {
      case " ":
      case "enter":
        // Space should not scroll page
        e.preventDefault();
        this.togglePlay();
        break;
      case "k": // YouTube-style
        e.preventDefault();
        this.togglePlay();
        break;
      case "m":
        e.preventDefault();
        this.toggleMute();
        break;
      case "f":
        e.preventDefault();
        this.toggleFullscreen();
        break;
      case "c":
        e.preventDefault();
        this.toggleCaptions();
        break;
      case "i":
        if (e.shiftKey) break;
        e.preventDefault();
        this.togglePip();
        break;
      case "l":
        e.preventDefault();
        this.toggleLoop();
        break;
      case "arrowleft":
        e.preventDefault();
        this.seekBy(e.shiftKey ? -10 : -5);
        this.showSeekFeedback(-(e.shiftKey ? 10 : 5));
        break;
      case "arrowright":
        e.preventDefault();
        this.seekBy(e.shiftKey ? 10 : 5);
        this.showSeekFeedback(e.shiftKey ? 10 : 5);
        break;
      case "arrowup":
        e.preventDefault();
        this.setVolume(Math.min(1, this.video.volume + 0.05));
        break;
      case "arrowdown":
        e.preventDefault();
        this.setVolume(Math.max(0, this.video.volume - 0.05));
        break;
      case ",":
        // Frame step? Or speed down
        if (e.shiftKey) {
          e.preventDefault();
          this.setVolume(Math.max(0, this.video.volume - 0.05));
        }
        break;
      case ".":
        if (e.shiftKey) {
          e.preventDefault();
          this.setVolume(Math.min(1, this.video.volume + 0.05));
        }
        break;
      default:
        // Number keys 0-9: seek to percentage
        if (/^[0-9]$/.test(e.key)) {
          e.preventDefault();
          const pct = parseInt(e.key, 10) / 10;
          const duration = this.video.duration || 0;
          this.seek(duration * pct);
        }
        break;
    }
  }

  // ========= Gestures & Auto-hide =========
  setupGestures() {
    if (!this.tapZoneLeft || !this.tapZoneRight || !this.tapZoneCenter) return;

    let lastTap = 0;
    let lastTapX = 0;
    // Single tap center toggles controls / play? Spec: single tap show/hide controls
    // We'll implement: single tap in center area toggles controls visibility (if hidden, show; else if playing, hide after delay)
    const handleTap = (e) => {
      const now = Date.now();
      const dt = now - lastTap;
      lastTap = now;
      // Show controls on any tap
      this.showControls();
      this.resetHideTimer();
    };

    this.tapZoneCenter.addEventListener("click", (e) => {
      handleTap(e);
      // If controls were hidden and video is paused, also toggle play? Keep simple: tap center toggles play when paused, otherwise shows controls
      // For mobile, single tap should show/hide controls, not play/pause. We'll not toggle play here.
      // But allow tap to play if paused and controls visible? We'll toggle play on center tap only if video is paused
      if (this.video.paused) {
        // Small delay to distinguish double tap? We'll toggle play
        setTimeout(() => {
          // If not double-tapped recently
          if (Date.now() - lastTap > 300) this.togglePlay();
        }, 220);
      }
    });

    // Double tap left/right to seek ±10s
    const setupDoubleTap = (zone, direction) => {
      let lastTapTime = 0;
      zone.addEventListener("click", (e) => {
        const now = Date.now();
        const timeDiff = now - lastTapTime;
        lastTapTime = now;
        if (timeDiff < 350 && timeDiff > 0) {
          // Double tap detected
          e.preventDefault();
          this.seekBy(direction * 10);
          this.showSeekFeedback(direction * 10);
          this.showControls();
          this.resetHideTimer();
          // Prevent single tap action
          lastTap = 0; // reset center tap logic
        }
      });
      // Also handle touchend for better responsiveness
      zone.addEventListener("touchend", (e) => {
        // Prevent click delay issues; handled by click above
      }, { passive: true });
    };

    setupDoubleTap(this.tapZoneLeft, -1);
    setupDoubleTap(this.tapZoneRight, 1);

    // Also support double-click for desktop
    this.tapZoneLeft.addEventListener("dblclick", (e) => {
      e.preventDefault();
      this.seekBy(-10);
      this.showSeekFeedback(-10);
    });
    this.tapZoneRight.addEventListener("dblclick", (e) => {
      e.preventDefault();
      this.seekBy(10);
      this.showSeekFeedback(10);
    });
  }

  showSeekFeedback(delta) {
    const el = delta < 0 ? this.feedbackLeft : this.feedbackRight;
    if (!el) return;
    el.classList.add("is-visible");
    // Update text if needed
    const span = el.querySelector("span");
    if (span) span.textContent = (delta > 0 ? "+" : "") + delta + "s";
    clearTimeout(el._hideTimer);
    el._hideTimer = setTimeout(() => el.classList.remove("is-visible"), 600);
  }

  setupAutoHide() {
    const events = ["mousemove", "mousedown", "keydown", "touchstart", "touchmove", "wheel"];
    events.forEach(ev => {
      this.container.addEventListener(ev, () => {
        this.showControls();
        this.resetHideTimer();
      }, { passive: true });
    });
    this.container.addEventListener("mouseleave", () => {
      if (!this.video.paused) this.resetHideTimer();
    });
    this.container.addEventListener("mouseenter", () => {
      this.showControls();
      this.resetHideTimer();
    });
  }

  showControls() {
    this.container.classList.remove("mv-player--idle");
  }

  hideControls() {
    if (this.video.paused) return; // don't hide when paused
    if (this.settingsMenu?.classList.contains("is-open") || this.captionsMenu?.classList.contains("is-open") || this.helpOverlay?.classList.contains("is-open")) return;
    // Don't hide if hovering controls
    const isHoveringControls = this.container.querySelector(".mv-controls:hover");
    if (isHoveringControls) {
      this.resetHideTimer();
      return;
    }
    this.container.classList.add("mv-player--idle");
  }

  resetHideTimer() {
    clearTimeout(this.hideControlsTimer);
    this.showControls();
    if (!this.video.paused) {
      this.hideControlsTimer = setTimeout(() => this.hideControls(), 2800);
    }
  }

  // ========= Menus & Help =========
  openMenu(menuEl) {
    if (!menuEl) return;
    menuEl.classList.add("is-open");
    menuEl.setAttribute("aria-hidden", "false");
    // Focus first item for keyboard nav
    const firstItem = menuEl.querySelector("button");
    firstItem?.focus();
  }

  closeAllMenus() {
    [this.settingsMenu, this.captionsMenu].forEach(m => {
      if (m) {
        m.classList.remove("is-open");
        m.setAttribute("aria-hidden", "true");
      }
    });
  }

  toggleHelp() {
    if (!this.helpOverlay) return;
    const isOpen = this.helpOverlay.classList.toggle("is-open");
    this.helpOverlay.setAttribute("aria-hidden", String(!isOpen));
    if (isOpen) {
      this.helpOverlay.querySelector("button")?.focus();
    }
  }
  closeHelp() {
    if (!this.helpOverlay) return;
    this.helpOverlay.classList.remove("is-open");
    this.helpOverlay.setAttribute("aria-hidden", "true");
    this.container.focus();
  }

  // ========= Destroy =========
  destroy() {
    clearTimeout(this.hideControlsTimer);
    // Remove listeners? For simplicity not fully implemented; for SPA use, recreate instance
  }
}

// Expose globally for easy integration
window.ModernVideoPlayer = ModernVideoPlayer;

// Auto-initialize demo if data-auto-init present
document.addEventListener("DOMContentLoaded", () => {
  const autoContainers = document.querySelectorAll("[data-mv-auto-init]");
  autoContainers.forEach(container => {
    // Options may be passed via data attributes or window.MV_DEMO_OPTIONS
    const opts = window.MV_DEMO_OPTIONS || {};
    // Allow per-container JSON options via data-options
    let perOpts = {};
    try {
      if (container.dataset.options) perOpts = JSON.parse(container.dataset.options);
    } catch {}
    new ModernVideoPlayer(container, { ...opts, ...perOpts });
  });
});
