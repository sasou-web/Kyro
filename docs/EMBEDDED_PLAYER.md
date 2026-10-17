# Player embarqué Kyro (libmpv)

Cette note décrit l'architecture du **player embarqué** : un moteur
libmpv chargé dynamiquement par Kyro qui rend la vidéo dans une
fenêtre dédiée de l'application, avec une UI de contrôle Svelte
fournie par Kyro lui-même.

Le **mode externe historique** (mpv.net + IPC NDJSON via named pipe +
`kyro.lua`) reste pleinement supporté et continue d'être l'option
choisie par l'utilisateur via le réglage `playback_mode = "external"`.
Aucune modification du flux externe n'est imposée par cette
fonctionnalité.

## Objectif

Permettre à l'utilisateur de lire ses fichiers depuis Kyro sans avoir à
installer mpv.net, sans configurer de chemin externe, et sans qu'aucune
configuration mpv.net globale (`%APPDATA%\mpv.net\`) ne soit lue ni
modifiée.

## Choix technique

Le moteur retenu est **libmpv** (la bibliothèque C derrière mpv et
mpv.net). Trois variantes étaient envisageables ; libmpv embarqué dans
une fenêtre Tauri dédiée est le seul à cocher toutes les contraintes :

| Variante                                | Conflit config mpv.net | Disponible hors Kyro | UI 100% Kyro |
| --------------------------------------- | ---------------------- | -------------------- | ------------ |
| Garder mpv.net externe (statu quo)      | Possible               | Oui                  | Non          |
| Embarquer libmpv dans une fenêtre Kyro  | **Aucun**              | **Non**              | **Oui**      |
| Décoder via WebView2 (`<video>` HTML5)  | Aucun                  | Non                  | Oui          |

WebView2 pur a été écarté : qualité de rendu inférieure à mpv,
sous-titres ASS non équivalents, formats vidéo limités. Reproduire la
parité avec mpv via FFmpeg + canvas serait un projet plus lourd que
d'embarquer libmpv directement.

## Garanties d'isolation

libmpv est instancié par Kyro avec un environnement strictement isolé :

* **Pas de lecture de `%APPDATA%\mpv.net\`** : Kyro positionne
  `config-dir = %APPDATA%\Kyro\mpv-config` avant `mpv_initialize`.
* **Pas de raccourcis clavier mpv** : `input-default-bindings = no`,
  `input-vo-keyboard = no`. Toutes les interactions passent par l'UI
  Svelte de Kyro.
* **Pas d'OSC mpv** : `osc = no`. Les contrôles sont rendus par Kyro.
* **Pas d'effet sur l'instance mpv.net externe** : libmpv tourne
  in-process, ne touche aucun named pipe global, ne s'enregistre
  comme association de fichier nulle part.

Le mode externe (mpv.net + `kyro.lua`) reste branché sur ses propres
chemins et son propre canal IPC. Les deux modes peuvent coexister
sans interaction.

## Chargement dynamique de `libmpv-2.dll`

La DLL libmpv est chargée à l'ouverture du player via `libloading`,
**pas** linkée à la compilation. Conséquences :

* Le crate Rust se compile sans header ni .lib mpv (toolchain plus
  légère côté CI / dev).
* Si `libmpv-2.dll` est absente au runtime, le mode embarqué retourne
  une erreur explicite et Kyro **ne plante pas** : l'utilisateur peut
  basculer sur `external` depuis Paramètres et continuer à lire ses
  fichiers via mpv.net.
* La résolution du chemin de la DLL suit la même priorité que le
  locator ffprobe :

  1. À côté de `kyro.exe` (déposé par l'installeur — production).
  2. Dans le `target/<profile>/` du build Cargo (dev).
  3. Sur le `PATH` système.

## Surface API libmpv consommée

Le module `infra::embedded_player` consomme un sous-ensemble minimal
de l'API libmpv :

* `mpv_create`, `mpv_initialize`, `mpv_terminate_destroy` — cycle de
  vie du contexte.
* `mpv_set_option_string` — préconfiguration (`config-dir`,
  `input-default-bindings = no`, `osc = no`, etc.) **avant**
  `mpv_initialize`.
* `mpv_set_property_string` — `wid` (HWND embed), `pause`, `aid`,
  `sid` après `mpv_initialize`.
* `mpv_command_async` — `loadfile <path>`, `seek <secs> absolute`,
  `quit`.
* `mpv_observe_property` + `mpv_wait_event` — pump des propriétés
  observées (`time-pos`, `duration`, `pause`, `eof-reached`,
  `track-list`) sur un thread Rust dédié.

Ce sous-ensemble est suffisant pour reproduire ce que `kyro.lua` fait
côté mpv.net (progress, completed, file-changed) et l'étendre avec
les contrôles que l'UI Kyro veut exposer (pause, seek, sélection de
pistes).

## Découpe du code

```
src-tauri/src/infra/embedded_player/
├── mod.rs         # API publique : EmbeddedPlayer, EmbeddedPlayerEvent
├── ffi.rs         # bindings libloading + signatures C minimales
├── config.rs      # build des options mpv (isolation)
└── event_loop.rs  # thread pump mpv_wait_event → mpsc::Receiver
```

```
src-tauri/src/commands/embedded_player.rs
# commandes Tauri : embedded_player_open, _pause, _seek, _set_track, _quit
```

## Réglage `playback_mode`

Nouvelle clé `setting_value` :

* `"embedded"` (défaut au premier démarrage) — utilise libmpv embarqué.
* `"external"` — utilise le `external_player_path` configuré (mpv.net
  ou autre). Branche existante intacte.

La commande `play_media` lit ce réglage en début d'exécution et route :

```text
playback_mode = "embedded"
    ├─ libmpv DLL résolue        → ouvre fenêtre player + lance libmpv
    └─ libmpv DLL introuvable    → KyroError::Validation (UI affiche
                                    « libmpv-2.dll introuvable, basculez
                                    en mode externe dans les Paramètres »)

playback_mode = "external"
    └─ flux actuel inchangé : launch_player + ipc_pipe + run_ipc_session_emit
```

## Fenêtre player

Une seconde fenêtre Tauri `player` est créée dynamiquement à chaque
appel `play_media` (mode embedded). Elle est détruite à la fin de la
lecture (event `playback-ended` ou `embedded_player_quit`).

* Label : `player`.
* Décorations système conservées (titre Kyro, redimensionnement,
  fermeture). Plein écran disponible via touche `f` câblée par l'UI.
* WebView2 transparente : la zone vidéo affiche le child HWND libmpv
  rendu en arrière-plan, les contrôles Svelte sont superposés via
  pointer-events ciblés.
* Une seule instance à la fois : ouvrir une nouvelle lecture ferme
  proprement la précédente (équivalent du `mpv_command(quit)` côté
  externe).

## Persistance de la progression

Le module `embedded_player` réutilise exactement le même `Repo` et la
même fonction `set_watch_progress` que la branche externe. Les events
émis par `mpv_observe_property time-pos` sont consommés par une task
Tokio qui appelle `Repo::set_watch_progress(target, position)` au fil
de l'eau, comme `run_ipc_session_emit` le fait pour la branche
externe.

À la fin de la lecture (eof-reached = true) la position finale est
forcée à la durée pour basculer le statut à `Vu`, en cohérence avec
le comportement `Completed` de la session NDJSON.

## Bundle de la DLL

`libmpv-2.dll` (~30 MB) est déposée dans `src-tauri/binaries/` au
même titre que `ffprobe-x86_64-pc-windows-msvc.exe`. Elle est exposée
via `bundle.resources` (et non `externalBin`, qui est réservé aux
exécutables avec triple cible). L'installeur la copie à côté de
`kyro.exe`.

Le poids supplémentaire ~30 MB dépasse mécaniquement la limite
historique Requirement 1.3 « ≤ 50 Mo hors ffprobe ». Cette limite
est étendue au cas par cas dans `SIDECAR-NOTES.md` :
**« hors ffprobe et hors libmpv-2.dll si playback_mode embedded
disponible »**.

L'utilisateur peut supprimer la DLL après installation pour récupérer
la place : Kyro détectera son absence et continuera à fonctionner en
mode `external`.
