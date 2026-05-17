/**
 * @vitest-environment jsdom
 *
 * Tests unitaires du composant `HoverOverlay.svelte` (tâche 14.2).
 *
 * Vérifient le contrat visuel de l'overlay (requirement 15.4) :
 *  - rend la note utilisateur quand elle est définie ;
 *  - rend la durée totale formatée pour les Films ;
 *  - rend le nombre d'épisodes pour les Séries / Animes ;
 *  - rend toujours le statut courant ;
 *  - omet les lignes optionnelles quand la donnée correspondante
 *    est absente ;
 *  - applique la classe `.visible` quand la prop `isHovering`
 *    est `true`.
 *
 * _Requirements: 15.4_
 */

import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render } from '@testing-library/svelte';

afterEach(() => {
  cleanup();
});

import HoverOverlay from './HoverOverlay.svelte';
import type { MediaItem } from '$lib/api/types';

type HoverOverlayItem = MediaItem & {
  user_rating?: number | null;
  duration_seconds?: number | null;
  episode_count?: number | null;
};

const filmBase: HoverOverlayItem = {
  id: 1,
  title: 'Blade Runner',
  original_title: null,
  year: 1982,
  type: 'Film',
  status: 'non_vu',
  genres: [],
  user_rating: null,
  duration_seconds: null,
  episode_count: null
};

const seriesBase: HoverOverlayItem = {
  id: 2,
  title: 'Breaking Bad',
  original_title: null,
  year: 2008,
  type: 'Series',
  status: 'en_cours',
  genres: [],
  user_rating: null,
  duration_seconds: null,
  episode_count: null
};

describe('HoverOverlay — note utilisateur', () => {
  it('rend la note quand `user_rating` est défini', () => {
    const { getByTestId } = render(HoverOverlay, {
      props: { item: { ...filmBase, user_rating: 8 } }
    });
    expect(getByTestId('hover-rating').textContent).toContain('8 / 10');
  });

  it('omet la ligne note quand `user_rating` est null', () => {
    const { queryByTestId } = render(HoverOverlay, {
      props: { item: { ...filmBase, user_rating: null } }
    });
    expect(queryByTestId('hover-rating')).toBeNull();
  });
});

describe('HoverOverlay — durée des Films', () => {
  it('rend la durée formatée Xh MM pour un Film', () => {
    const { getByTestId } = render(HoverOverlay, {
      props: { item: { ...filmBase, duration_seconds: 7320 } } // 2 h 02
    });
    expect(getByTestId('hover-duration').textContent).toContain('2 h 02');
  });

  it('rend uniquement les minutes pour une durée < 1 h', () => {
    const { getByTestId } = render(HoverOverlay, {
      props: { item: { ...filmBase, duration_seconds: 1800 } } // 30 min
    });
    expect(getByTestId('hover-duration').textContent).toContain('30 min');
  });

  it('omet la ligne durée pour un Film sans durée', () => {
    const { queryByTestId } = render(HoverOverlay, {
      props: { item: { ...filmBase, duration_seconds: null } }
    });
    expect(queryByTestId('hover-duration')).toBeNull();
  });

  it("n'affiche pas la durée pour une Série / un Anime, même si fournie", () => {
    const { queryByTestId } = render(HoverOverlay, {
      props: { item: { ...seriesBase, duration_seconds: 3600 } }
    });
    expect(queryByTestId('hover-duration')).toBeNull();
  });
});

describe('HoverOverlay — nombre d\'épisodes (Séries / Animes)', () => {
  it('rend le nombre d\'épisodes pour une Série', () => {
    const { getByTestId } = render(HoverOverlay, {
      props: { item: { ...seriesBase, episode_count: 62 } }
    });
    expect(getByTestId('hover-episodes').textContent).toContain('62 épisodes');
  });

  it('utilise le singulier pour exactement un épisode', () => {
    const { getByTestId } = render(HoverOverlay, {
      props: { item: { ...seriesBase, episode_count: 1 } }
    });
    const txt = getByTestId('hover-episodes').textContent ?? '';
    expect(txt).toContain('1 épisode');
    expect(txt).not.toContain('1 épisodes');
  });

  it('omet la ligne épisodes pour une Série sans compteur', () => {
    const { queryByTestId } = render(HoverOverlay, {
      props: { item: { ...seriesBase, episode_count: null } }
    });
    expect(queryByTestId('hover-episodes')).toBeNull();
  });

  it("n'affiche pas le compteur d'épisodes pour un Film, même si fourni", () => {
    const { queryByTestId } = render(HoverOverlay, {
      props: { item: { ...filmBase, episode_count: 10 } }
    });
    expect(queryByTestId('hover-episodes')).toBeNull();
  });
});

describe('HoverOverlay — statut courant', () => {
  it('rend toujours le statut courant (Film, non_vu)', () => {
    const { getByTestId } = render(HoverOverlay, {
      props: { item: { ...filmBase, status: 'non_vu' } }
    });
    expect(getByTestId('hover-status').textContent).toContain('Non vu');
  });

  it('rend "En cours" pour status=en_cours', () => {
    const { getByTestId } = render(HoverOverlay, {
      props: { item: { ...seriesBase, status: 'en_cours' } }
    });
    expect(getByTestId('hover-status').textContent).toContain('En cours');
  });

  it('rend "Vu" pour status=vu', () => {
    const { getByTestId } = render(HoverOverlay, {
      props: { item: { ...filmBase, status: 'vu' } }
    });
    expect(getByTestId('hover-status').textContent).toContain('Vu');
  });
});

describe('HoverOverlay — visibilité contrôlée par `isHovering`', () => {
  it('applique la classe `.visible` quand isHovering=true', () => {
    const { getByTestId } = render(HoverOverlay, {
      props: { item: filmBase, isHovering: true }
    });
    expect(getByTestId('hover-overlay').classList.contains('visible')).toBe(true);
  });

  it("n'applique pas la classe `.visible` quand isHovering=false (défaut)", () => {
    const { getByTestId } = render(HoverOverlay, {
      props: { item: filmBase }
    });
    expect(getByTestId('hover-overlay').classList.contains('visible')).toBe(false);
  });

  it('reflète aria-hidden sur la valeur de isHovering', () => {
    const { getByTestId } = render(HoverOverlay, {
      props: { item: filmBase, isHovering: true }
    });
    expect(getByTestId('hover-overlay').getAttribute('aria-hidden')).toBe('false');
  });
});
