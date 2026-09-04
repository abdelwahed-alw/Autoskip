{{-- resources/views/components/video-player.blade.php --}}
{{--
  Props:
    string  $src       — main video source (fallback if no qualities)
    string  $poster    — poster image URL
    array   $tracks    — [[label, lang, src, default], ...]
    array   $chapters  — [[time, title], ...]
    array   $qualities — [[label, src, default], ...]
    array   $playlist  — optional playlist items (see index.html demoPlaylist)
    bool    $autoplayNext
  Usage:
    <x-video-player :src="asset('videos/demo.mp4')" :poster="asset('images/poster.jpg')" :tracks="$tracks" />
    // With playlist:
    <x-video-player :playlist="$playlist" :autoplayNext="true" />
--}}

@props([
    'src' => null,
    'poster' => null,
    'tracks' => [],
    'chapters' => [],
    'qualities' => [],
    'playlist' => null,
    'autoplayNext' => true,
    'loop' => false,
])

@php
  // Build options for JS. Keep keys in English.
  $options = array_filter([
      'src' => $src,
      'poster' => $poster,
      'tracks' => $tracks ?: null,
      'chapters' => $chapters ?: null,
      'qualities' => $qualities ?: null,
      'playlist' => $playlist,
      'autoplayNext' => $autoplayNext,
      'loop' => $loop,
  ], fn($v) => !is_null($v));
@endphp

{{-- Styles — publish css/player.css to public/css/player.css --}}
<link rel="stylesheet" href="{{ asset('css/player.css') }}">

<div
    class="mv-player"
    tabindex="0"
    role="region"
    aria-label="Video player"
    data-mv-auto-init
    data-options="{{ json_encode($options, JSON_HEX_APOS | JSON_HEX_QUOT) }}"
