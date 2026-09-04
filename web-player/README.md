# Modern Video Player — Vanilla HTML/CSS/JS

A modern, responsive HTML5 `<video>` player with **custom controls** (no browser defaults), dark theme, fully accessible and touch-ready. Drop-in for static HTML, Laravel Blade or React.

## Features

### Must-have (core)
- **Play / Pause** — central idle button + control-bar button, icon morphs
- **Progress / Seek** — buffered range, dragged seek, hover tooltip with time + chapter title
- **Time indicators** — `current / total` next to progress
- **Volume & Mute** — icon toggles mute, horizontal slider updates `video.volume`
- **Fullscreen** — Fullscreen API, works on desktop & mobile
- **Captions** — toggle button + menu for multiple `<track kind="subtitles">`
- **Settings (gear)** — playback speed `0.25x–2x`, quality selector (simulated source switch)

### Advanced UX
- **Playlist** — side panel (desktop) / stacked (mobile), Prev/Next buttons, autoplay next
- **Chapters** — yellow markers on progress bar, hover shows title
- **Loop / Repeat** — toggles `video.loop` or playlist loop
- **Picture-in-Picture** — PiP API with graceful fallback
- **Keyboard shortcuts** — `Space/K` play, `←/→` seek 5s (Shift 10s), `↑/↓` volume, `F` fullscreen, `M` mute, `C` captions, `0–9` jump, `?` help
- **Mobile gestures** — single tap show/hide controls, double-tap left/right ±10s, 44×44px targets
- **Accessibility** — Tab navigation, focus rings, `aria-label`/`aria-pressed`/`role="slider"`

### Design
- Dark chrome: very dark gray `#0f0f0f` (not pure black), `1px` border `#2a2a2a`
- Accent `#00d4aa` (turquoise) for progress & active states
- Auto-hide controls after ~2.8s inactivity; reappear on mouse move / tap / focus
- Inter font, 16:9 responsive, `backdrop-filter` blur, rounded `16px`

## Files

```
web-player/
├── index.html          # Demo page + full HTML structure + integration guide
├── css/player.css      # Dark theme, layout, responsive, animations
├── js/player.js        # Vanilla class ModernVideoPlayer (~800 lines, commented)
└── examples/
    ├── react-player.jsx
    └── blade-player.blade.php
```

## Quick start — Static HTML

1. Copy `css/player.css` and `js/player.js` into your project.
2. Copy the `#mv-player` block from `index.html` (keep all `data-mv` attributes — they are the JS hooks).
3. Initialize:

```html
<link rel="stylesheet" href="css/player.css">

<div id="mv-player" class="mv-player" tabindex="0">
  <video class="mv-player__video" poster="poster.jpg" playsinline></video>
  <!-- paste the rest of the controls markup from index.html -->
</div>

<script src="js/player.js"></script>
<script>
  const player = new ModernVideoPlayer(document.querySelector('#mv-player'), {
    src: 'video-720.mp4',
    poster: 'poster.jpg',
    // optional:
    qualities: [
      { label: '1080p', src: 'video-1080.mp4' },
      { label: '720p',  src: 'video-720.mp4', default: true }
    ],
    tracks: [
      { label: 'English',  lang: 'en', src: 'captions-en.vtt', default: true },
      { label: 'Français', lang: 'fr', src: 'captions-fr.vtt' }
    ],
    chapters: [
      { time: 0,  title: 'Intro' },
      { time: 42, title: 'Chapter 1 — The chase' }
    ]
  });
</script>
```

Or use playlist:

```js
new ModernVideoPlayer(document.querySelector('#mv-player'), {
  playlist: [
    {
      title: 'Big Buck Bunny',
      subtitle: '10:34 • Blender',
      src: 'bbb.mp4',
      poster: 'bbb.jpg',
      duration: '10:34',
      tracks: [{ label: 'English', lang: 'en', src: 'en.vtt' }],
      chapters: [{ time: 0, title: 'Intro' }, { time: 32, title: 'Forest' }]
    },
    // ...
  ],
  autoplayNext: true     // auto-play next on ended
});
```

## Laravel Blade

See `examples/blade-player.blade.php`. Minimal:

```blade
<x-video-player :src="asset('videos/demo.mp4')" :tracks="[['label'=>'English','lang'=>'en','src'=>asset('captions/en.vtt')]]" />
```

The component uses `data-mv-auto-init` + `data-options` JSON so `player.js` auto-initializes on `DOMContentLoaded`.

## React

See `examples/react-player.jsx`:

```jsx
import { useEffect, useRef } from 'react';
import './player.css';
import './player.js'; // or dynamic import

export default function VideoPlayer({ src, tracks }) {
  const ref = useRef(null);
  useEffect(() => {
    const p = new window.ModernVideoPlayer(ref.current, { src, tracks });
    return () => p.destroy();
  }, [src]);
  return <div ref={ref} className="mv-player">{/* inner markup */}</div>;
}
```

## HTML structure (essential hooks)

```html
<div class="mv-player" tabindex="0">
  <video class="mv-player__video"></video>
  <button data-mv="center-play"></button>
  <div data-mv="tap-left"></div><div data-mv="tap-center"></div><div data-mv="tap-right"></div>
  <div data-mv="progress-wrap"><div data-mv="progress"><div data-mv="progress-buffered"></div><div data-mv="progress-filled"></div></div><div data-mv="tooltip"></div></div>
  <button data-mv="play"></button><button data-mv="prev"></button><button data-mv="next"></button>
  <button data-mv="mute"></button><input data-mv="volume" type="range" min="0" max="1" step="0.01">
  <span data-mv="time-current"></span><span data-mv="time-total"></span>
  <button data-mv="loop"></button><button data-mv="captions"></button><button data-mv="settings"></button><button data-mv="pip"></button><button data-mv="fullscreen"></button>
  <div data-mv="settings-menu"></div><div data-mv="captions-menu"></div>
</div>
<aside data-mv="playlist"><div data-mv="playlist-list"></div></aside>
```

All `data-mv` attributes are required hooks for `player.js` `querySelector`s (see file header).

## CSS customization

Override variables:

```css
.mv-player {
  --mv-accent: #3b82f6;      /* brand blue instead of turquoise */
  --mv-radius-lg: 12px;
}
```

## Accessibility notes

- Buttons are native `<button>` with `aria-label` and `aria-pressed`.
- Progress is `role="slider"` with `aria-valuenow`/`aria-valuetext`.
- Focus rings use `--mv-accent` outline, `Tab` order preserved.
- Captions use native `<track>`; toggle sets `textTracks[i].mode` to `showing`/`hidden`.

## Browser support

- Fullscreen: `requestFullscreen` / `webkitRequestFullscreen` (iOS Safari fallback via `webkitEnterFullscreen`)
- PiP: hidden if `document.pictureInPictureEnabled` is false
- Buffered range via `video.buffered`

## License

MIT — use freely in any project.

