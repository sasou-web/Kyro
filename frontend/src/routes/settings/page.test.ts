/**
 * @vitest-environment jsdom
 *
 * Tests unitaires de la page Paramètres `/settings/+page.svelte`
 * (tâche 15.7).
 *
 * Stratégie :
 *   - on mocke `@tauri-apps/plugin-dialog` (pas de dialogue natif) ;
 *   - on mocke `$lib/api/commands` (toutes les commandes IPC sont
 *     remplacées par des espions résolvant des valeurs synthétiques) ;
 *   - on précharge le store `settings` avec un `SettingsSummary`
 *     déterministe pour ne pas dépendre de l'ordre `getSettingsSummary`
 *     ↔ rendu initial.
 *
 * Couvre les requirements 2.1, 2.2, 2.3, 2.4, 2.5, 3.7, 5.4, 9.7,
 * 10.1, 10.3, 10.4 et 16.14 — chaque section de la page a au moins
 * un cas qui valide un point de contrat.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';

vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: vi.fn(),
  save: vi.fn()
}));

vi.mock('$lib/api/commands', () => ({
  addLibraryFolder: vi.fn(),
  removeLibraryFolder: vi.fn(),
  listLibraryFolders: vi.fn(),
  setExternalPlayerPath: vi.fn(),
  setFfprobePath: vi.fn(),
  setScanAtStartup: vi.fn(),
  installLuaScript: vi.fn(),
  exportLibraryBackup: vi.fn(),
  restoreLibraryBackup: vi.fn(),
  getAppVersion: vi.fn(async () => ({
    version: '1.0.0',
    releases_url: 'https://example.test/releases'
  })),
  openReleasesPage: vi.fn(async () => undefined),
  getSettingsSummary: vi.fn(),
  getTheme: vi.fn(async () => 'adwaita-dark'),
  setTheme: vi.fn(async () => undefined)
}));

import { open } from '@tauri-apps/plugin-dialog';
import {
  addLibraryFolder,
  getSettingsSummary,
  installLuaScript,
  removeLibraryFolder,
  setExternalPlayerPath,
  setFfprobePath,
  setScanAtStartup
} from '$lib/api/commands';

import SettingsPage from './+page.svelte';
import { flags, settings } from '$lib/stores/library';
import type { SettingsSummary } from '$lib/api/types';

/** SettingsSummary minimal réutilisé par les tests. */
function summary(overrides: Partial<SettingsSummary> = {}): SettingsSummary {
  return {
    library_folders: [
      { id: 1, path: 'D:/Films', category: 'Films' },
      { id: 2, path: 'D:/Series', category: 'Series' }
    ],
    external_player_path: 'C:/Program Files/mpv.net/mpv.net.exe',
    ffprobe_path: null,
    scan_at_startup: false,
    theme: 'adwaita-dark',
    last_category: null,
    playback_mode: 'embedded',
    ...overrides
  };
}

beforeEach(() => {
  vi.mocked(open).mockReset();
  vi.mocked(addLibraryFolder).mockReset();
  vi.mocked(removeLibraryFolder).mockReset();
  vi.mocked(setExternalPlayerPath).mockReset();
  vi.mocked(setFfprobePath).mockReset();
  vi.mocked(setScanAtStartup).mockReset();
  vi.mocked(installLuaScript).mockReset();
  vi.mocked(getSettingsSummary).mockReset();

  // Valeurs par défaut « heureuses » pour la majorité des tests :
  // chaque mutation refresh le store.
  vi.mocked(getSettingsSummary).mockResolvedValue(summary());

  // Pré-charge le store pour que la page rende immédiatement la
  // liste des dossiers / les drafts pré-remplis.
  settings.set(summary());
  flags.set({ ffprobeMissing: false, backendDegraded: false });
});

afterEach(() => {
  cleanup();
  settings.set(null);
  flags.set({ ffprobeMissing: false, backendDegraded: false });
});