>
    <video
        class="mv-player__video"
        @if($poster) poster="{{ $poster }}" @endif
        preload="metadata"
        playsinline
        crossorigin="anonymous"
    >
        {{-- Fallback tracks rendered server-side; JS will also inject via options --}}
        @foreach($tracks as $t)
            <track kind="subtitles"
                   label="{{ $t['label'] ?? $t['lang'] }}"
                   srclang="{{ $t['lang'] ?? $t['srclang'] ?? 'en' }}"
                   src="{{ $t['src'] }}"
                   @if(!empty($t['default'])) default @endif>
        @endforeach
        Your browser does not support the video tag.
    </video>

    <div class="mv-player__spinner" data-mv="spinner" aria-hidden="true">
        <div class="mv-spinner" role="status" aria-label="Loading"></div>
    </div>

    <button class="mv-center-play" data-mv="center-play" aria-label="Play (Space)">
        <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M8 5.14v14l11-7-11-7z"/></svg>
    </button>

    <div class="mv-tap-zones" aria-hidden="true">
        <div class="mv-tap-zone mv-tap-zone--left" data-mv="tap-left"></div>
        <div class="mv-tap-zone mv-tap-zone--center" data-mv="tap-center"></div>
        <div class="mv-tap-zone mv-tap-zone--right" data-mv="tap-right"></div>
    </div>

    <div class="mv-gesture-feedback mv-gesture-feedback--left" data-mv="feedback-left" aria-hidden="true">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><path d="M9 8l-5 4 5 4"/><path d="M20 12a8 8 0 11-8-8"/><text x="11.5" y="15.5" font-size="7" font-weight="700" fill="currentColor" stroke="none" text-anchor="middle">10</text></svg>
        <span>-10s</span>
    </div>
    <div class="mv-gesture-feedback mv-gesture-feedback--right" data-mv="feedback-right" aria-hidden="true">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><path d="M15 8l5 4-5 4"/><path d="M4 12a8 8 0 108-8"/><text x="12.5" y="15.5" font-size="7" font-weight="700" fill="currentColor" stroke="none" text-anchor="middle">10</text></svg>
        <span>+10s</span>
    </div>

    <div class="mv-captions-cue" data-mv="captions-cue" aria-live="polite"></div>

    <div class="mv-help" data-mv="help" aria-hidden="true" role="dialog" aria-label="Keyboard shortcuts">
        <div class="mv-help__title">
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><circle cx="12" cy="12" r="10"/><path d="M9.09 9a3 3 0 015.83 1c0 2-3 3-3 3"/><path d="M12 17h.01"/></svg>
            Keyboard shortcuts
            <button class="mv-menu__close" data-mv="help-close" aria-label="Close help">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M18 6L6 18"/><path d="M6 6l12 12"/></svg>
            </button>
        </div>
        <div class="mv-help__grid">
            <span class="mv-help__key">Space</span><span class="mv-help__desc">Play / Pause</span>
            <span class="mv-help__key">← / →</span><span class="mv-help__desc">Seek −5s / +5s</span>
            <span class="mv-help__key">↑ / ↓</span><span class="mv-help__desc">Volume +/−5%</span>
            <span class="mv-help__key">M</span><span class="mv-help__desc">Mute</span>
            <span class="mv-help__key">F</span><span class="mv-help__desc">Fullscreen</span>
            <span class="mv-help__key">C</span><span class="mv-help__desc">Captions</span>
        </div>
    </div>

    <div class="mv-controls" data-mv="controls">
        <div class="mv-progress-wrap" data-mv="progress-wrap" aria-label="Video progress">
            <div class="mv-progress" data-mv="progress">
                <div class="mv-progress__buffered" data-mv="progress-buffered"></div>
                <div class="mv-progress__filled" data-mv="progress-filled"></div>
            </div>
            <div class="mv-tooltip" data-mv="tooltip" role="tooltip">
                <span data-mv="tooltip-time">0:00</span>
                <span class="mv-tooltip__chapter" data-mv="tooltip-chapter"></span>
            </div>
        </div>

        <div class="mv-bar">
            <div class="mv-bar__left">
                <button class="mv-btn" data-mv="play" aria-label="Play (Space)"></button>
                <button class="mv-btn mv-btn--small" data-mv="prev" aria-label="Previous video">
                    <svg viewBox="0 0 24 24" fill="currentColor"><path d="M6 6h2v12H6zM9.5 12L18 6v12l-8.5-6z"/></svg>
                </button>
                <button class="mv-btn mv-btn--small" data-mv="next" aria-label="Next video">
                    <svg viewBox="0 0 24 24" fill="currentColor"><path d="M6 18l8.5-6L6 6v12zM16 6v12h2V6h-2z"/></svg>
                </button>
                <div class="mv-volume" data-mv="volume-group">
                    <button class="mv-btn mv-btn--small" data-mv="mute" aria-label="Mute (M)"></button>
                    <div class="mv-volume__slider-wrap">
                        <input class="mv-volume__slider" data-mv="volume" type="range" min="0" max="1" step="0.01" value="1" aria-label="Volume">
                    </div>
                </div>
                <div class="mv-time" aria-live="off">
                    <span data-mv="time-current">0:00</span>
                    <span class="mv-time__sep">/</span>
                    <span class="mv-time__total" data-mv="time-total">0:00</span>
                </div>
            </div>
            <div class="mv-bar__right">
                <button class="mv-btn mv-btn--small" data-mv="loop" aria-label="Loop" aria-pressed="false">
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M17 1l4 4-4 4"/><path d="M3 11V9a4 4 0 014-4h14"/><path d="M7 23l-4-4 4-4"/><path d="M21 13v2a4 4 0 01-4 4H3"/></svg>
                </button>
                <button class="mv-btn mv-btn--small" data-mv="captions" aria-label="Captions (C)">
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><rect x="3" y="6" width="18" height="12" rx="2"/><path d="M7 12a2 2 0 012-2h1"/><path d="M7 12a2 2 0 002 2h1"/><path d="M13 12a2 2 0 012-2h1"/><path d="M13 12a2 2 0 002 2h1"/></svg>
                </button>
                <button class="mv-btn mv-btn--small" data-mv="settings" aria-label="Settings">
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><circle cx="12" cy="12" r="3"/><path d="M12 1v2"/><path d="M12 21v2"/><path d="M4.22 4.22l1.42 1.42"/><path d="M18.36 18.36l1.42 1.42"/><path d="M1 12h2"/><path d="M21 12h2"/><path d="M4.22 19.78l1.42-1.42"/><path d="M18.36 5.64l1.42-1.42"/></svg>
                </button>
                <button class="mv-btn mv-btn--small" data-mv="pip" aria-label="Picture-in-Picture">
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><rect x="3" y="5" width="18" height="14" rx="2"/><rect x="11" y="11" width="8" height="6" rx="1" fill="currentColor" stroke="none"/></svg>
                </button>
                <button class="mv-btn mv-btn--small" data-mv="help-btn" aria-label="Help (?)" title="Keyboard shortcuts">
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><circle cx="12" cy="12" r="10"/><path d="M9.09 9a3 3 0 015.83 1c0 2-3 3-3 3"/><path d="M12 17h.01"/></svg>
                </button>
                <button class="mv-btn mv-btn--small" data-mv="fullscreen" aria-label="Fullscreen (F)">
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M8 3H5a2 2 0 00-2 2v3"/><path d="M16 3h3a2 2 0 012 2v3"/><path d="M8 21H5a2 2 0 01-2-2v-3"/><path d="M16 21h3a2 2 0 002-2v-3"/></svg>
                </button>
            </div>
        </div>

        <div class="mv-menu" data-mv="settings-menu" aria-hidden="true" role="menu"></div>
        <div class="mv-menu mv-menu--captions" data-mv="captions-menu" aria-hidden="true" role="menu"></div>
    </div>
</div>

@if($playlist)
<aside class="mv-playlist" data-mv="playlist" aria-label="Playlist" style="margin-top:16px;">
    <div class="mv-playlist__header">
        <div>
            <div class="mv-playlist__title">Up next</div>
            <div class="mv-playlist__meta" data-mv="playlist-meta">{{ count($playlist) }} videos</div>
        </div>
        <label class="mv-playlist__autoplay">
            <span>Autoplay</span>
            <input type="checkbox" data-mv="autoplay-toggle" checked>
        </label>
    </div>
    <div class="mv-playlist__list" data-mv="playlist-list" role="list"></div>
</aside>
@endif

{{-- Script — publish js/player.js to public/js/player.js --}}
<script src="{{ asset('js/player.js') }}"></script>
