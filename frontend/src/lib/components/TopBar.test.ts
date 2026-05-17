/**
 * @vitest-environment jsdom
 *
 * Tests unitaires de `<TopBar>`.
 *
 * Couverture :
 *  - la SearchBar imbriquée est rendue (requirement 13.1) ;
 *  - l'indicateur de scan apparaît / disparaît selon l'état de
 *    `scanProgress` et restitue les bons compteurs (requirement 3.6) ;
 *  - les badges `ffprobe-missing` (jaune, requirement 5.4) et
 *    `backend-degraded` (rouge, requirement 18.4) sont rendus
 *    uniquement quand le drapeau correspondant est `true`, et
 *    peuvent coexister.
 *
 * Les tests pilotent les stores partagés (`scanProgress`, `flags`)
 * directement, sans mock — c'est l'approche utilisée par les autres
 * tests de composants du projet (cf. `FilterPanel.test.ts`).
 *
 * _Requirements: 13.1, 3.6, 5.4, 18.4_
 */

import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, render } from '@testing-library/svelte';
import { tick } from 'svelte';

import TopBar from './TopBar.svelte';
import { flags, scanProgress } from '$lib/stores/library';

// Hygiène : les stores sont des singletons writable du module
// `$lib/stores/library`. On les remet à leurs valeurs initiales
// avant et après chaque test pour éviter toute fuite d'état.
beforeEach(() => {
  scanProgress.set(null);
  flags.set({ ffprobeMissing: false, backendDegraded: false });
});

afterEach(() => {
  cleanup();
  scanProgress.set(null);
  flags.set({ ffprobeMissing: false, backendDegraded: false });
});

describe('<TopBar> — SearchBar', () => {
  it('imbrique le composant SearchBar (champ de recherche accessible)', () => {
    const { getByRole } = render(TopBar);
    const input = getByRole('searchbox') as HTMLInputElement;
    expect(input).not.toBeNull();
    expect(input.getAttribute('aria-label')).toBe('Rechercher');
  });
});

describe('<TopBar> — indicateur de scan', () => {
  it("n'affiche pas l'indicateur quand scanProgress est null", () => {
    const { queryByTestId } = render(TopBar);
    expect(queryByTestId('scan-indicator')).toBeNull();
  });

  it('affiche l\'indicateur formaté quand scanProgress est défini', async () => {
    const { queryByTestId } = render(TopBar);
    expect(queryByTestId('scan-indicator')).toBeNull();

    scanProgress.set({
      processed: 12,
      total: 200,
      current_file: 'D:\\Films\\Inception.2010.1080p.mkv'
    });
    await tick();

    const indicator = queryByTestId('scan-indicator');
    expect(indicator).not.toBeNull();
    expect(indicator?.textContent?.trim()).toBe(
      'Scan: 12/200 - D:\\Films\\Inception.2010.1080p.mkv'
    );
  });

  it("retire l'indicateur quand scanProgress repasse à null", async () => {
    scanProgress.set({ processed: 1, total: 10, current_file: 'a.mkv' });

    const { queryByTestId } = render(TopBar);
    await tick();
    expect(queryByTestId('scan-indicator')).not.toBeNull();

    scanProgress.set(null);
    await tick();
    expect(queryByTestId('scan-indicator')).toBeNull();
  });
});

describe('<TopBar> — badges d\'état dégradé', () => {
  it("n'affiche aucun badge quand les drapeaux sont à false", () => {
    const { queryByTestId } = render(TopBar);
    expect(queryByTestId('badge-ffprobe-missing')).toBeNull();
    expect(queryByTestId('badge-backend-degraded')).toBeNull();
  });

  it('affiche le badge ffprobe-missing quand flags.ffprobeMissing est true', async () => {
    const { queryByTestId } = render(TopBar);
    flags.set({ ffprobeMissing: true, backendDegraded: false });
    await tick();

    const badge = queryByTestId('badge-ffprobe-missing');
    expect(badge).not.toBeNull();
    expect(badge?.textContent?.trim()).toBe('ffprobe-missing');
    // Toujours pas de badge backend-degraded.
    expect(queryByTestId('badge-backend-degraded')).toBeNull();
  });

  it('affiche le badge backend-degraded quand flags.backendDegraded est true', async () => {
    const { queryByTestId } = render(TopBar);
    flags.set({ ffprobeMissing: false, backendDegraded: true });
    await tick();

    const badge = queryByTestId('badge-backend-degraded');
    expect(badge).not.toBeNull();
    expect(badge?.textContent?.trim()).toBe('backend-degraded');
    expect(queryByTestId('badge-ffprobe-missing')).toBeNull();
  });

  it('peut afficher les deux badges simultanément', async () => {
    const { queryByTestId } = render(TopBar);
    flags.set({ ffprobeMissing: true, backendDegraded: true });
    await tick();

    expect(queryByTestId('badge-ffprobe-missing')).not.toBeNull();
    expect(queryByTestId('badge-backend-degraded')).not.toBeNull();
  });

  it('retire un badge quand son drapeau revient à false', async () => {
    flags.set({ ffprobeMissing: true, backendDegraded: false });

    const { queryByTestId } = render(TopBar);
    await tick();
    expect(queryByTestId('badge-ffprobe-missing')).not.toBeNull();

    flags.set({ ffprobeMissing: false, backendDegraded: false });
    await tick();
    expect(queryByTestId('badge-ffprobe-missing')).toBeNull();
  });
});
