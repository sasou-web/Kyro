<script lang="ts">
  /**
   * Sidebar — barre de navigation latérale principale de Kyro.
   *
   * Expose six entrées correspondant aux routes top-level du SPA
   * SvelteKit (cf. `design.md` § « Routing ») :
   *
   * | Lien            | Route          | Couvre                       |
   * | --------------- | -------------- | ---------------------------- |
   * | Films           | `/films`       | Requirement 7.1, 7.2         |
   * | Séries          | `/series`      | Requirement 7.1, 7.2         |
   * | Animes          | `/animes`      | Requirement 7.1, 7.2         |
   * | Favoris         | `/favoris`     | Requirement 11.4             |
   * | À regarder      | `/a-regarder`  | Requirement 11.5             |
   * | Paramètres      | `/settings`    | Requirement 2, 5.4, 16.14    |
   *
   * Le lien correspondant à la route courante est marqué `aria-current="page"`
   * et reçoit la classe `.active` (couleur d'accent Adwaita Dark) afin
   * que l'utilisateur identifie en permanence sa position dans
   * l'application (requirement 7.1).
   *
   * Polish UX :
   *   - Logo/titre en tête, transitions fluides sur le hover.
   *   - Indicateur visuel de la route active (barre verticale
   *     accent à gauche + fond accent atténué).
   *   - Bouton refresh avec icône SVG qui tourne pendant le scan
   *     (animation `spin`) au lieu d'un simple texte.
   *
   * _Requirements: 7.1, 11.4, 11.5_
   */
  import { page } from '$app/stores';
  import { startScan } from '$lib/api/commands';
  import { scanProgress } from '$lib/stores/library';
  import { toast } from '$lib/stores/toast';

  /** Définition d'une entrée de navigation. */
  interface NavLink {
    href: string;
    label: string;
  }

  /**
   * Liste statique et exhaustive des entrées de navigation. L'ordre
   * reflète la hiérarchie spécifiée : catégories principales
   * d'abord, listes thématiques ensuite, paramètres en bas.
   */
  const links: readonly NavLink[] = [
    { href: '/films', label: 'Films' },
    { href: '/series', label: 'Séries' },
    { href: '/animes', label: 'Animes' },
    { href: '/favoris', label: 'Favoris' },
    { href: '/a-regarder', label: 'À regarder' },
    { href: '/settings', label: 'Paramètres' }
  ];

  /**
   * Vrai si la route courante est égale au `href` du lien.
   *
   * Comparaison stricte plutôt que `startsWith` : `/settings`
   * et `/settings/logs` doivent être deux états distincts. Les
   * sous-vues détaillées (`/detail/:id`, `/edit/:id`) ne
   * « rallument » pas une entrée du Sidebar — elles vivent en
   * dehors de cette navigation principale.
   */
  function isActive(pathname: string, href: string): boolean {
    return pathname === href;
  }

  /**
   * `true` quand un scan vient juste d'être déclenché localement et
   * que `scanProgress` n'a pas encore été mis à jour par le premier
   * événement `scan-progress` du backend. Évite les double-clics.
   */
  let scanLaunching = false;

  /**
   * Vrai si un scan est en cours (déclenchement local OU progression
   * remontée par le backend). Désactive le bouton « Rafraîchir ».
   */
  $: scanInProgress = scanLaunching || $scanProgress !== null;

  async function handleRefresh(): Promise<void> {
    if (scanInProgress) return;
    scanLaunching = true;
    try {
      await startScan();
      toast.info('Scan démarré');
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      toast.error(`Échec du scan : ${message}`);
      console.warn('Kyro: échec start_scan', err);
    } finally {
      scanLaunching = false;
    }
  }
</script>

