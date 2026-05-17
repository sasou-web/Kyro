<script lang="ts">
  /**
   * Page `/logs` — vue du fichier de log le plus récent (tâche 15.8).
   *
   * Rôle :
   *   1. Au mount, appeler la commande backend `read_latest_log`
   *      (cf. `commands::logs::read_latest_log`, tronqué à 5 MiB) ;
   *   2. Afficher le contenu dans un `<pre>` monospace, défilement
   *      vertical, `max-height: calc(100vh - 200px)` pour laisser la
   *      place à la `TopBar`, aux bandeaux et au padding du layout
   *      racine ;
   *   3. Exposer un bouton « Recharger » qui relit le fichier — utile
   *      si un scan ou une lecture vient d'écrire dans `kyro.log` et
   *      que l'utilisateur veut voir les nouvelles lignes ;
   *   4. En cas d'erreur (commande qui throw), afficher le message
   *      de fallback `Impossible de lire les logs : <cause>` à la
   *      place du `<pre>`.
   *
   * Style : tokens Adwaita Dark (`--surface`, `--border`,
   * `--text-primary`, `--text-secondary`, `--accent`, `--radius-card`,
   * `--radius-control`, `--space-*`).
   *
   * _Requirements: 18.3_
   */
  import { onMount } from 'svelte';

  import { readLatestLog } from '$lib/api/commands';

  /**
   * Contenu courant du fichier de log. `null` tant que la première
   * lecture n'est pas terminée — permet de distinguer « en cours de
   * chargement » de « lu mais vide » (chaîne vide).
   */
  let content: string | null = null;

  /**
   * Message d'erreur à afficher si `read_latest_log` échoue. Préfixé
   * par « Impossible de lire les logs : » pour rester cohérent avec
   * le ton FR du reste de l'UI.
   */
  let errorMessage: string | null = null;

  /**
   * `true` pendant qu'un appel à `readLatestLog` est en vol — utilisé
   * pour désactiver le bouton « Recharger » et éviter un double-fetch
   * accidentel.
   */
  let loading = false;

  /**
   * Lit le dernier fichier de log via la commande Tauri. Réinitialise
   * l'état avant l'appel pour que les anciennes données ne restent
   * pas visibles si la nouvelle lecture échoue, et ne touche
   * volontairement pas à `content` si l'appel throw — ainsi le
   * fallback d'erreur s'affiche sans contenu obsolète en arrière-plan.
   */
  async function loadLog(): Promise<void> {
    loading = true;
    errorMessage = null;
    try {
      content = await readLatestLog();
    } catch (err) {
      content = null;
      const cause = err instanceof Error ? err.message : String(err);
      errorMessage = `Impossible de lire les logs : ${cause}`;
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    void loadLog();
  });
</script>

<section class="logs" aria-labelledby="logs-title">
  <header class="logs-header">
    <div class="logs-titles">
      <h1 id="logs-title">Journal</h1>
      <p class="logs-subtitle">
        Contenu du fichier de log le plus récent (tronqué à 5 Mo).
      </p>
    </div>
    <button
      type="button"
      class="primary"
      on:click={loadLog}
      disabled={loading}
      data-testid="logs-reload"
    >
      {loading ? 'Chargement…' : 'Recharger'}
    </button>
  </header>

  {#if errorMessage !== null}
    <p class="logs-error" role="alert" data-testid="logs-error">
      {errorMessage}
    </p>
  {:else if content === null}
    <p class="logs-loading" data-testid="logs-loading">Chargement du journal…</p>
  {:else if content === ''}
    <p class="logs-empty" data-testid="logs-empty">
      Aucun log enregistré pour le moment.
    </p>
  {:else}
    <pre class="logs-content" data-testid="logs-content">{content}</pre>
  {/if}
</section>

<style>
  .logs {
    display: flex;
    flex-direction: column;
    gap: var(--space-4, 16px);
    min-height: 0;
  }

  .logs-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-4, 16px);
    flex-wrap: wrap;
  }

  .logs-titles {
    display: flex;
    flex-direction: column;
    gap: var(--space-1, 4px);
    min-width: 0;
  }

  .logs-titles h1 {
    margin: 0;
    font-size: 1.25rem;
    font-weight: 600;
    color: var(--text-primary);
  }

  .logs-subtitle {
    margin: 0;
    color: var(--text-secondary);
    font-size: 0.9rem;
  }

  /*
   * Bouton « Recharger » : tokens du thème (`button.primary` est déjà
   * stylé par `theme.css`, on n'ajuste que la marge). On garde
   * `cursor: not-allowed` quand `disabled` pour signaler qu'un appel
   * est en vol.
   */
  button.primary {
    padding: var(--space-2, 8px) var(--space-4, 16px);
    border: 1px solid var(--border);
    cursor: pointer;
    font-family: inherit;
    font-size: 0.9rem;
  }

  button.primary:disabled {
    cursor: not-allowed;
    opacity: 0.6;
  }

  /*
   * Bloc principal : `<pre>` monospace, défilement vertical, hauteur
   * bornée à la fenêtre moins ~200 px (TopBar + bandeaux + header).
   * `white-space: pre-wrap` + `word-break: break-word` pour éviter
   * le défilement horizontal sur les longues lignes JSON.
   */
  .logs-content {
    margin: 0;
    padding: var(--space-3, 12px);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-card, 12px);
    color: var(--text-primary);
    font-family: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas,
      "Liberation Mono", monospace;
    font-size: 0.85rem;
    line-height: 1.45;
    max-height: calc(100vh - 200px);
    overflow-y: auto;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .logs-loading,
  .logs-empty {
    margin: 0;
    color: var(--text-secondary);
    font-size: 0.9rem;
  }

  .logs-error {
    margin: 0;
    padding: var(--space-3, 12px);
    background: var(--surface);
    border: 1px solid #e01b24;
    border-radius: var(--radius-control, 8px);
    color: var(--text-primary);
    font-size: 0.9rem;
  }
</style>