describe('/settings — section Dossiers de bibliothèque', () => {
  it('liste les dossiers configurés avec leur catégorie (req. 2.1, 2.5)', async () => {
    const { getByText, getByTestId } = render(SettingsPage);
    await tick();

    // Les chemins sont uniques dans le DOM ⇒ getByText.
    expect(getByText('D:/Films')).toBeTruthy();
    expect(getByText('D:/Series')).toBeTruthy();

    // Les libellés de catégorie apparaissent à la fois dans la liste
    // et dans le `<select>` du formulaire d'ajout : on cherche
    // explicitement à l'intérieur de la section Dossiers.
    const section = getByTestId('section-folders');
    const categories = section.querySelectorAll('.folder-category');
    const labels = Array.from(categories).map((c) => c.textContent?.trim());
    expect(labels).toEqual(['Films', 'Series']);
  });

  it('appelle addLibraryFolder avec le chemin et la catégorie sélectionnés (req. 2.2, 2.5)', async () => {
    vi.mocked(addLibraryFolder).mockResolvedValue(7);

    const { getByLabelText, getByRole } = render(SettingsPage);
    await tick();

    const pathInput = getByLabelText('Chemin du dossier') as HTMLInputElement;
    await fireEvent.input(pathInput, { target: { value: 'D:/Animes' } });

    const categorySelect = getByLabelText('Catégorie') as HTMLSelectElement;
    await fireEvent.change(categorySelect, { target: { value: 'Animes' } });

    const submitBtn = getByRole('button', { name: 'Ajouter' });
    await fireEvent.click(submitBtn);

    await waitFor(() => {
      expect(addLibraryFolder).toHaveBeenCalledTimes(1);
    });
    expect(addLibraryFolder).toHaveBeenCalledWith('D:/Animes', 'Animes');
  });

  it('affiche le message d\'erreur du backend quand le chemin est inexistant (req. 2.4)', async () => {
    vi.mocked(addLibraryFolder).mockRejectedValue(
      'Le chemin "D:/Inexistant" n\'existe pas sur le système de fichiers'
    );

    const { getByLabelText, getByRole, findByTestId } = render(SettingsPage);
    await tick();

    await fireEvent.input(getByLabelText('Chemin du dossier'), {
      target: { value: 'D:/Inexistant' }
    });
    await fireEvent.click(getByRole('button', { name: 'Ajouter' }));

    const error = await findByTestId('folders-error');
    expect(error.textContent).toContain('D:/Inexistant');
    expect(error.textContent).toContain("n'existe pas");
  });

  it("appelle removeLibraryFolder quand l'utilisateur clique sur ✕ (req. 2.3)", async () => {
    vi.mocked(removeLibraryFolder).mockResolvedValue(undefined);

    const { getAllByLabelText } = render(SettingsPage);
    await tick();

    const removeBtns = getAllByLabelText('Retirer ce dossier');
    expect(removeBtns).toHaveLength(2);

    await fireEvent.click(removeBtns[0]);
    await waitFor(() => {
      expect(removeLibraryFolder).toHaveBeenCalledTimes(1);
    });
    expect(removeLibraryFolder).toHaveBeenCalledWith(1);
  });

  it("ouvre le dialogue natif quand l'utilisateur clique sur Parcourir…", async () => {
    vi.mocked(open).mockResolvedValue('D:/NouveauDossier');

    const { getByLabelText, getAllByRole } = render(SettingsPage);
    await tick();

    // Premier bouton « Parcourir… » = celui de la section dossiers.
    const browseBtns = getAllByRole('button', { name: 'Parcourir…' });
    await fireEvent.click(browseBtns[0]);

    await waitFor(() => {
      expect(open).toHaveBeenCalledTimes(1);
    });
    const args = vi.mocked(open).mock.calls[0][0] as {
      directory: boolean;
      multiple: boolean;
    };
    expect(args.directory).toBe(true);
    expect(args.multiple).toBe(false);

    // Le chemin retourné est injecté dans le champ.
    await waitFor(() => {
      const input = getByLabelText('Chemin du dossier') as HTMLInputElement;
      expect(input.value).toBe('D:/NouveauDossier');
    });
  });
});

describe('/settings — section Lecteur externe', () => {
  it("pré-remplit l'input avec external_player_path du store (req. 10.1)", async () => {
    const { getByLabelText } = render(SettingsPage);
    await tick();

    const input = getByLabelText('Chemin du lecteur externe') as HTMLInputElement;
    expect(input.value).toBe('C:/Program Files/mpv.net/mpv.net.exe');
  });

  it("le bouton Vérifier appelle setExternalPlayerPath et affiche succès (req. 10.3)", async () => {
    vi.mocked(setExternalPlayerPath).mockResolvedValue(undefined);

    const { getByTestId } = render(SettingsPage);
    await tick();

    await fireEvent.click(getByTestId('verify-external-player'));

    await waitFor(() => {
      expect(setExternalPlayerPath).toHaveBeenCalledTimes(1);
    });
    expect(setExternalPlayerPath).toHaveBeenCalledWith(
      'C:/Program Files/mpv.net/mpv.net.exe'
    );

    await waitFor(() => {
      const status = getByTestId('external-player-status');
      expect(status.textContent).toContain('Lecteur trouvé');
    });
  });

  it('affiche le message d\'erreur quand le backend rejette (req. 10.4)', async () => {
    vi.mocked(setExternalPlayerPath).mockRejectedValue(
      'Le chemin du lecteur externe doit pointer vers un fichier .exe existant'
    );

    const { getByTestId } = render(SettingsPage);
    await tick();

    await fireEvent.click(getByTestId('verify-external-player'));

    await waitFor(() => {
      const status = getByTestId('external-player-status');
      expect(status.textContent).toContain('.exe existant');
    });
  });
});

