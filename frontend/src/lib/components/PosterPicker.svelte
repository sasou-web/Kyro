<script lang="ts">
  /**
   * PosterPicker — Tâche 14.7 (Requirements 6.2, 6.3).
   *
   * Bouton qui :
   *   1. ouvre la boîte de dialogue native d'ouverture de fichier
   *      (Tauri v2 plugin `@tauri-apps/plugin-dialog`) avec un filtre
   *      restreint aux extensions image courantes (`png`, `jpg`,
   *      `jpeg`, `webp`, `bmp`) — équivalent du filtre `image/*`
   *      demandé dans la sous-tâche ;
   *   2. selon `kind`, appelle `import_poster_for_media` ou
   *      `import_poster_for_episode` côté backend pour copier
   *      l'image dans `%APPDATA%\Kyro\posters\` puis mettre à jour
   *      `poster_path` dans la DB ;
   *   3. affiche un petit message de statut (succès / erreur) à
   *      côté du bouton et émet `imported` avec le chemin final
   *      retourné par le backend pour permettre au parent
   *      (formulaires d'édition Media_Item / Episode_Item) de
   *      rafraîchir l'affiche.
   *
   * Prérequis :
   *   - le plugin Tauri `tauri-plugin-dialog` est enregistré côté
   *     backend (`main.rs` : `.plugin(tauri_plugin_dialog::init())`)
   *     et la permission `dialog:allow-open` est listée dans
   *     `capabilities/default.json` ;
   *   - la permission `core:path:default` (déjà active) autorise la
   *     résolution `appDataDir()` utilisée pour calculer le dossier
   *     `posters/` injecté à la commande Rust.
   *
   * _Requirements: 6.2, 6.3_
   */

  import { createEventDispatcher } from 'svelte';

  import { open } from '@tauri-apps/plugin-dialog';
  import { appDataDir, join } from '@tauri-apps/api/path';

  import {
    importPosterForEpisode,
    importPosterForMedia
  } from '$lib/api/commands';

  /**
   * Discriminant côté backend : importe une affiche pour un
   * `Media_Item` (Films / racine Série / Anime) ou pour un
   * `Episode_Item` (Requirement 6.5).
   */
  export let kind: 'media' | 'episode';

  /**
   * Identifiant numérique du `Media_Item` ou de l'`Episode_Item`
   * cible. Doit être > 0 (les IDs SQLite auto-incrémentés
   * commencent à 1).
   */
  export let id: number;

  /**
   * Libellé personnalisable du bouton. Par défaut : « Choisir une
   * affiche… » — cohérent avec le ton FR du reste de l'UI.
   */
  export let label: string = 'Choisir une affiche…';

  /**
   * Désactive le bouton (utile pendant la soumission d'un
   * formulaire d'édition englobant, par exemple).
   */
  export let disabled = false;

  /** État interne — vrai pendant l'import (dialogue ou copie). */
  let busy = false;

  /**
   * Message d'état affiché à côté du bouton. `null` ⇒ pas de
   * message ; `kind: 'success'` ⇒ confirmation d'import ; `kind:
   * 'error'` ⇒ message d'erreur (lecture dialogue, validation
   * extension, copie disque, écriture DB).
   */
  let status: { kind: 'success' | 'error'; message: string } | null = null;

  type ImportedDetail = { kind: 'media' | 'episode'; id: number; destPath: string };

  const dispatch = createEventDispatcher<{ imported: ImportedDetail }>();

  /**
   * Extensions image acceptées par le backend
   * (`infra::posters::SUPPORTED_EXTENSIONS`). On ajoute `bmp` côté
   * filtre dialogue pour matcher la spec de la sous-tâche, mais le
   * backend rejettera proprement l'import via une `KyroError::
   * Validation` (« extension non supportée ») — ce cas est traité
   * par la branche d'erreur ci-dessous.
   */
  const IMAGE_EXTENSIONS = ['png', 'jpg', 'jpeg', 'webp', 'bmp'];

  async function handleClick(): Promise<void> {
    if (busy || disabled) return;

    busy = true;
    status = null;

    try {
      // 1. Ouverture du picker natif. `multiple: false` ⇒ Tauri
      //    retourne `string | null` directement (pas un tableau).
      const selected = await open({
        multiple: false,
        directory: false,
        filters: [
          {
            name: 'Image',
            extensions: IMAGE_EXTENSIONS
          }
        ]
      });

      // L'utilisateur a annulé ⇒ on ne touche ni au statut (pas
      // d'erreur à afficher) ni au poster déjà associé.
      if (selected === null || typeof selected !== 'string') {
        return;
      }

      // 2. Résolution du dossier `%APPDATA%\Kyro\posters\` côté
      //    frontend. La commande Rust accepte `posters_dir` en
      //    paramètre injectable (cf. `infra::posters::import_poster`)
      //    pour faciliter les tests unitaires côté backend ; ici on
      //    le matérialise systématiquement en `appDataDir()/posters`.
      const dataDir = await appDataDir();
      const postersDir = await join(dataDir, 'posters');

      // 3. Délégation au backend selon `kind`. Le backend valide
      //    l'extension et copie le fichier dans `posters_dir/<prefix>-
      //    <id>.<ext>` puis met à jour `poster_path` dans une
      //    transaction ; le chemin final est retourné.
      const destPath =
        kind === 'media'
          ? await importPosterForMedia(id, selected, postersDir)
          : await importPosterForEpisode(id, selected, postersDir);

      status = { kind: 'success', message: 'Affiche importée.' };
      dispatch('imported', { kind, id, destPath });
    } catch (err) {
      // Le backend renvoie une `String` (cf. `KyroError` ⇒
      // `tauri::InvokeError`). On la conserve telle quelle pour ne
      // pas masquer le message localisé FR construit côté Rust.
      const message =
        err instanceof Error ? err.message : typeof err === 'string' ? err : String(err);
      status = { kind: 'error', message: `Échec de l'import : ${message}` };
    } finally {
      busy = false;
    }
  }
</script>

<div class="poster-picker">
  <button
    type="button"
    class="primary"
    on:click={handleClick}
    disabled={disabled || busy}
    aria-busy={busy}
  >
    {busy ? 'Import en cours…' : label}
  </button>

  {#if status !== null}
    <p
      class="status"
      class:status--success={status.kind === 'success'}
      class:status--error={status.kind === 'error'}
      role={status.kind === 'error' ? 'alert' : 'status'}
    >
      {status.message}
    </p>
  {/if}
</div>

<style>
  .poster-picker {
    display: flex;
    align-items: center;
    gap: var(--space-3, 12px);
    flex-wrap: wrap;
  }

  /* Bouton : on s'aligne sur `button.primary` défini dans
     `theme.css` (Adwaita Dark, requirements 16.8/16.9/16.11) tout
     en garantissant un état désactivé non-ambigu. */
  button {
    padding: var(--space-2, 8px) var(--space-4, 16px);
    border: 1px solid var(--border);
    border-radius: var(--radius-control, 8px);
    background: var(--accent);
    color: var(--text-primary);
    font-family: var(--font-family);
    font-size: 14px;
    cursor: pointer;
  }

  button:hover:not(:disabled) {
    background: var(--accent-hover);
  }

  button:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .status {
    margin: 0;
    font-size: 0.875rem;
  }

  .status--success {
    color: var(--text-secondary);
  }

  .status--error {
    /* Rouge Adwaita destructive — cohérent avec `RatingInput`. */
    color: #e01b24;
  }
</style>
