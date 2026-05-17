<script lang="ts">
  /**
   * Page Paramètres — Tâche 15.7.
   *
   * Six sections verticales, chacune adressant un sous-ensemble des
   * exigences couvertes par la page (cf. `design.md` § « Routing » et
   * `requirements.md` Req. 2, 3.7, 5.4, 9.7, 10.x, 16.14) :
   *
   *  (a) **Dossiers de bibliothèque** — liste des `LibraryFolder` avec
   *      catégorie, bouton × pour retirer (`removeLibraryFolder`), et
   *      formulaire d'ajout (sélecteur de dossier natif via Tauri
   *      `plugin-dialog` + `<select>` catégorie + bouton « Ajouter »
   *      qui appelle `addLibraryFolder`). Si le backend rejette le
   *      chemin (inexistant, déjà existant), on affiche le message
   *      d'erreur tel quel à côté du formulaire (Requirement 2.4).
   *      _Requirements: 2.1, 2.2, 2.3, 2.4, 2.5_
   *
   *  (b) **Lecteur externe** — champ texte pré-rempli avec
   *      `external_player_path` (issu de `SettingsSummary`), bouton
   *      « Parcourir… » qui ouvre le dialogue natif filtré sur `.exe`,
   *      bouton « Vérifier » qui re-soumet le chemin courant à
   *      `setExternalPlayerPath` (la commande valide existence +
   *      extension `.exe` côté backend, requirement 10.3) et affiche
   *      « Lecteur trouvé » sur succès / le message d'erreur sur
   *      échec. Le bouton « Enregistrer » persiste également.
   *      _Requirements: 10.1, 10.2, 10.3, 10.4_
   *
   *  (c) **Chemin ffprobe** — champ texte pré-rempli avec
   *      `ffprobe_path`, bouton « Parcourir… ». Le drapeau
   *      `flags.ffprobeMissing` (posé par l'événement
   *      `ffprobe-missing` au boot) est utilisé pour afficher un
   *      avertissement « ffprobe introuvable » ; sinon on affiche un
   *      simple « ffprobe configuré ». L'agrégat `SettingsSummary`
   *      n'expose pas de drapeau « sidecar détecté » distinct, donc
   *      on s'en tient au fait que le binaire est résoluble.
   *      _Requirements: 5.4_
   *
   *  (d) **Scan au démarrage** — toggle bound sur
   *      `settings.scan_at_startup`, persistence via
   *      `setScanAtStartup` à chaque changement.
   *      _Requirements: 3.7_
   *
   *  (e) **Thème** — bouton qui appelle `toggleTheme()` (le store
   *      gère persistance + application DOM). Affiche le thème
   *      courant.
   *      _Requirements: 16.14_
   *
   *  (f) **Installer le script Lua** — bouton qui appelle
   *      `installLuaScript` ; affiche le chemin retourné sur succès,
   *      le message d'erreur sur échec.
   *      _Requirements: 9.7_
   *
   * Chaque mutation est suivie d'un `getSettingsSummary` pour mettre
   * à jour le store `settings` et rafraîchir l'affichage côté
   * Sidebar / TopBar / autres consommateurs.
   *
   * Style : Adwaita Dark via les variables CSS de
   * `frontend/src/styles/theme.css`. Headers de section, espacement
   * vertical homogène.
   *
   * _Requirements: 2.1, 2.2, 2.3, 2.4, 2.5, 3.7, 5.4, 9.7, 10.1, 10.2, 10.3, 10.4, 16.14_
   */

  import { open, save } from '@tauri-apps/plugin-dialog';

  import {
    addLibraryFolder,
    exportLibraryBackup,
    getAppVersion,
    getSettingsSummary,
    installLuaScript,
    listLibraryFolders,
    openReleasesPage,
    removeLibraryFolder,
    restoreLibraryBackup,
    setExternalPlayerPath,
    setFfprobePath,
    setScanAtStartup
  } from '$lib/api/commands';
  import type { Category } from '$lib/api/types';
  import { flags, settings } from '$lib/stores/library';
  import { theme, toggleTheme } from '$lib/stores/theme';

  // ---------------------------------------------------------------------------
  // (a) Dossiers de bibliothèque — état local du formulaire d'ajout.
  // ---------------------------------------------------------------------------

  /** Chemin saisi (ou retourné par le dialogue de sélection) avant ajout. */
  let newFolderPath = '';

  /** Catégorie sélectionnée pour le nouveau dossier. */
  let newFolderCategory: Category = 'Films';

  /** Message d'erreur à afficher dans la section dossiers (null = pas d'erreur). */
  let foldersError: string | null = null;

  /** Vrai pendant le `addLibraryFolder` ⇒ désactive le bouton. */
  let foldersBusy = false;

  // ---------------------------------------------------------------------------
  // (b) Lecteur externe — état local.
  // ---------------------------------------------------------------------------

  /** Brouillon du chemin External_Player avant `setExternalPlayerPath`. */
  let externalPlayerDraft = '';

  /** Statut du dernier `Vérifier` / `Enregistrer`. */
  let externalPlayerStatus:
    | { kind: 'success' | 'error'; message: string }
    | null = null;

  /** Vrai pendant la persistence ⇒ désactive les boutons. */
  let externalPlayerBusy = false;

  // ---------------------------------------------------------------------------
  // (c) Chemin ffprobe — état local.
  // ---------------------------------------------------------------------------

  /** Brouillon du chemin ffprobe avant `setFfprobePath`. */
  let ffprobeDraft = '';

  /** Statut du dernier `Enregistrer`. */
  let ffprobeStatus: { kind: 'success' | 'error'; message: string } | null = null;

  /** Vrai pendant la persistence ⇒ désactive les boutons. */
  let ffprobeBusy = false;

  // ---------------------------------------------------------------------------
  // (f) Installation du script Lua — état local.
  // ---------------------------------------------------------------------------

  /** Statut du dernier `installLuaScript`. */
  let luaStatus: { kind: 'success' | 'error'; message: string } | null = null;

  // ---------------------------------------------------------------------------
  // (g) Sauvegarde / restauration (v1.0).
  // ---------------------------------------------------------------------------

  /** Statut du dernier export/restore. */
  let backupStatus:
    | { kind: 'success' | 'error'; message: string }
    | null = null;

  /** Vrai pendant un export ou un restore (désactive les boutons). */
  let backupBusy = false;

  // ---------------------------------------------------------------------------
  // (h) À propos / version (v1.0).
  // ---------------------------------------------------------------------------

  /** Version applicative chargée au mount via `get_app_version`. */
  let appVersion: string | null = null;
  /** URL des releases — exposée pour affichage clic-droit / copy. */
  let releasesUrl: string | null = null;

  /** Vrai pendant l'installation. */
  let luaBusy = false;

  // ---------------------------------------------------------------------------
  // Synchronisation des brouillons avec le store `settings`.
  //
  // On ne veut pas écraser une saisie utilisateur en cours quand le
  // store change (rafraîchissement post-mutation). Les `$:` ci-dessous
  // initialisent les brouillons une fois quand le store passe de
  // `null` à une valeur, puis ne touchent plus aux drafts (le
  // formulaire est dès lors maître).
  // ---------------------------------------------------------------------------

  let draftsInitialised = false;

  $: if (!draftsInitialised && $settings !== null) {
    externalPlayerDraft = $settings.external_player_path ?? '';
    ffprobeDraft = $settings.ffprobe_path ?? '';
    draftsInitialised = true;
  }

  /**
   * Recharge l'agrégat `SettingsSummary` après une mutation et écrit
   * dans le store `settings`. Appelée par `addLibraryFolder`,
   * `removeLibraryFolder`, `setExternalPlayerPath`, `setFfprobePath`,
   * `setScanAtStartup` — pour que les autres parties de l'UI
   * (Sidebar, TopBar) voient la nouvelle valeur immédiatement.
   */
  async function refreshSettings(): Promise<void> {
    try {
      const summary = await getSettingsSummary();
      settings.set(summary);
    } catch (err) {
      // Erreur non-fatale : on garde la valeur précédente du store ;
      // les bandeaux globaux du layout informeront l'utilisateur si
      // le backend devient indisponible.
      console.warn('Kyro: échec refresh settings', err);
    }
  }

  /** Extrait le message lisible d'une erreur Tauri (`String`). */
  function toMessage(err: unknown): string {
    if (err instanceof Error) return err.message;
    if (typeof err === 'string') return err;
    return String(err);
  }

  // ---------------------------------------------------------------------------
  // (a) Handlers — dossiers de bibliothèque.
  // ---------------------------------------------------------------------------

  /**
   * Ouvre le dialogue natif de sélection de dossier (Tauri v2
   * `plugin-dialog`). Le résultat est écrit dans `newFolderPath`.
   * Annulation utilisateur ⇒ no-op.
   */
  async function pickFolder(): Promise<void> {
    foldersError = null;
    try {
      const selected = await open({
        directory: true,
        multiple: false
      });
      if (typeof selected === 'string') {
        newFolderPath = selected;
      }
    } catch (err) {
      foldersError = toMessage(err);
    }
  }

  /**
   * Ajoute le dossier saisi avec la catégorie sélectionnée. Si le
   * backend rejette (chemin inexistant ⇒ `KyroError::Validation`),
   * on affiche le message tel quel (Requirement 2.4).
   */
  async function handleAddFolder(): Promise<void> {
    if (foldersBusy) return;
    foldersError = null;

    const trimmed = newFolderPath.trim();
    if (trimmed === '') {
      foldersError = 'Veuillez sélectionner un dossier.';
      return;
    }

    foldersBusy = true;
    try {
      await addLibraryFolder(trimmed, newFolderCategory);
      // Rafraîchissement de la liste avant reset du formulaire pour
      // que l'utilisateur voie l'entrée apparaître.
      await refreshSettings();
      newFolderPath = '';
    } catch (err) {
      foldersError = toMessage(err);
    } finally {
      foldersBusy = false;
    }
  }

  /** Retire un dossier (les items rattachés sont marqués indisponibles). */
  async function handleRemoveFolder(id: number): Promise<void> {
    if (foldersBusy) return;
    foldersError = null;
    foldersBusy = true;
    try {
      await removeLibraryFolder(id);
      await refreshSettings();
    } catch (err) {
      foldersError = toMessage(err);
    } finally {
      foldersBusy = false;
    }
  }

  // ---------------------------------------------------------------------------
  // (b) Handlers — lecteur externe.
  // ---------------------------------------------------------------------------

  /**
   * Ouvre le dialogue natif filtré sur `.exe`. Annulation ⇒ no-op.
   */
  async function pickExternalPlayer(): Promise<void> {
    externalPlayerStatus = null;
    try {
      const selected = await open({
        directory: false,
        multiple: false,
        filters: [
          { name: 'Exécutable', extensions: ['exe'] }
        ]
      });
      if (typeof selected === 'string') {
        externalPlayerDraft = selected;
      }
    } catch (err) {
      externalPlayerStatus = { kind: 'error', message: toMessage(err) };
    }
  }

  /**
   * Persiste le chemin courant. La commande backend valide existence
   * + extension `.exe` (requirements 10.3, 10.5) et rejette sinon.
   * Sert aussi de « Vérifier » : un appel idempotent qui rapporte
   * succès si le binaire est valide, message d'erreur sinon.
   */
  async function handleSaveExternalPlayer(): Promise<void> {
    if (externalPlayerBusy) return;
    externalPlayerStatus = null;

    const trimmed = externalPlayerDraft.trim();
    if (trimmed === '') {
      externalPlayerStatus = {
        kind: 'error',
        message: 'Veuillez sélectionner un exécutable.'
      };
      return;
    }

    externalPlayerBusy = true;
    try {
      await setExternalPlayerPath(trimmed);
      await refreshSettings();
      externalPlayerStatus = { kind: 'success', message: 'Lecteur enregistré.' };
    } catch (err) {
      externalPlayerStatus = { kind: 'error', message: toMessage(err) };
    } finally {
      externalPlayerBusy = false;
    }
  }

  /**
   * « Vérifier » — re-soumet le chemin courant pour déclencher la
   * validation backend (existence + `.exe`). On utilise
   * `setExternalPlayerPath` qui est idempotent. Les messages de
   * succès / erreur sont distincts pour communiquer le sens
   * « Vérifier » plutôt qu'« Enregistrer ».
   */
  async function handleVerifyExternalPlayer(): Promise<void> {
    if (externalPlayerBusy) return;
    externalPlayerStatus = null;

    const trimmed = externalPlayerDraft.trim();
    if (trimmed === '') {
      externalPlayerStatus = {
        kind: 'error',
        message: 'Veuillez sélectionner un exécutable.'
      };
      return;
    }

    externalPlayerBusy = true;
    try {
      await setExternalPlayerPath(trimmed);
      await refreshSettings();
      externalPlayerStatus = { kind: 'success', message: 'Lecteur trouvé.' };
    } catch (err) {
      externalPlayerStatus = { kind: 'error', message: toMessage(err) };
    } finally {
      externalPlayerBusy = false;
    }
  }

  // ---------------------------------------------------------------------------
  // (c) Handlers — chemin ffprobe.
  // ---------------------------------------------------------------------------

  async function pickFfprobe(): Promise<void> {
    ffprobeStatus = null;
    try {
      const selected = await open({
        directory: false,
        multiple: false,
        filters: [
          { name: 'Exécutable', extensions: ['exe'] }
        ]
      });
      if (typeof selected === 'string') {
        ffprobeDraft = selected;
      }
    } catch (err) {
      ffprobeStatus = { kind: 'error', message: toMessage(err) };
    }
  }

  async function handleSaveFfprobe(): Promise<void> {
    if (ffprobeBusy) return;
    ffprobeStatus = null;

    const trimmed = ffprobeDraft.trim();
    if (trimmed === '') {
      ffprobeStatus = {
        kind: 'error',
        message: 'Veuillez sélectionner ffprobe.exe.'
      };
      return;
    }

    ffprobeBusy = true;
    try {
      await setFfprobePath(trimmed);
      await refreshSettings();
      // L'utilisateur a fourni un chemin valide : on retire le drapeau
      // ffprobe-missing pour masquer le bandeau et le badge TopBar.
      flags.update((f) => ({ ...f, ffprobeMissing: false }));
      ffprobeStatus = { kind: 'success', message: 'Chemin ffprobe enregistré.' };
    } catch (err) {
      ffprobeStatus = { kind: 'error', message: toMessage(err) };
    } finally {
      ffprobeBusy = false;
    }
  }

  // ---------------------------------------------------------------------------
  // (d) Handler — scan au démarrage.
  // ---------------------------------------------------------------------------

  /**
   * Persiste l'état du toggle `scan_at_startup`. Appelé sur chaque
   * `change` du `<input type="checkbox">`.
   */
  async function handleToggleScanAtStartup(
    event: Event & { currentTarget: EventTarget & HTMLInputElement }
  ): Promise<void> {
    const enabled = event.currentTarget.checked;
    try {
      await setScanAtStartup(enabled);
      await refreshSettings();
    } catch (err) {
      // En cas d'erreur, on remet le checkbox dans son état précédent
      // pour ne pas laisser l'UI désynchronisée du backend.
      event.currentTarget.checked = !enabled;
      console.warn('Kyro: échec setScanAtStartup', err);
    }
  }

  // ---------------------------------------------------------------------------
  // (e) Handler — thème.
  // ---------------------------------------------------------------------------

  async function handleToggleTheme(): Promise<void> {
    try {
      await toggleTheme();
      await refreshSettings();
    } catch (err) {
      console.warn('Kyro: échec toggleTheme', err);
    }
  }

  // ---------------------------------------------------------------------------
  // (f) Handler — installation du script Lua.
  // ---------------------------------------------------------------------------

  async function handleInstallLuaScript(): Promise<void> {
    if (luaBusy) return;
    luaBusy = true;
    luaStatus = null;
    try {
      const dest = await installLuaScript();
      luaStatus = { kind: 'success', message: `Script installé : ${dest}` };
    } catch (err) {
      luaStatus = { kind: 'error', message: toMessage(err) };
    } finally {
      luaBusy = false;
    }
  }

  // ---------------------------------------------------------------------------
  // (g) Handlers — sauvegarde et restauration.
  // ---------------------------------------------------------------------------

  /**
   * Ouvre un dialogue natif `Save As` pour choisir où écrire l'archive
   * de sauvegarde, puis appelle `exportLibraryBackup`. Le manifest
   * retourné est résumé dans le statut affiché.
   */
  async function handleExportBackup(): Promise<void> {
    if (backupBusy) return;
    backupBusy = true;
    backupStatus = null;
    try {
      const stamp = new Date().toISOString().slice(0, 10).replace(/-/g, '');
      const defaultName = `kyro-backup-${stamp}.zip`;
      const dest = await save({
        title: 'Sauvegarder la bibliothèque Kyro',
        defaultPath: defaultName,
        filters: [{ name: 'Archive Kyro', extensions: ['zip'] }]
      });
      if (!dest) {
        // L'utilisateur a annulé le dialogue.
        backupBusy = false;
        return;
      }
      const manifest = await exportLibraryBackup(dest);
      backupStatus = {
        kind: 'success',
        message: `Sauvegarde créée : ${manifest.media_count} films/séries, ${manifest.episode_count} épisodes, ${manifest.posters_count} affiches.`
      };
    } catch (err) {
      backupStatus = { kind: 'error', message: toMessage(err) };
    } finally {
      backupBusy = false;
    }
  }

  /**
   * Ouvre un dialogue `Open` pour sélectionner une archive Kyro,
   * confirme avec l'utilisateur, restaure, et recharge la WebView
   * pour rouvrir la base sur le nouveau fichier.
   */
  async function handleRestoreBackup(): Promise<void> {
    if (backupBusy) return;
    backupBusy = true;
    backupStatus = null;
    try {
      const picked = await open({
        title: 'Restaurer une sauvegarde Kyro',
        multiple: false,
        directory: false,
        filters: [{ name: 'Archive Kyro', extensions: ['zip'] }]
      });
      if (!picked) {
        backupBusy = false;
        return;
      }
      const archivePath = Array.isArray(picked) ? picked[0] : picked;
      const ok = window.confirm(
        'La restauration va remplacer la bibliothèque actuelle. ' +
          'Une copie de sécurité est créée automatiquement à côté de ' +
          'la base courante. Continuer ?'
      );
      if (!ok) {
        backupBusy = false;
        return;
      }
      const manifest = await restoreLibraryBackup(archivePath);
      backupStatus = {
        kind: 'success',
        message: `Restauration OK (${manifest.media_count} films/séries, ${manifest.episode_count} épisodes). Rechargement…`
      };
      // Recharge pour réouvrir le pool sur la base restaurée.
      setTimeout(() => {
        if (typeof window !== 'undefined') {
          window.location.reload();
        }
      }, 1200);
    } catch (err) {
      backupStatus = { kind: 'error', message: toMessage(err) };
      backupBusy = false;
    }
  }

  // ---------------------------------------------------------------------------
  // (h) Handlers — version et page releases.
  // ---------------------------------------------------------------------------

  async function loadAppVersion(): Promise<void> {
    try {
      const info = await getAppVersion();
      appVersion = info.version;
      releasesUrl = info.releases_url;
    } catch (err) {
      console.warn('Kyro: échec getAppVersion', err);
    }
  }

  async function handleOpenReleases(): Promise<void> {
    try {
      await openReleasesPage();
    } catch (err) {
      console.warn('Kyro: échec openReleasesPage', err);
    }
  }

  // ---------------------------------------------------------------------------
  // Listing des dossiers (réactif au store).
  //
  // On dérive depuis `$settings.library_folders` quand l'agrégat est
  // disponible ; sinon on bascule sur un fetch direct
  // `listLibraryFolders` au montage pour être robuste si la page est
  // ouverte avant que `initSettings` n'ait abouti.
  // ---------------------------------------------------------------------------

  /** Fallback : fetch direct si le store n'est pas encore peuplé. */
  async function ensureFolders(): Promise<void> {
    if ($settings !== null) return;
    try {
      const folders = await listLibraryFolders();
      // On mute partiellement le store ; les autres champs restent
      // tels que dans `init.ts` (null) et seront peuplés par le
      // prochain `refreshSettings`.
      settings.update((current) => {
        if (current !== null) return current;
        return {
          library_folders: folders,
          external_player_path: null,
          ffprobe_path: null,
          scan_at_startup: false,
          theme: 'adwaita-dark',
          last_category: null
        };
      });
    } catch (err) {
      console.warn('Kyro: échec listLibraryFolders', err);
    }
  }

  // Appel one-shot ; pas de réactivité nécessaire.
  void ensureFolders();
  void loadAppVersion();