describe('/settings — section ffprobe', () => {
  it("affiche un avertissement quand flags.ffprobeMissing est vrai (req. 5.4)", async () => {
    flags.set({ ffprobeMissing: true, backendDegraded: false });
    const { getByTestId } = render(SettingsPage);
    await tick();

    const indicator = getByTestId('ffprobe-indicator');
    expect(indicator.textContent).toMatch(/introuvable/i);
  });

  it('appelle setFfprobePath puis retire le drapeau ffprobeMissing à la persistence (req. 5.4)', async () => {
    flags.set({ ffprobeMissing: true, backendDegraded: false });
    vi.mocked(setFfprobePath).mockResolvedValue(undefined);

    const { getByLabelText, getAllByRole } = render(SettingsPage);
    await tick();

    const input = getByLabelText('Chemin de ffprobe') as HTMLInputElement;
    await fireEvent.input(input, {
      target: { value: 'C:/ffmpeg/bin/ffprobe.exe' }
    });

    // Section ffprobe : 4e bouton « Enregistrer » est celui de
    // ffprobe (folders n'a pas de bouton Enregistrer ; player a un
    // Enregistrer) — on cible via le 2e Enregistrer rendu.
    const saveBtns = getAllByRole('button', { name: 'Enregistrer' });
    // saveBtns[0] = lecteur externe ; saveBtns[1] = ffprobe.
    await fireEvent.click(saveBtns[1]);

    await waitFor(() => {
      expect(setFfprobePath).toHaveBeenCalledTimes(1);
    });
    expect(setFfprobePath).toHaveBeenCalledWith('C:/ffmpeg/bin/ffprobe.exe');

    // Le drapeau a été remis à `false`.
    await waitFor(() => {
      let current = { ffprobeMissing: true, backendDegraded: false };
      const unsub = flags.subscribe((v) => {
        current = v;
      });
      unsub();
      expect(current.ffprobeMissing).toBe(false);
    });
  });
});

describe('/settings — section Scan au démarrage (req. 3.7)', () => {
  it('appelle setScanAtStartup quand le toggle change', async () => {
    vi.mocked(setScanAtStartup).mockResolvedValue(undefined);

    const { getByTestId } = render(SettingsPage);
    await tick();

    const toggle = getByTestId('toggle-scan-at-startup') as HTMLInputElement;
    expect(toggle.checked).toBe(false);

    await fireEvent.change(toggle, { target: { checked: true } });

    await waitFor(() => {
      expect(setScanAtStartup).toHaveBeenCalledTimes(1);
    });
    expect(setScanAtStartup).toHaveBeenCalledWith(true);
  });
});

describe('/settings — section Thème (req. 16.14)', () => {
  it('le bouton Basculer appelle toggleTheme via le store', async () => {
    const { getByTestId } = render(SettingsPage);
    await tick();

    // État initial : adwaita-dark.
    expect(getByTestId('theme-current').textContent).toContain('Adwaita Dark');

    await fireEvent.click(getByTestId('theme-toggle'));

    // Après toggle : adwaita-light.
    await waitFor(() => {
      expect(getByTestId('theme-current').textContent).toContain('Adwaita Light');
    });
  });
});

describe('/settings — section Installer le script Lua (req. 9.7)', () => {
  it('appelle installLuaScript et affiche le chemin retourné', async () => {
    vi.mocked(installLuaScript).mockResolvedValue(
      'C:/Users/test/AppData/Roaming/mpv.net/scripts/kyro.lua'
    );

    const { getByTestId } = render(SettingsPage);
    await tick();

    await fireEvent.click(getByTestId('install-lua-script'));

    await waitFor(() => {
      expect(installLuaScript).toHaveBeenCalledTimes(1);
    });

    await waitFor(() => {
      const status = getByTestId('lua-status');
      expect(status.textContent).toContain('kyro.lua');
    });
  });

  it('affiche un message d\'erreur quand l\'installation échoue', async () => {
    vi.mocked(installLuaScript).mockRejectedValue(
      'Impossible de créer le dossier scripts'
    );

    const { getByTestId } = render(SettingsPage);
    await tick();

    await fireEvent.click(getByTestId('install-lua-script'));

    await waitFor(() => {
      const status = getByTestId('lua-status');
      expect(status.textContent).toContain('Impossible de créer');
    });
  });
});