<nav class="sidebar" aria-label="Navigation principale">
  <div class="brand">
    <span class="brand-mark" aria-hidden="true">K</span>
    <span class="brand-name">Kyro</span>
  </div>

  <ul>
    {#each links as link (link.href)}
      {@const active = isActive($page.url.pathname, link.href)}
      <li>
        <a
          href={link.href}
          class:active
          aria-current={active ? 'page' : undefined}
        >
          <span class="link-indicator" aria-hidden="true"></span>
          <span class="link-label">{link.label}</span>
        </a>
      </li>
    {/each}
  </ul>

  <div class="sidebar-footer">
    <button
      type="button"
      class="refresh"
      class:scanning={scanInProgress}
      on:click={handleRefresh}
      disabled={scanInProgress}
      aria-busy={scanInProgress}
      data-testid="sidebar-refresh"
    >
      <svg
        class="refresh-icon"
        viewBox="0 0 24 24"
        width="16"
        height="16"
        aria-hidden="true"
      >
        <path
          d="M4 12a8 8 0 0 1 13.66-5.66M20 4v5h-5"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
        />
        <path
          d="M20 12a8 8 0 0 1-13.66 5.66M4 20v-5h5"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
        />
      </svg>
      <span>{scanInProgress ? 'Scan en cours…' : 'Rafraîchir'}</span>
    </button>
  </div>
</nav>

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
    width: 220px;
    height: 100vh;
    /* Sidebar strictement fixe : `box-sizing: border-box` (cf.
       theme.css) garantit que le padding rentre dans 100vh, donc
       jamais de scroll interne. `overflow: hidden` empêche tout
       débordement résiduel d'introduire une scrollbar parasite. */
    overflow: hidden;
    padding: var(--space-3) var(--space-3);
    background: var(--bg-secondary);
    border-right: 1px solid var(--border);
    color: var(--text-primary);
    font-family: var(--font-family);
  }

  /* En-tête de la sidebar : logo + nom de l'app. */
  .brand {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3) var(--space-4);
    margin-bottom: var(--space-2);
  }

  .brand-mark {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    background: var(--accent);
    color: #ffffff;
    border-radius: 8px;
    font-weight: 700;
    font-size: 0.95rem;
    letter-spacing: -0.02em;
  }

  .brand-name {
    font-size: 1.05rem;
    font-weight: 600;
    letter-spacing: -0.01em;
  }

  ul {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    list-style: none;
    padding: 0;
    margin: 0;
  }

  a {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    color: var(--text-secondary);
    text-decoration: none;
    border-radius: var(--radius-control);
    font-size: 0.95rem;
    line-height: 1.4;
    transition:
      background-color var(--duration-fast) var(--ease-standard),
      color var(--duration-fast) var(--ease-standard);
  }

  /*
   * Indicateur vertical à gauche de l'item actif. Apparaît avec un
   * léger scale-in transition.
   */
  .link-indicator {
    position: absolute;
    left: 0;
    top: 50%;
    width: 3px;
    height: 0;
    background: var(--accent);
    border-radius: 0 2px 2px 0;
    transform: translateY(-50%);
    transition: height var(--duration-base) var(--ease-emphasized);
  }

  a.active .link-indicator {
    height: 60%;
  }

  .link-label {
    flex: 1 1 auto;
  }

  a:hover {
    background: var(--surface);
    color: var(--text-primary);
  }

  a.active {
    background: rgba(53, 132, 228, 0.12);
    color: var(--text-primary);
    font-weight: 600;
  }

  a.active:hover {
    background: rgba(53, 132, 228, 0.18);
  }

  /* Bouton « Rafraîchir » dans le footer de la sidebar. */
  .sidebar-footer {
    margin-top: auto;
    /* Marge du bas réduite pour décoller le bouton du bord de la
       fenêtre — sans excès, on garde un peu de respiration au-dessus. */
    padding-top: var(--space-3);
    padding-bottom: var(--space-2);
  }

  .refresh {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    width: 100%;
    padding: var(--space-2) var(--space-3);
    background: var(--surface);
    color: var(--text-primary);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    font-family: var(--font-family);
    font-size: 0.9rem;
    cursor: pointer;
    transition:
      background-color var(--duration-fast) var(--ease-standard),
      border-color var(--duration-fast) var(--ease-standard),
      transform var(--duration-instant) var(--ease-standard);
  }

  .refresh:hover:not(:disabled) {
    background: var(--bg-secondary);
    border-color: var(--accent);
    color: var(--accent);
  }

  /* Feedback à l'appui : léger scale-down pour le ressenti tactile. */
  .refresh:active:not(:disabled) {
    transform: scale(0.98);
  }

  .refresh:disabled {
    opacity: 0.7;
    cursor: not-allowed;
  }

  .refresh-icon {
    flex-shrink: 0;
  }

  /* Spin pendant le scan : feedback visuel clair. */
  .refresh.scanning .refresh-icon {
    animation: kyro-spin 1.2s linear infinite;
    color: var(--accent);
  }

  @keyframes kyro-spin {
    from { transform: rotate(0deg); }
    to   { transform: rotate(360deg); }
  }
</style>