</script>

<section class="settings" aria-labelledby="settings-title">
  <h1 id="settings-title">Paramètres</h1>

  <!-- (a) Dossiers de bibliothèque ------------------------------------- -->
  <article class="card" aria-labelledby="folders-heading" data-testid="section-folders">
    <h2 id="folders-heading">Dossiers de bibliothèque</h2>

    <ul class="folders" aria-label="Dossiers configurés">
      {#if $settings === null || $settings.library_folders.length === 0}
        <li class="empty">Aucun dossier configuré.</li>
      {:else}
        {#each $settings.library_folders as folder (folder.id)}
          <li class="folder-row">
            <span class="folder-category">{folder.category}</span>
            <span class="folder-path" title={folder.path}>{folder.path}</span>
            <button
              type="button"
              class="folder-remove"
              aria-label="Retirer ce dossier"
              on:click={() => handleRemoveFolder(folder.id)}
              disabled={foldersBusy}
            >
              ✕
            </button>
          </li>
        {/each}
      {/if}
    </ul>

    <form class="folder-form" on:submit|preventDefault={handleAddFolder}>
      <div class="folder-form-row">
        <input
          type="text"
          class="folder-input"
          placeholder="Chemin du dossier"
          bind:value={newFolderPath}
          aria-label="Chemin du dossier"
        />
        <button type="button" on:click={pickFolder}>Parcourir…</button>
        <select bind:value={newFolderCategory} aria-label="Catégorie">
          <option value="Films">Films</option>
          <option value="Series">Séries</option>
          <option value="Animes">Animes</option>
        </select>
        <button type="submit" class="primary" disabled={foldersBusy}>
          {foldersBusy ? 'Ajout…' : 'Ajouter'}
        </button>
      </div>

      {#if foldersError !== null}
        <p class="status status--error" role="alert" data-testid="folders-error">
          {foldersError}
        </p>
      {/if}
    </form>
  </article>

  <!-- (b) Lecteur externe --------------------------------------------- -->
  <article
    class="card"
    aria-labelledby="player-heading"
    data-testid="section-external-player"
  >
    <h2 id="player-heading">Lecteur externe</h2>

    <div class="path-row">
      <input
        type="text"
        class="path-input"
        placeholder="C:\Program Files\mpv.net\mpv.net.exe"
        bind:value={externalPlayerDraft}
        aria-label="Chemin du lecteur externe"
      />
      <button type="button" on:click={pickExternalPlayer}>Parcourir…</button>
      <button
        type="button"
        on:click={handleVerifyExternalPlayer}
        disabled={externalPlayerBusy}
        data-testid="verify-external-player"
      >
        Vérifier
      </button>
      <button
        type="button"
        class="primary"
        on:click={handleSaveExternalPlayer}
        disabled={externalPlayerBusy}
      >
        Enregistrer
      </button>
    </div>

    {#if externalPlayerStatus !== null}
      <p
        class="status"
        class:status--success={externalPlayerStatus.kind === 'success'}
        class:status--error={externalPlayerStatus.kind === 'error'}
        role={externalPlayerStatus.kind === 'error' ? 'alert' : 'status'}
        data-testid="external-player-status"
      >
        {externalPlayerStatus.message}
      </p>
    {/if}
  </article>

  <!-- (c) Chemin ffprobe ---------------------------------------------- -->
  <article class="card" aria-labelledby="ffprobe-heading" data-testid="section-ffprobe">
    <h2 id="ffprobe-heading">Chemin ffprobe</h2>

    <div class="path-row">
      <input
        type="text"
        class="path-input"
        placeholder="C:\Program Files\ffmpeg\bin\ffprobe.exe"
        bind:value={ffprobeDraft}
        aria-label="Chemin de ffprobe"
      />
      <button type="button" on:click={pickFfprobe}>Parcourir…</button>
      <button
        type="button"
        class="primary"
        on:click={handleSaveFfprobe}
        disabled={ffprobeBusy}
      >
        Enregistrer
      </button>
    </div>

    <p
      class="status status--info"
      data-testid="ffprobe-indicator"
      role="status"
    >
      {#if $flags.ffprobeMissing}
        ⚠ ffprobe introuvable — configurez un chemin valide ci-dessus.
      {:else if $settings !== null && $settings.ffprobe_path !== null && $settings.ffprobe_path !== ''}
        ✓ ffprobe configuré ({$settings.ffprobe_path})
      {:else}
        ✓ ffprobe résolu (sidecar ou PATH)
      {/if}
    </p>

    {#if ffprobeStatus !== null}
      <p
        class="status"
        class:status--success={ffprobeStatus.kind === 'success'}
        class:status--error={ffprobeStatus.kind === 'error'}
        role={ffprobeStatus.kind === 'error' ? 'alert' : 'status'}
      >
        {ffprobeStatus.message}
      </p>
    {/if}
  </article>

  <!-- (d) Scan au démarrage ------------------------------------------- -->
  <article
    class="card"
    aria-labelledby="scan-heading"
    data-testid="section-scan-at-startup"
  >
    <h2 id="scan-heading">Scan au démarrage</h2>
    <label class="toggle-row">
      <input
        type="checkbox"
        checked={$settings?.scan_at_startup ?? false}
        on:change={handleToggleScanAtStartup}
        data-testid="toggle-scan-at-startup"
      />
      <span>Lancer un scan automatique à l'ouverture de Kyro</span>
    </label>
  </article>

  <!-- (e) Thème ------------------------------------------------------- -->
  <article class="card" aria-labelledby="theme-heading" data-testid="section-theme">
    <h2 id="theme-heading">Thème</h2>
    <div class="path-row">
      <span class="theme-current" data-testid="theme-current">
        Thème courant : {$theme === 'adwaita-dark' ? 'Adwaita Dark' : 'Adwaita Light'}
      </span>
      <button
        type="button"
        class="primary"
        on:click={handleToggleTheme}
        data-testid="theme-toggle"
      >
        Basculer
      </button>
    </div>
  </article>

  <!-- (f) Installer le script Lua ------------------------------------- -->
  <article class="card" aria-labelledby="lua-heading" data-testid="section-lua">
    <h2 id="lua-heading">Script de reprise mpv.net</h2>
    <div class="path-row">
      <button
        type="button"
        class="primary"
        on:click={handleInstallLuaScript}
        disabled={luaBusy}
        data-testid="install-lua-script"
      >
        {luaBusy ? 'Installation…' : 'Installer le script Lua'}
      </button>
    </div>

    {#if luaStatus !== null}
      <p
        class="status"
        class:status--success={luaStatus.kind === 'success'}
        class:status--error={luaStatus.kind === 'error'}
        role={luaStatus.kind === 'error' ? 'alert' : 'status'}
        data-testid="lua-status"
      >
        {luaStatus.message}
      </p>
    {/if}
  </article>

  <!-- (g) Sauvegarde / restauration ------------------------------------ -->
  <article class="card" aria-labelledby="backup-heading" data-testid="section-backup">
    <h2 id="backup-heading">Sauvegarde et restauration</h2>
    <p class="hint">
      Exporte la bibliothèque (base de données + affiches importées)
      vers une archive ZIP que tu peux mettre de côté ou transférer
      sur une autre machine. La restauration remplace l'état courant ;
      une copie de sécurité est créée automatiquement avant écrasement.
    </p>
    <div class="path-row">
      <button
        type="button"
        class="primary"
        on:click={handleExportBackup}
        disabled={backupBusy}
        data-testid="backup-export"
      >
        {backupBusy ? 'En cours…' : 'Exporter la bibliothèque'}
      </button>
      <button
        type="button"
        class="secondary"
        on:click={handleRestoreBackup}
        disabled={backupBusy}
        data-testid="backup-restore"
      >
        Restaurer une sauvegarde…
      </button>
    </div>

    {#if backupStatus !== null}
      <p
        class="status"
        class:status--success={backupStatus.kind === 'success'}
        class:status--error={backupStatus.kind === 'error'}
        role={backupStatus.kind === 'error' ? 'alert' : 'status'}
        data-testid="backup-status"
      >
        {backupStatus.message}
      </p>
    {/if}
  </article>

  <!-- (h) À propos / mises à jour ---------------------------------------- -->
  <article class="card" aria-labelledby="about-heading" data-testid="section-about">
    <h2 id="about-heading">À propos</h2>
    <p class="hint" data-testid="about-version">
      Version actuelle : <strong>{appVersion ?? '…'}</strong>
    </p>
    <p class="hint">
      Pour mettre à jour Kyro, télécharge la dernière release depuis
      la page GitHub puis exécute le nouvel installeur. La
      configuration et la base de données sont conservées.
    </p>
    <div class="path-row">
      <button
        type="button"
        class="secondary"
        on:click={handleOpenReleases}
        data-testid="updates-open-releases"
      >
        Voir les releases…
      </button>
    </div>
    {#if releasesUrl}
      <p class="hint" style="word-break: break-all;">
        URL : <code>{releasesUrl}</code>
      </p>
    {/if}
  </article>
</section>

<style>
  .settings {
    display: flex;
    flex-direction: column;
    gap: var(--space-4, 16px);
    max-width: 900px;
    margin: 0 auto;
    padding: var(--space-2, 8px);
    color: var(--text-primary);
    font-family: var(--font-family);
  }

  h1 {
    margin: 0;
    font-size: 1.6rem;
    font-weight: 600;
  }

  .card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-card, 12px);
    padding: var(--space-4, 16px);
    display: flex;
    flex-direction: column;
    gap: var(--space-3, 12px);
    box-shadow: var(--shadow-card);
  }

  h2 {
    margin: 0;
    font-size: 1.1rem;
    font-weight: 600;
    color: var(--text-primary);
  }

  /* (a) Dossiers ----------------------------------------------------- */

  .folders {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1, 4px);
  }

  .folders .empty {
    color: var(--text-secondary);
    font-style: italic;
  }

  .folder-row {
    display: grid;
    grid-template-columns: 80px 1fr auto;
    align-items: center;
    gap: var(--space-3, 12px);
    padding: var(--space-2, 8px) var(--space-3, 12px);
    background: var(--bg-secondary);
    border: 1px solid var(--border);
    border-radius: var(--radius-control, 8px);
  }

  .folder-category {
    font-size: 0.85rem;
    color: var(--text-secondary);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .folder-path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-family);
    font-size: 0.95rem;
  }

  .folder-remove {
    background: transparent;
    color: var(--text-secondary);
    border: 1px solid var(--border);
    border-radius: var(--radius-control, 8px);
    padding: 2px 8px;
    cursor: pointer;
    line-height: 1.2;
  }

  .folder-remove:hover:not(:disabled) {
    background: #e01b24;
    color: #ffffff;
    border-color: #e01b24;
  }

  .folder-form {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
  }

  .folder-form-row {
    display: grid;
    grid-template-columns: 1fr auto auto auto;
    gap: var(--space-2, 8px);
    align-items: center;
  }

  .folder-input {
    min-width: 0;
  }

  /* (b) Path rows (player + ffprobe + theme + lua) ------------------- */

  .path-row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2, 8px);
    align-items: center;
  }

  .path-input {
    flex: 1 1 240px;
    min-width: 0;
  }

  /* Toggle ----------------------------------------------------------- */

  .toggle-row {
    display: flex;
    align-items: center;
    gap: var(--space-3, 12px);
    cursor: pointer;
    color: var(--text-primary);
  }

  .theme-current {
    color: var(--text-secondary);
    font-size: 0.95rem;
  }

  /* Statuts ---------------------------------------------------------- */

  .status {
    margin: 0;
    font-size: 0.875rem;
  }

  .status--success {
    color: var(--text-secondary);
  }

  .status--error {
    color: #e01b24;
  }

  .status--info {
    color: var(--text-secondary);
  }
</style>
