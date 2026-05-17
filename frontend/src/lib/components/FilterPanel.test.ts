/**
 * @vitest-environment jsdom
 *
 * Tests unitaires de `<FilterPanel>`.
 *
 * Couverture :
 *   - Sémantique union sur les genres (sélection multiple).
 *   - Sémantique intersection / valeur unique pour année / type / statut.
 *   - Mise à jour réactive du compteur d'éléments visibles (req. 14.5).
 *   - Bouton « Réinitialiser » remet les quatre dimensions à []
 *     (req. 14.4).
 *
 * Les tests s'appuient sur les stores réels de `$lib/stores/library`
 * (pas de mock), ce qui correspond à la directive « tests sans mocks
 * tant que possible » du plan d'implémentation.
 *
 * _Requirements: 14.1, 14.2, 14.3, 14.4, 14.5_
 */

import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { get } from 'svelte/store';
import { tick } from 'svelte';

import FilterPanel from './FilterPanel.svelte';
import { filters, items, searchQuery } from '$lib/stores/library';
import type { MediaItem } from '$lib/api/types';

// ---------------------------------------------------------------------------
// Fixtures.
// ---------------------------------------------------------------------------

function fixture(): MediaItem[] {
  return [
    {
      id: 1,
      title: 'Inception',
      original_title: null,
      year: 2010,
      type: 'Film',
      status: 'vu',
      genres: ['Action', 'Science-Fiction']
    },
    {
      id: 2,
      title: 'Le Voyage de Chihiro',
      original_title: 'Sen to Chihiro no Kamikakushi',
      year: 2001,
      type: 'Anime',
      status: 'vu',
      genres: ['Animation', 'Aventure']
    },
    {
      id: 3,
      title: 'Breaking Bad',
      original_title: null,
      year: 2008,
      type: 'Series',
      status: 'en_cours',
      genres: ['Drame', 'Thriller']
    },
    {
      id: 4,
      title: 'Interstellar',
      original_title: null,
      year: 2014,
      type: 'Film',
      status: 'non_vu',
      genres: ['Science-Fiction', 'Drame']
    }
  ];
}

// ---------------------------------------------------------------------------
// Hygiène : reset des stores partagés entre tests (les modules de
// `$lib/stores/library` exportent des singletons writable).
// ---------------------------------------------------------------------------

beforeEach(() => {
  items.set(fixture());
  filters.set({ genres: [], years: [], types: [], statuses: [] });
  searchQuery.set('');
});

afterEach(() => {
  cleanup();
  items.set([]);
  filters.set({ genres: [], years: [], types: [], statuses: [] });
  searchQuery.set('');
});

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

describe('<FilterPanel> — compteur d\'éléments visibles', () => {
  it('affiche le total quand aucun filtre n\'est actif (req. 14.5)', () => {
    const { getByTestId } = render(FilterPanel);
    expect(getByTestId('visible-count').textContent?.trim()).toBe(
      '4 éléments visibles'
    );
  });

  it('utilise le singulier quand exactement un item est visible', async () => {
    const { getByTestId, getByRole } = render(FilterPanel);
    // Filtre par type Film + statut "vu" ⇒ seul Inception correspond.
    await fireEvent.click(getByRole('button', { name: 'Film' }));
    await fireEvent.click(getByRole('button', { name: 'Vu' }));
    expect(getByTestId('visible-count').textContent?.trim()).toBe(
      '1 élément visible'
    );
  });

  it('met à jour le compteur lors d\'une mutation du store `items`', async () => {
    const { getByTestId } = render(FilterPanel);
    items.set(fixture().slice(0, 2));
    await tick();
    expect(getByTestId('visible-count').textContent?.trim()).toBe(
      '2 éléments visibles'
    );
  });
});

