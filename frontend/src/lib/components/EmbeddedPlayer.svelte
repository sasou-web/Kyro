<script lang="ts">
  /**
   * Player embarqué intégré à la fenêtre Kyro.
   *
   * libmpv rend dans un HWND Win32 placé par-dessus la WebView.
   * Conséquence : aucun élément Svelte ne peut s'afficher au-dessus du
   * rectangle vidéo. Toute la chrome (top bar, bottom bar, menus,
   * boutons PiP, etc.) doit donc être **à côté** de la zone vidéo,
   * jamais par-dessus.
   *
   * Implémentation :
   * - Layout flex column : [topbar?][video-zone (flex 1)][controls-bar]
   * - En mode `fullscreen`, top + controls peuvent disparaître en
   *   auto-hide ; quand ils disparaissent, la zone vidéo s'agrandit.
   * - Les menus déroulants (audio / sub / vitesse) repoussent les
   *   contrôles en agrandissant la barre du bas. Tant que le menu est
   *   ouvert, la zone vidéo est compressée pour libérer la place.
   * - Le PiP a sa propre disposition : vidéo en haut, mini-barre en
   *   bas (visible en permanence) avec play/pause, expand, fermer.
   *
   * Click / double-click sur la vidéo : le HWND mpv intercepte tous
   * les clics côté DOM. On reçoit donc les événements
   * `embedded-player-clicked` / `-double-clicked` émis par la wnd proc
   * Win32 (cf. `infra::embedded_player::host_window`).
   */
  import { onDestroy, onMount, tick } from 'svelte';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { getCurrentWindow } from '@tauri-apps/api/window';

  import {
    embeddedPlayerSeek,
    embeddedPlayerSetAudioTrack,
    embeddedPlayerSetGeometry,
    embeddedPlayerSetMute,
    embeddedPlayerSetSpeed,
    embeddedPlayerSetSubtitleTrack,
    embeddedPlayerSetVisible,
    embeddedPlayerSetVolume,
    embeddedPlayerTogglePause,
    embeddedPlayerTrackList
  } from '$lib/api/commands';
  import {
    EVENT_EMBEDDED_PLAYER_CLICKED,
    EVENT_EMBEDDED_PLAYER_CLOSED,
    EVENT_EMBEDDED_PLAYER_COMPLETED,
    EVENT_EMBEDDED_PLAYER_DOUBLE_CLICKED,
    EVENT_EMBEDDED_PLAYER_PAUSE_CHANGED,
    EVENT_EMBEDDED_PLAYER_PROGRESS,
    type EmbeddedPlayerPause,
    type EmbeddedPlayerProgress,
    type MpvTrack
  } from '$lib/api/types';
  import {
    closeSession,
    player,
    setMode,
    type PlayerState
  } from '$lib/stores/player';

  // --- Refs DOM ---------------------------------------------------------

  let videoZone: HTMLDivElement;
  let root: HTMLDivElement;

  // --- État local -------------------------------------------------------

  let mode: PlayerState['mode'] = 'closed';
  let position = 0;
  let duration = 0;
  let paused = false;
  let title: string | null = null;

  let scrubbing = false;
  let scrubPosition = 0;

  let volume = 100;
  let muted = false;
  let speed = 1;
  const SPEEDS = [0.5, 0.75, 1, 1.25, 1.5, 1.75, 2];

  let tracks: MpvTrack[] = [];
  let selectedAudioId: number | null = null;
  let selectedSubId: number | null = null;
  let trackRefreshTimer: ReturnType<typeof setTimeout> | null = null;

  let systemFullscreen = false;

  type MenuKind = 'audio' | 'sub' | 'speed' | null;
  let openMenu: MenuKind = null;

  let controlsVisible = true;
  let controlsHideTimer: ReturnType<typeof setTimeout> | null = null;

  let pipDragging = false;
  let pipOffsetX = 0;
  let pipOffsetY = 0;
  let pipX = 0;
  let pipY = 0;

  // --- Bindings store ---------------------------------------------------

  const unlisteners: UnlistenFn[] = [];
  let resizeObserver: ResizeObserver | null = null;
  let lastGeometry = { x: -1, y: -1, w: -1, h: -1 };
  let geometryRaf = 0;

  $: {
    mode = $player.mode;
    title = $player.title;
    if (!scrubbing) position = $player.position;
    duration = $player.duration;
    paused = $player.paused;
  }

  $: if (mode !== 'closed' && videoZone) {
    void scheduleGeometryUpdate();
  }
  $: if (mode === 'closed') {
    lastGeometry = { x: -1, y: -1, w: -1, h: -1 };
    if (controlsHideTimer !== null) {
      clearTimeout(controlsHideTimer);
      controlsHideTimer = null;
    }
    if (trackRefreshTimer !== null) {
      clearTimeout(trackRefreshTimer);
      trackRefreshTimer = null;
    }
    tracks = [];
    selectedAudioId = null;
    selectedSubId = null;
    openMenu = null;
    void exitSystemFullscreen();
  }
  $: if (controlsVisible !== undefined && mode !== 'closed') {
    void scheduleGeometryUpdate();
  }
  // Re-pose la geometry à chaque changement d'ouverture de menu
  // (la barre de contrôles change de hauteur).
  $: if (openMenu !== undefined && mode !== 'closed') {
    void scheduleGeometryUpdate();
  }

  onMount(async () => {
    unlisteners.push(
      await listen<EmbeddedPlayerProgress>(
        EVENT_EMBEDDED_PLAYER_PROGRESS,
        (e) => {
          player.update((s) => ({
            ...s,
            position: e.payload.position,
            duration: e.payload.duration > 0 ? e.payload.duration : s.duration
          }));
        }
      )
    );

    unlisteners.push(
      await listen<EmbeddedPlayerPause>(
        EVENT_EMBEDDED_PLAYER_PAUSE_CHANGED,
        (e) => {
          player.update((s) => ({ ...s, paused: e.payload.paused }));
        }
      )
    );

    unlisteners.push(
      await listen<unknown>(EVENT_EMBEDDED_PLAYER_COMPLETED, () => {
        setTimeout(() => void closeSession(), 800);
      })
    );

    unlisteners.push(
      await listen<EmbeddedPlayerProgress>(EVENT_EMBEDDED_PLAYER_CLOSED, () => {
        void closeSession();
      })
    );

    // Click sur la vidéo (intercepté par la wnd proc Win32) →
    // toggle pause. Double-click → bascule plein écran.
    unlisteners.push(
      await listen<unknown>(EVENT_EMBEDDED_PLAYER_CLICKED, () => {
        if (mode === 'closed') return;
        void togglePause();
      })
    );
    unlisteners.push(
      await listen<unknown>(EVENT_EMBEDDED_PLAYER_DOUBLE_CLICKED, () => {
        if (mode === 'closed') return;
        if (mode === 'pip') {
          expand();
        } else {
          void toggleSystemFullscreen();
        }
      })
    );

    if (typeof ResizeObserver !== 'undefined') {
      resizeObserver = new ResizeObserver(() => scheduleGeometryUpdate());
      if (videoZone) resizeObserver.observe(videoZone);
    }

    window.addEventListener('scroll', scheduleGeometryUpdate, true);
    window.addEventListener('resize', scheduleGeometryUpdate);
  });

  onDestroy(() => {
    for (const u of unlisteners) {
      try {
        u();
      } catch {
        /* ignore */
      }
    }
    if (resizeObserver) resizeObserver.disconnect();
    if (controlsHideTimer !== null) clearTimeout(controlsHideTimer);
    if (trackRefreshTimer !== null) clearTimeout(trackRefreshTimer);
    window.removeEventListener('scroll', scheduleGeometryUpdate, true);
    window.removeEventListener('resize', scheduleGeometryUpdate);
  });

  // --- Tracklist --------------------------------------------------------

  $: if (mode === 'fullscreen' || mode === 'pip') {
    if (tracks.length === 0 && !trackRefreshTimer) {
      trackRefreshTimer = setTimeout(() => {
        void refreshTracks();
        trackRefreshTimer = setTimeout(() => {
          if (tracks.length === 0) void refreshTracks();
          trackRefreshTimer = null;
        }, 1500);
      }, 600);
    }
  }

  async function refreshTracks() {
    try {
      const raw = await embeddedPlayerTrackList();
      const parsed = JSON.parse(raw) as MpvTrack[];
      tracks = parsed;
      const sa = parsed.find((t) => t.type === 'audio' && t.selected);
      const ss = parsed.find((t) => t.type === 'sub' && t.selected);
      selectedAudioId = sa ? sa.id : null;
      selectedSubId = ss ? ss.id : null;
    } catch (err) {
      console.warn('embedded-player: track-list', err);
    }
  }

  // --- Geometry ---------------------------------------------------------

  function scheduleGeometryUpdate(): void {
    if (geometryRaf !== 0) return;
    geometryRaf = requestAnimationFrame(() => {
      geometryRaf = 0;
      void applyGeometry();
    });
  }

  async function applyGeometry(): Promise<void> {
    if (!videoZone || mode === 'closed') return;
    await tick();
    const rect = videoZone.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    const x = Math.round(rect.left * dpr);
    const y = Math.round(rect.top * dpr);
    const w = Math.max(1, Math.round(rect.width * dpr));
    const h = Math.max(1, Math.round(rect.height * dpr));

    if (
      x === lastGeometry.x &&
      y === lastGeometry.y &&
      w === lastGeometry.w &&
      h === lastGeometry.h
    ) {
      return;
    }
    lastGeometry = { x, y, w, h };

    try {
      await embeddedPlayerSetGeometry(x, y, w, h);
      await embeddedPlayerSetVisible(true);
    } catch (err) {
      console.warn('embedded-player: set_geometry', err);
    }
  }

  // --- Auto-hide --------------------------------------------------------

  function armControlsTimer() {
    controlsVisible = true;
    if (controlsHideTimer !== null) clearTimeout(controlsHideTimer);
    if (mode !== 'fullscreen') return;
    if (openMenu !== null) return;
    controlsHideTimer = setTimeout(() => {
      if (!scrubbing && !paused && openMenu === null) {
        controlsVisible = false;
      }
    }, 3000);
  }

  function onPointerMove() {
    armControlsTimer();
  }

  // --- Helpers UI -------------------------------------------------------

  function formatTime(seconds: number): string {
    if (!Number.isFinite(seconds) || seconds < 0) return '0:00';
    const total = Math.floor(seconds);
    const h = Math.floor(total / 3600);
    const m = Math.floor((total % 3600) / 60);
    const s = total % 60;
    if (h > 0) {
      return `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
    }
    return `${m}:${String(s).padStart(2, '0')}`;
  }

  async function togglePause() {
    try {
      await embeddedPlayerTogglePause();
    } catch (err) {
      console.warn('embedded-player: toggle pause', err);
    }
    armControlsTimer();
  }

  function onScrubInput(e: Event) {
    const t = e.target as HTMLInputElement;
    scrubbing = true;
    scrubPosition = Number(t.value);
    armControlsTimer();
  }

  async function onScrubCommit() {
    scrubbing = false;
    try {
      await embeddedPlayerSeek(scrubPosition);
      position = scrubPosition;
    } catch (err) {
      console.warn('embedded-player: seek', err);
    }
    armControlsTimer();
  }

  async function close() {
    await closeSession();
  }

  function expand() {
    setMode('fullscreen');
    armControlsTimer();
  }
  function minimize() {
    setMode('pip');
    openMenu = null;
  }

  async function toggleSystemFullscreen() {
    try {
      const w = getCurrentWindow();
      const newState = !systemFullscreen;
      await w.setFullscreen(newState);
      systemFullscreen = newState;
    } catch (err) {
      console.warn('embedded-player: setFullscreen', err);
    }
  }
  async function exitSystemFullscreen() {
    if (!systemFullscreen) return;
    try {
      const w = getCurrentWindow();
      await w.setFullscreen(false);
      systemFullscreen = false;
    } catch {
      /* ignore */
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if (mode === 'closed') return;
    armControlsTimer();
    if (e.key === ' ' || e.key === 'k') {
      e.preventDefault();
      void togglePause();
    } else if (e.key === 'ArrowLeft') {
      e.preventDefault();
      void embeddedPlayerSeek(Math.max(0, position - 5));
    } else if (e.key === 'ArrowRight') {
      e.preventDefault();
      void embeddedPlayerSeek(
        Math.min(duration > 0 ? duration : position + 10, position + 5)
      );
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      void changeVolume(volume + 5);
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      void changeVolume(volume - 5);
    } else if (e.key === 'm') {
      e.preventDefault();
      void toggleMute();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      if (systemFullscreen) {
        void exitSystemFullscreen();
      } else if (openMenu !== null) {
        openMenu = null;
      } else if (mode === 'pip') {
        void close();
      } else {
        minimize();
      }
    } else if (e.key === 'f') {
      e.preventDefault();
      if (mode === 'pip') expand();
      else void toggleSystemFullscreen();
    }
  }

  async function changeVolume(v: number) {
    volume = Math.max(0, Math.min(150, Math.round(v)));
    try {
      await embeddedPlayerSetVolume(volume);
      if (volume > 0 && muted) {
        muted = false;
        await embeddedPlayerSetMute(false);
      }
    } catch (err) {
      console.warn('embedded-player: setVolume', err);
    }
  }

  async function onVolumeInput(e: Event) {
    const t = e.target as HTMLInputElement;
    await changeVolume(Number(t.value));
  }

  async function toggleMute() {
    muted = !muted;
    try {
      await embeddedPlayerSetMute(muted);
    } catch (err) {
      console.warn('embedded-player: setMute', err);
    }
  }

  async function changeSpeed(s: number) {
    speed = s;
    try {
      await embeddedPlayerSetSpeed(s);
    } catch (err) {
      console.warn('embedded-player: setSpeed', err);
    }
    openMenu = null;
  }

  async function pickAudio(id: number | null) {
    selectedAudioId = id;
    try {
      await embeddedPlayerSetAudioTrack(id);
    } catch (err) {
      console.warn('embedded-player: setAudioTrack', err);
    }
    openMenu = null;
  }

  async function pickSub(id: number | null) {
    selectedSubId = id;
    try {
      await embeddedPlayerSetSubtitleTrack(id);
    } catch (err) {
      console.warn('embedded-player: setSubtitleTrack', err);
    }
    openMenu = null;
  }

  function toggleMenu(kind: MenuKind) {
    openMenu = openMenu === kind ? null : kind;
    if (openMenu !== null && controlsHideTimer !== null) {
      clearTimeout(controlsHideTimer);
      controlsHideTimer = null;
    }
  }

  // --- PiP drag --------------------------------------------------------

  function onPipMouseDown(e: MouseEvent) {
    if (mode !== 'pip') return;
    if (!root) return;
    const rect = root.getBoundingClientRect();
    pipDragging = true;
    pipOffsetX = e.clientX - rect.left;
    pipOffsetY = e.clientY - rect.top;
    e.preventDefault();
  }

  function onWindowMouseMove(e: MouseEvent) {
    if (!pipDragging) return;
    pipX = Math.max(0, e.clientX - pipOffsetX);
    pipY = Math.max(0, e.clientY - pipOffsetY);
  }

  function onWindowMouseUp() {
    pipDragging = false;
  }

  // --- Filtres ----------------------------------------------------------

  $: audioTracks = tracks.filter((t) => t.type === 'audio');
  $: subTracks = tracks.filter((t) => t.type === 'sub');
  $: trackLabel = (t: MpvTrack) => {
    const parts: string[] = [];
    if (t.lang) parts.push(t.lang.toUpperCase());
    if (t.title) parts.push(t.title);
    if (parts.length === 0 && t.codec) parts.push(t.codec);
    if (parts.length === 0) parts.push(`#${t.id}`);
    if (t['demux-channel-count']) parts.push(`${t['demux-channel-count']}.0`);
    return parts.join(' · ');
  };
</script>

<svelte:window
  on:keydown={onKeydown}
  on:mousemove={onWindowMouseMove}
  on:mouseup={onWindowMouseUp}
/>

{#if mode !== 'closed'}
  <div
    class="player-overlay"
    class:fullscreen={mode === 'fullscreen'}
    class:pip={mode === 'pip'}
    class:controls-hidden={!controlsVisible && mode === 'fullscreen'}
    style:left={mode === 'pip' && pipX > 0 ? `${pipX}px` : ''}
    style:top={mode === 'pip' && pipY > 0 ? `${pipY}px` : ''}
    style:right={mode === 'pip' && pipX === 0 ? '24px' : ''}
    style:bottom={mode === 'pip' && pipY === 0 ? '24px' : ''}
    bind:this={root}
    on:mousemove={onPointerMove}
  >
    {#if mode === 'fullscreen'}
      <header class="topbar" class:hidden={!controlsVisible}>
        <div class="topbar-inner">
          <button
            type="button"
            class="icon-btn"
            on:click={close}
            aria-label="Fermer"
            title="Fermer"
          >
            <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
              <path d="M19 12H5M12 19l-7-7 7-7" />
            </svg>
          </button>
          {#if title}
            <h2 class="title">{title}</h2>
          {/if}
          <div class="topbar-actions">
            <button
              type="button"
              class="icon-btn"
              on:click={minimize}
              aria-label="Réduire en PiP"
              title="Mode PiP (Échap)"
            >
              <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <rect x="3" y="3" width="18" height="14" rx="2" />
                <rect x="13" y="11" width="8" height="6" rx="1" fill="currentColor" />
              </svg>
            </button>
            <button
              type="button"
              class="icon-btn"
              on:click={toggleSystemFullscreen}
              aria-label={systemFullscreen ? 'Quitter le plein écran' : 'Plein écran'}
              title="Plein écran (F)"
            >
              {#if systemFullscreen}
                <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M9 9V3M9 9H3M15 9V3M15 9h6M9 15v6M9 15H3M15 15v6M15 15h6" />
                </svg>
              {:else}
                <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M3 9V3h6M21 9V3h-6M3 15v6h6M21 15v6h-6" />
                </svg>
              {/if}
            </button>
          </div>
        </div>
      </header>
    {/if}

    <div class="video-zone" bind:this={videoZone}></div>

    {#if mode === 'fullscreen'}
      <footer class="controls-bar" class:hidden={!controlsVisible}>
        <!-- Le menu sort VERS LE HAUT à l'intérieur de la barre,
             donc en dehors du HWND vidéo : il est visible. -->
        {#if openMenu !== null}
          <div class="menu" role="menu">
            {#if openMenu === 'speed'}
              <div class="menu-title">Vitesse</div>
              {#each SPEEDS as s}
                <button
                  type="button"
                  class="menu-item"
                  class:selected={Math.abs(speed - s) < 0.01}
                  on:click={() => changeSpeed(s)}
                  role="menuitemradio"
                  aria-checked={Math.abs(speed - s) < 0.01}
                >
                  {s}× {s === 1 ? '(normale)' : ''}
                </button>
              {/each}
            {:else if openMenu === 'audio'}
              <div class="menu-title">Piste audio</div>
              {#each audioTracks as t}
                <button
                  type="button"
                  class="menu-item"
                  class:selected={selectedAudioId === t.id}
                  on:click={() => pickAudio(t.id)}
                  role="menuitemradio"
                  aria-checked={selectedAudioId === t.id}
                >
                  {trackLabel(t)}
                </button>
              {/each}
              {#if audioTracks.length === 0}
                <div class="menu-empty">Aucune piste détectée</div>
              {/if}
            {:else if openMenu === 'sub'}
              <div class="menu-title">Sous-titres</div>
              <button
                type="button"
                class="menu-item"
                class:selected={selectedSubId === null}
                on:click={() => pickSub(null)}
                role="menuitemradio"
                aria-checked={selectedSubId === null}
              >
                Désactivés
              </button>
              {#each subTracks as t}
                <button
                  type="button"
                  class="menu-item"
                  class:selected={selectedSubId === t.id}
                  on:click={() => pickSub(t.id)}
                  role="menuitemradio"
                  aria-checked={selectedSubId === t.id}
                >
                  {trackLabel(t)}
                </button>
              {/each}
            {/if}
          </div>
        {/if}

        <input
          type="range"
          class="timeline"
          min="0"
          max={duration > 0 ? duration : 1}
          step="0.1"
          value={scrubbing ? scrubPosition : position}
          on:input={onScrubInput}
          on:change={onScrubCommit}
          aria-label="Position de lecture"
        />

        <div class="row">
          <button
            type="button"
            class="play-btn"
            on:click={togglePause}
            aria-label={paused ? 'Lecture' : 'Pause'}
          >
            {#if paused}
              <svg viewBox="0 0 24 24" width="22" height="22" fill="currentColor">
                <path d="M8 5v14l11-7z" />
              </svg>
            {:else}
              <svg viewBox="0 0 24 24" width="22" height="22" fill="currentColor">
                <path d="M6 4h4v16H6zM14 4h4v16h-4z" />
              </svg>
            {/if}
          </button>

          <span class="time" aria-live="polite">
            {formatTime(scrubbing ? scrubPosition : position)}
            <span class="time-sep">/</span>
            {formatTime(duration)}
          </span>

          <div class="volume-group">
            <button
              type="button"
              class="icon-btn"
              on:click={toggleMute}
              aria-label={muted ? 'Activer le son' : 'Couper le son'}
              title={muted ? 'Activer le son (M)' : 'Couper le son (M)'}
            >
              {#if muted || volume === 0}
                <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M11 5L6 9H2v6h4l5 4V5zM23 9l-6 6M17 9l6 6" />
                </svg>
              {:else if volume < 50}
                <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M11 5L6 9H2v6h4l5 4V5zM15.54 8.46a5 5 0 0 1 0 7.07" />
                </svg>
              {:else}
                <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M11 5L6 9H2v6h4l5 4V5zM19.07 4.93a10 10 0 0 1 0 14.14M15.54 8.46a5 5 0 0 1 0 7.07" />
                </svg>
              {/if}
            </button>
            <input
              type="range"
              class="volume-slider"
              min="0"
              max="100"
              step="1"
              value={muted ? 0 : volume}
              on:input={onVolumeInput}
              aria-label="Volume"
            />
          </div>

          <span class="spacer"></span>

          <button
            type="button"
            class="icon-btn text-btn"
            on:click={() => toggleMenu('speed')}
            class:active={openMenu === 'speed'}
            aria-label="Vitesse de lecture"
            title="Vitesse"
          >
            {speed}×
          </button>

          <button
            type="button"
            class="icon-btn"
            on:click={() => toggleMenu('audio')}
            class:active={openMenu === 'audio'}
            aria-label="Piste audio"
            title="Piste audio"
          >
            <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M3 18v-6a9 9 0 0 1 18 0v6" />
              <path d="M21 19a2 2 0 0 1-2 2h-1v-6h3zM3 19a2 2 0 0 0 2 2h1v-6H3z" />
            </svg>
          </button>

          <button
            type="button"
            class="icon-btn"
            on:click={() => toggleMenu('sub')}
            class:active={openMenu === 'sub'}
            aria-label="Sous-titres"
            title="Sous-titres"
          >
            <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <rect x="3" y="6" width="18" height="12" rx="2" />
              <path d="M7 14h2M11 14h6M7 11h10" />
            </svg>
          </button>
        </div>
      </footer>
    {/if}

    {#if mode === 'pip'}
      <!-- Mini-barre PiP fixée en bas, TOUJOURS visible (pas en
           overlay sur la vidéo, mais en dessous, dans le flux). -->
      <footer class="pip-bar">
        <button
          type="button"
          class="pip-drag"
          on:mousedown={onPipMouseDown}
          aria-label="Déplacer le mini-lecteur"
          title="Cliquer-glisser pour déplacer"
        >
          <svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor">
            <circle cx="9" cy="6" r="1.5" />
            <circle cx="15" cy="6" r="1.5" />
            <circle cx="9" cy="12" r="1.5" />
            <circle cx="15" cy="12" r="1.5" />
            <circle cx="9" cy="18" r="1.5" />
            <circle cx="15" cy="18" r="1.5" />
          </svg>
        </button>

        <button
          type="button"
          class="icon-btn pip-btn"
          on:click={togglePause}
          aria-label={paused ? 'Lecture' : 'Pause'}
        >
          {#if paused}
            <svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor">
              <path d="M8 5v14l11-7z" />
            </svg>
          {:else}
            <svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor">
              <path d="M6 4h4v16H6zM14 4h4v16h-4z" />
            </svg>
          {/if}
        </button>

        <span class="pip-time">{formatTime(position)}</span>
        <div class="pip-progress" aria-hidden="true">
          <div
            class="pip-progress-fill"
            style:width="{duration > 0 ? (position / duration) * 100 : 0}%"
          ></div>
        </div>

        <button
          type="button"
          class="icon-btn pip-btn"
          on:click={expand}
          aria-label="Sortir du PiP"
          title="Agrandir (F)"
        >
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M3 9V3h6M21 9V3h-6M3 15v6h6M21 15v6h-6" />
          </svg>
        </button>

        <button
          type="button"
          class="icon-btn pip-btn"
          on:click={close}
          aria-label="Fermer"
        >
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
            <path d="M18 6L6 18M6 6l12 12" />
          </svg>
        </button>
      </footer>
    {/if}
  </div>
{/if}

<style>
  /* Container ----------------------------------------------------------- */

  .player-overlay {
    position: fixed;
    z-index: 1000;
    background: #000;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .player-overlay.fullscreen {
    inset: 0;
    border-radius: 0;
  }

  .player-overlay.pip {
    width: 380px;
    height: 250px; /* 220 vidéo + 30 mini-barre */
    border-radius: 12px;
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.5),
      0 0 0 1px rgba(255, 255, 255, 0.08);
  }

  .video-zone {
    flex: 1 1 auto;
    min-height: 0;
    background: #000;
  }

  /* Top bar ------------------------------------------------------------- */

  .topbar {
    flex: 0 0 auto;
    display: flex;
    background: rgba(15, 15, 20, 0.96);
    border-bottom: 1px solid rgba(255, 255, 255, 0.06);
    transition: opacity 220ms ease, max-height 220ms ease;
    max-height: 56px;
    overflow: hidden;
  }
  .topbar.hidden {
    opacity: 0;
    max-height: 0;
    pointer-events: none;
    border-bottom: none;
  }
  .topbar-inner {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 10px 16px;
  }
  .title {
    flex: 1 1 auto;
    margin: 0;
    font-size: 1rem;
    font-weight: 500;
    color: rgba(255, 255, 255, 0.95);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .topbar-actions {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  /* Boutons ------------------------------------------------------------- */

  .icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
    padding: 0;
    background: transparent;
    border: none;
    border-radius: 8px;
    color: rgba(255, 255, 255, 0.85);
    cursor: pointer;
    transition: background-color 140ms ease, color 140ms ease;
    font-size: 0.9rem;
  }
  .icon-btn:hover {
    background: rgba(255, 255, 255, 0.12);
    color: #fff;
  }
  .icon-btn:active,
  .icon-btn.active {
    background: rgba(255, 255, 255, 0.2);
  }
  .icon-btn.text-btn {
    width: auto;
    min-width: 44px;
    padding: 0 10px;
    font-variant-numeric: tabular-nums;
    font-weight: 500;
  }

  /* Bottom bar (fullscreen) -------------------------------------------- */

  .controls-bar {
    flex: 0 0 auto;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px 24px 14px;
    background: rgba(15, 15, 20, 0.96);
    border-top: 1px solid rgba(255, 255, 255, 0.06);
    transition: opacity 220ms ease, max-height 220ms ease;
    overflow: visible;
  }
  .controls-bar.hidden {
    opacity: 0;
    max-height: 0;
    padding: 0 24px;
    pointer-events: none;
    border-top: none;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  /* Menus dans le flux : agrandissent la barre vers le haut ------------ */

  .menu {
    background: rgba(20, 20, 26, 0.98);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 10px;
    padding: 6px;
    max-height: 320px;
    overflow-y: auto;
    align-self: flex-end;
    width: 280px;
  }
  .menu-title {
    padding: 6px 10px;
    font-size: 0.78rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: rgba(255, 255, 255, 0.55);
  }
  .menu-item {
    display: flex;
    align-items: center;
    width: 100%;
    padding: 8px 10px;
    background: transparent;
    border: none;
    border-radius: 6px;
    color: rgba(255, 255, 255, 0.9);
    cursor: pointer;
    text-align: left;
    font-size: 0.9rem;
    transition: background-color 100ms ease;
  }
  .menu-item:hover {
    background: rgba(255, 255, 255, 0.08);
  }
  .menu-item.selected {
    background: rgba(79, 142, 247, 0.22);
    color: #fff;
  }
  .menu-item.selected::before {
    content: '✓';
    margin-right: 8px;
    color: var(--accent, #4f8ef7);
    font-weight: 700;
  }
  .menu-item:not(.selected)::before {
    content: '';
    margin-right: 8px;
    width: 14px;
    display: inline-block;
  }
  .menu-empty {
    padding: 12px;
    color: rgba(255, 255, 255, 0.5);
    font-size: 0.88rem;
    text-align: center;
  }

  /* Timeline / play ---------------------------------------------------- */

  .timeline {
    -webkit-appearance: none;
    appearance: none;
    width: 100%;
    height: 4px;
    background: rgba(255, 255, 255, 0.15);
    border-radius: 2px;
    cursor: pointer;
    outline: none;
    transition: height 140ms ease;
  }
  .timeline:hover,
  .timeline:active {
    height: 6px;
  }
  .timeline::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 14px;
    height: 14px;
    background: var(--accent, #4f8ef7);
    border: 2px solid #fff;
    border-radius: 50%;
    cursor: pointer;
    opacity: 0;
    transition: opacity 140ms ease;
  }
  .timeline:hover::-webkit-slider-thumb,
  .timeline:active::-webkit-slider-thumb {
    opacity: 1;
  }

  .play-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 44px;
    height: 44px;
    padding: 0;
    background: rgba(255, 255, 255, 0.1);
    border: none;
    border-radius: 50%;
    color: #fff;
    cursor: pointer;
    transition: background-color 140ms ease, transform 140ms ease;
  }
  .play-btn:hover {
    background: rgba(255, 255, 255, 0.2);
    transform: scale(1.04);
  }
  .play-btn:active {
    transform: scale(0.96);
  }

  .time {
    font-variant-numeric: tabular-nums;
    font-size: 0.9rem;
    color: rgba(255, 255, 255, 0.85);
    letter-spacing: 0.02em;
  }
  .time-sep {
    margin: 0 6px;
    opacity: 0.4;
  }

  .spacer {
    flex: 1 1 auto;
  }

  /* Volume -------------------------------------------------------------- */

  .volume-group {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .volume-slider {
    -webkit-appearance: none;
    appearance: none;
    width: 90px;
    height: 4px;
    background: rgba(255, 255, 255, 0.18);
    border-radius: 2px;
    cursor: pointer;
    outline: none;
    transition: height 120ms ease;
  }
  .volume-slider:hover {
    height: 6px;
  }
  .volume-slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 12px;
    height: 12px;
    background: #fff;
    border-radius: 50%;
    cursor: pointer;
  }

  /* PiP bar (toujours visible, sous la vidéo) -------------------------- */

  .pip-bar {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    background: rgba(15, 15, 20, 0.98);
    border-top: 1px solid rgba(255, 255, 255, 0.06);
  }
  .pip-drag {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 26px;
    padding: 0;
    background: transparent;
    border: none;
    color: rgba(255, 255, 255, 0.5);
    cursor: grab;
  }
  .pip-drag:active {
    cursor: grabbing;
    color: #fff;
  }
  .pip-btn {
    width: 26px;
    height: 26px;
    border-radius: 50%;
  }
  .pip-time {
    font-variant-numeric: tabular-nums;
    font-size: 0.78rem;
    color: rgba(255, 255, 255, 0.85);
    min-width: 38px;
  }
  .pip-progress {
    flex: 1 1 auto;
    height: 3px;
    background: rgba(255, 255, 255, 0.15);
    border-radius: 2px;
    overflow: hidden;
  }
  .pip-progress-fill {
    height: 100%;
    background: var(--accent, #4f8ef7);
    transition: width 200ms linear;
  }

  /* Curseur masqué en fullscreen quand contrôles cachés ----------------- */

  .player-overlay.fullscreen.controls-hidden {
    cursor: none;
  }
</style>
