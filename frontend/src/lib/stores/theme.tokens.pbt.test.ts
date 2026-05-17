/**
 * @vitest-environment jsdom
 *
 * Test de propriété frontend — Property 34 : Tokens du thème Adwaita Dark.
 *
 * On injecte dynamiquement le contenu réel de `frontend/src/styles/theme.css`
 * dans le document jsdom, puis on applique `data-theme="adwaita-dark"` sur
 * `<html>`. Pour chaque token déclaré par le Requirement 16, on vérifie
 * via `getComputedStyle(document.documentElement).getPropertyValue(...)` que
 * la valeur calculée correspond exactement à celle attendue dans la
 * spécification. On vérifie en plus que la teinte HSL de `--accent` reste
 * dans la gamme bleue [200°, 240°] (Requirement 16.13 — exclusion des
 * accents violet/néon).
 *
 * **Validates: Requirements 16.2, 16.3, 16.4, 16.5, 16.6, 16.7, 16.8, 16.9,
 * 16.10, 16.11, 16.12, 16.13**
 */

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

import { beforeAll, afterAll, describe, expect, it } from 'vitest';
import fc from 'fast-check';

// ---------------------------------------------------------------------------
// Tokens attendus — tirés directement du Requirement 16 (acceptance criteria
// 16.2 à 16.12). Ces couples (nom, valeur attendue) constituent l'espace
// d'échantillonnage de la propriété : la propriété doit être universellement
// vraie pour chacun d'eux.
// ---------------------------------------------------------------------------

type ExpectedToken = readonly [name: string, expected: string];

/**
 * Valeur du token `--font-family` telle que déclarée dans `theme.css`.
 * Le requirement 16.12 exige une famille sans empattement de type Inter
 * avec repli système ; on vérifie l'égalité exacte avec la déclaration CSS.
 */
const EXPECTED_FONT_FAMILY =
  '"Inter", -apple-system, "Segoe UI", system-ui, sans-serif';

const EXPECTED_TOKENS: readonly ExpectedToken[] = [
  ['--bg-primary', '#1e1e2e'],        // 16.2
  ['--bg-secondary', '#242535'],      // 16.3
  ['--surface', '#2a2b3d'],           // 16.4
  ['--border', '#383a4f'],            // 16.5
  ['--text-primary', '#e6e6ec'],      // 16.6
  ['--text-secondary', '#a8a8b5'],    // 16.7
  ['--accent', '#3584e4'],            // 16.8
  ['--accent-hover', '#1c71d8'],      // 16.9
  ['--radius-card', '12px'],          // 16.10
  ['--radius-control', '8px'],        // 16.11
  ['--font-family', EXPECTED_FONT_FAMILY] // 16.12
] as const;

// ---------------------------------------------------------------------------
// Helpers — conversion hex → HSL pour le contrôle de la teinte de --accent.
// Implémentation classique (cf. https://en.wikipedia.org/wiki/HSL_and_HSV).
// ---------------------------------------------------------------------------

interface HSL {
  readonly h: number; // teinte en degrés [0, 360)
  readonly s: number; // saturation [0, 1]
  readonly l: number; // luminance [0, 1]
}

function parseHexColor(input: string): { r: number; g: number; b: number } {
  const hex = input.trim().replace(/^#/, '');
  if (hex.length !== 6) {
    throw new Error(`Couleur hex inattendue: ${input}`);
  }
  const r = Number.parseInt(hex.slice(0, 2), 16);
  const g = Number.parseInt(hex.slice(2, 4), 16);
  const b = Number.parseInt(hex.slice(4, 6), 16);
  return { r, g, b };
}

function rgbToHsl(r: number, g: number, b: number): HSL {
  const rn = r / 255;
  const gn = g / 255;
  const bn = b / 255;
  const max = Math.max(rn, gn, bn);
  const min = Math.min(rn, gn, bn);
  const l = (max + min) / 2;
  const d = max - min;
  if (d === 0) {
    return { h: 0, s: 0, l };
  }
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
  let h: number;
  if (max === rn) {
    h = ((gn - bn) / d) % 6;
  } else if (max === gn) {
    h = (bn - rn) / d + 2;
  } else {
    h = (rn - gn) / d + 4;
  }
  h *= 60;
  if (h < 0) h += 360;
  return { h, s, l };
}

// ---------------------------------------------------------------------------
// Setup global — on injecte le CSS et l'attribut data-theme une fois pour
// l'ensemble du fichier. Toutes les assertions partagent le même DOM.
// ---------------------------------------------------------------------------

let styleEl: HTMLStyleElement | null = null;

beforeAll(() => {
  // Résolution robuste du chemin vers `theme.css` quel que soit le cwd
  // d'exécution de Vitest (workspace racine ou `frontend/`).
  const here = dirname(fileURLToPath(import.meta.url));
  const themeCssPath = resolve(here, '../../styles/theme.css');
  const css = readFileSync(themeCssPath, 'utf8');

  styleEl = document.createElement('style');
  styleEl.setAttribute('data-test-theme', 'true');
  styleEl.textContent = css;
  document.head.appendChild(styleEl);

  document.documentElement.setAttribute('data-theme', 'adwaita-dark');
});

afterAll(() => {
  if (styleEl && styleEl.parentNode) {
    styleEl.parentNode.removeChild(styleEl);
  }
  document.documentElement.removeAttribute('data-theme');
});

// ---------------------------------------------------------------------------
// Property 34 — chaque token déclaré par Requirement 16 a la valeur exacte
// attendue lorsque `data-theme="adwaita-dark"` est appliqué.
// ---------------------------------------------------------------------------

describe('Property 34 — Tokens du thème Adwaita Dark', () => {
  it('Property 34 : tous les tokens du Requirement 16.2–16.12 ont la valeur exacte attendue', () => {
    fc.assert(
      fc.property(fc.constantFrom(...EXPECTED_TOKENS), ([name, expected]) => {
        const computed = getComputedStyle(document.documentElement)
          .getPropertyValue(name)
          .trim();
        expect(computed).toBe(expected);
      }),
      { numRuns: EXPECTED_TOKENS.length * 4 }
    );
  });

  it('Property 34 : la teinte HSL de --accent reste dans la gamme bleue [200°, 240°] (Requirement 16.13)', () => {
    const accent = getComputedStyle(document.documentElement)
      .getPropertyValue('--accent')
      .trim();
    expect(accent).toBe('#3584e4');

    const { r, g, b } = parseHexColor(accent);
    const { h } = rgbToHsl(r, g, b);
    // On contrôle la teinte uniquement : s/l varient peu et ne sont pas
    // contraintes par 16.13. La gamme bleue Adwaita Blue 3 (#3584e4)
    // tombe à ≈ 213°, bien à l'intérieur de [200°, 240°].
    expect(h).toBeGreaterThanOrEqual(200);
    expect(h).toBeLessThanOrEqual(240);
  });
});