describe('<FilterPanel> — genres (sélection multiple, union)', () => {
  it('un clic sur un chip de genre l\'ajoute au store filters.genres', async () => {
    const { getByRole } = render(FilterPanel);
    await fireEvent.click(getByRole('button', { name: 'Drame' }));
    expect(get(filters).genres).toEqual(['Drame']);
  });

  it('autorise plusieurs genres simultanés (union sur la dimension)', async () => {
    const { getByRole } = render(FilterPanel);
    await fireEvent.click(getByRole('button', { name: 'Drame' }));
    await fireEvent.click(getByRole('button', { name: 'Science-Fiction' }));
    const $f = get(filters);
    expect($f.genres).toContain('Drame');
    expect($f.genres).toContain('Science-Fiction');
    expect($f.genres).toHaveLength(2);
  });

  it('un second clic retire le genre (toggle)', async () => {
    const { getByRole } = render(FilterPanel);
    const chip = getByRole('button', { name: 'Drame' });
    await fireEvent.click(chip);
    await fireEvent.click(chip);
    expect(get(filters).genres).toEqual([]);
  });

  it('reflète l\'état selected via aria-pressed après mise à jour du store', async () => {
    const { getByRole } = render(FilterPanel);
    const chip = getByRole('button', { name: 'Drame' });
    expect(chip.getAttribute('aria-pressed')).toBe('false');
    await fireEvent.click(chip);
    expect(chip.getAttribute('aria-pressed')).toBe('true');
  });

  it('le compteur applique l\'union (Drame OR Animation)', async () => {
    const { getByRole, getByTestId } = render(FilterPanel);
    await fireEvent.click(getByRole('button', { name: 'Drame' }));
    await fireEvent.click(getByRole('button', { name: 'Animation' }));
    // Drame: Breaking Bad (3), Interstellar (4) ; Animation: Chihiro (2) ⇒ 3
    expect(getByTestId('visible-count').textContent?.trim()).toBe(
      '3 éléments visibles'
    );
  });
});

describe('<FilterPanel> — années / types / statuts (sélection unique)', () => {
  it('cliquer sur une année remplace la sélection précédente', async () => {
    const { getByRole } = render(FilterPanel);
    await fireEvent.click(getByRole('button', { name: '2010' }));
    expect(get(filters).years).toEqual([2010]);
    await fireEvent.click(getByRole('button', { name: '2014' }));
    expect(get(filters).years).toEqual([2014]);
  });

  it('cliquer une seconde fois sur la même année la désélectionne', async () => {
    const { getByRole } = render(FilterPanel);
    const chip = getByRole('button', { name: '2010' });
    await fireEvent.click(chip);
    await fireEvent.click(chip);
    expect(get(filters).years).toEqual([]);
  });

  it('cliquer sur un type remplace la sélection précédente', async () => {
    const { getByRole } = render(FilterPanel);
    await fireEvent.click(getByRole('button', { name: 'Film' }));
    expect(get(filters).types).toEqual(['Film']);
    await fireEvent.click(getByRole('button', { name: 'Anime' }));
    expect(get(filters).types).toEqual(['Anime']);
  });

  it('cliquer sur un statut remplace la sélection précédente', async () => {
    const { getByRole } = render(FilterPanel);
    await fireEvent.click(getByRole('button', { name: 'Vu' }));
    expect(get(filters).statuses).toEqual(['vu']);
    await fireEvent.click(getByRole('button', { name: 'En cours' }));
    expect(get(filters).statuses).toEqual(['en_cours']);
  });
});

describe('<FilterPanel> — intersection des dimensions (req. 14.2)', () => {
  it('combine genre (union) ET type (intersection)', async () => {
    const { getByRole, getByTestId } = render(FilterPanel);
    await fireEvent.click(getByRole('button', { name: 'Drame' }));
    await fireEvent.click(getByRole('button', { name: 'Film' }));
    // Drame ∩ Film ⇒ Interstellar uniquement.
    expect(getByTestId('visible-count').textContent?.trim()).toBe(
      '1 élément visible'
    );
  });
});

describe('<FilterPanel> — bouton Réinitialiser (req. 14.4)', () => {
  it('remet les quatre dimensions à [] et restaure le total', async () => {
    const { getByRole, getByText, getByTestId } = render(FilterPanel);
    await fireEvent.click(getByRole('button', { name: 'Drame' }));
    await fireEvent.click(getByRole('button', { name: 'Film' }));
    await fireEvent.click(getByRole('button', { name: '2010' }));
    await fireEvent.click(getByRole('button', { name: 'Vu' }));

    await fireEvent.click(getByText('Réinitialiser'));

    expect(get(filters)).toEqual({
      genres: [],
      years: [],
      types: [],
      statuses: []
    });
    expect(getByTestId('visible-count').textContent?.trim()).toBe(
      '4 éléments visibles'
    );
  });
});
