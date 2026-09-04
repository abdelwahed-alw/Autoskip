/**
 * React wrapper for ModernVideoPlayer (vanilla class in js/player.js)
 * Usage:
 *   import VideoPlayer from './react-player';
 *   <VideoPlayer src="video.mp4" poster="poster.jpg" tracks={[...]} playlist={[...]} />
 */

import { useEffect, useRef } from 'react';
// Import styles — adjust path to where you copied css/player.css
import '../css/player.css';
// Ensure player.js is loaded and registers window.ModernVideoPlayer
import '../js/player.js';

export default function VideoPlayer({
  src,
  poster,
  tracks,
  qualities,
  chapters,
  playlist,
  autoplayNext = true,
  loop = false,
  startVolume = 1,
  className = '',
  style,
}) {
  const containerRef = useRef(null);
  const playerRef = useRef(null);

  useEffect(() => {
    if (!containerRef.current || !window.ModernVideoPlayer) return;

    // Destroy previous instance on re-render
    if (playerRef.current) {
      playerRef.current.destroy();
      playerRef.current = null;
    }

    const options = {
      src,
      poster,
      tracks,
      qualities,
      chapters,
      playlist,
      autoplayNext,
      loop,
      startVolume,
    };

    playerRef.current = new window.ModernVideoPlayer(containerRef.current, options);

    // Expose for debugging
    // window.player = playerRef.current;

    return () => {
      playerRef.current?.destroy();
      playerRef.current = null;
    };
    // Re-create only when src/playlist identity changes; for fine-grained updates use imperative API
  }, [src, poster, JSON.stringify(tracks), JSON.stringify(playlist)]);

  return (
    <div
      ref={containerRef}
      className={`mv-player ${className}`}
      tabIndex={0}
      role="region"
      aria-label="Video player"
      style={style}
    >
      {/* Video element — src/poster can also be set via JS options */}
      <video
        className="mv-player__video"
        poster={poster}
        preload="metadata"
        playsInline
        crossOrigin="anonymous"
      />

      {/* Spinner */}
      <div className="mv-player__spinner" data-mv="spinner" aria-hidden="true">
        <div className="mv-spinner" />
      </div>

      {/* Center play */}
      <button className="mv-center-play" data-mv="center-play" aria-label="Play">
        <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
          <path d="M8 5.14v14l11-7-11-7z" />
        </svg>
      </button>

      {/* Tap zones */}
      <div className="mv-tap-zones" aria-hidden="true">
        <div className="mv-tap-zone mv-tap-zone--left" data-mv="tap-left" />
        <div className="mv-tap-zone mv-tap-zone--center" data-mv="tap-center" />
        <div className="mv-tap-zone mv-tap-zone--right" data-mv="tap-right" />
      </div>

      {/* Gesture feedback */}
      <div className="mv-gesture-feedback mv-gesture-feedback--left" data-mv="feedback-left" aria-hidden="true">
        <span>-10s</span>
      </div>
      <div className="mv-gesture-feedback mv-gesture-feedback--right" data-mv="feedback-right" aria-hidden="true">
        <span>+10s</span>
      </div>

      <div className="mv-captions-cue" data-mv="captions-cue" aria-live="polite" />

      {/* Help overlay */}
      <div className="mv-help" data-mv="help" aria-hidden="true" role="dialog" aria-label="Keyboard shortcuts">
        <div className="mv-help__title">
          Keyboard shortcuts
          <button className="mv-menu__close" data-mv="help-close" aria-label="Close">
            ×
          </button>
        </div>
        <div className="mv-help__grid">
          <span className="mv-help__key">Space</span><span className="mv-help__desc">Play / Pause</span>
          <span className="mv-help__key">M</span><span className="mv-help__desc">Mute</span>
          <span className="mv-help__key">F</span><span className="mv-help__desc">Fullscreen</span>
          <span className="mv-help__key">C</span><span className="mv-help__desc">Captions</span>
        </div>
      </div>

      {/* Controls */}
      <div className="mv-controls" data-mv="controls">
        <div className="mv-progress-wrap" data-mv="progress-wrap">
          <div className="mv-progress" data-mv="progress">
            <div className="mv-progress__buffered" data-mv="progress-buffered" />
            <div className="mv-progress__filled" data-mv="progress-filled" />
          </div>
          <div className="mv-tooltip" data-mv="tooltip">
            <span data-mv="tooltip-time">0:00</span>
            <span className="mv-tooltip__chapter" data-mv="tooltip-chapter" />
          </div>
        </div>

        <div className="mv-bar">
          <div className="mv-bar__left">
            <button className="mv-btn" data-mv="play" aria-label="Play" />
            <button className="mv-btn mv-btn--small" data-mv="prev" aria-label="Previous" />
            <button className="mv-btn mv-btn--small" data-mv="next" aria-label="Next" />
            <div className="mv-volume" data-mv="volume-group">
              <button className="mv-btn mv-btn--small" data-mv="mute" aria-label="Mute" />
              <div className="mv-volume__slider-wrap">
                <input className="mv-volume__slider" data-mv="volume" type="range" min="0" max="1" step="0.01" defaultValue="1" aria-label="Volume" />
              </div>
            </div>
            <div className="mv-time">
              <span data-mv="time-current">0:00</span>
              <span className="mv-time__sep">/</span>
              <span data-mv="time-total">0:00</span>
            </div>
          </div>
          <div className="mv-bar__right">
            <button className="mv-btn mv-btn--small" data-mv="loop" aria-label="Loop" />
            <button className="mv-btn mv-btn--small" data-mv="captions" aria-label="Captions" />
            <button className="mv-btn mv-btn--small" data-mv="settings" aria-label="Settings" />
            <button className="mv-btn mv-btn--small" data-mv="pip" aria-label="PiP" />
            <button className="mv-btn mv-btn--small" data-mv="help-btn" aria-label="Help" />
            <button className="mv-btn mv-btn--small" data-mv="fullscreen" aria-label="Fullscreen" />
          </div>
        </div>

        <div className="mv-menu" data-mv="settings-menu" aria-hidden="true" />
        <div className="mv-menu mv-menu--captions" data-mv="captions-menu" aria-hidden="true" />
      </div>
    </div>
  );
}
