# `src-tauri/scripts/`

Scripts auxiliaires utilisés par le pipeline de packaging Windows de Kyro
(Tauri v2). Cette documentation accompagne la sous-tâche **17.5 « Configurer
le bundling Windows MSI/NSIS »** et concentre toutes les valeurs « stables
entre releases » qu'il ne faut **pas** régénérer accidentellement.

## Fichiers

| Fichier | Rôle |
|---|---|
| `nsis-uninstall-hooks.nsh` | Hook NSIS — invite optionnelle « Supprimer aussi mes données » au moment de la désinstallation (Requirement 17.4). Référencé depuis `tauri.conf.json` via `bundle.windows.nsis.installerHooks`. |
| `make-placeholder-icon.ps1` | Script PowerShell qui produit un `icons/icon.ico` minimal pour les builds locaux quand aucun icône définitif n'est encore disponible. |

## Code de mise à jour MSI (WiX `upgradeCode`)

`tauri.conf.json` → `bundle.windows.wix.upgradeCode` :

```
4f6e1c14-3b7a-4d3a-9d2c-9c2c8e8b6f01
```

> **NE JAMAIS RÉGÉNÉRER CET UUID.**
>
> Ce GUID est l'identifiant que Windows Installer utilise pour reconnaître
> qu'une nouvelle version (`major.minor.patch`) de Kyro est une mise à
> jour de la version installée précédemment, et non une application
> distincte. Le changer casse la détection de mise à jour : les
> utilisateurs se retrouveraient avec deux Kyro installés en parallèle
> (`Programmes et fonctionnalités` afficherait deux entrées), avec deux
> répertoires de données potentiellement non cohérents.
>
> Si on a un jour besoin de forcer une réinstallation propre (changement
> majeur du schéma SQLite, par exemple), il faudra **majorer le numéro
> de version** dans `tauri.conf.json` et `Cargo.toml`, pas changer ce
> GUID.
>
> Voir aussi : <https://learn.microsoft.com/en-us/windows/win32/msi/upgradecode>.

Ce GUID a été généré une fois pour toutes lors de la sous-tâche 17.5
(`v0.1.0`) et doit rester identique pour toute la durée de vie de
l'application.

## Hook NSIS — comportement à la désinstallation (Requirement 17.4)

Le fichier `nsis-uninstall-hooks.nsh` définit la macro
`NSIS_HOOK_PREUNINSTALL`, invoquée par le template `installer.nsi` de
Tauri juste avant que l'uninstaller ne retire les fichiers programme.

Comportement :

1. **Par défaut, l'installeur NSIS ne touche pas à `%APPDATA%\Kyro\`.**
   Tauri n'écrit rien dans ce dossier à l'install (la base SQLite,
   les affiches et les logs y sont créés au runtime par le backend Rust)
   — l'uninstaller standard ne le supprime donc pas. Le hook ne fait
   qu'ajouter une option opt-in.
2. En désinstallation **interactive**, une `MessageBox` propose à
   l'utilisateur de supprimer également ses données utilisateur.
   Le bouton par défaut est **Non** (`MB_DEFBUTTON2`), donc une
   pression machinale sur Entrée conserve les données.
3. En désinstallation **silencieuse** (`msiexec /qn`, `Kyro_setup.exe /S`,
   désinstallation orchestrée par Intune ou un GPO), le drapeau
   `/SD IDNO` force la réponse par défaut à « Non » : les données
   sont conservées. Cela respecte la clause « sauf si l'utilisateur
   demande explicitement la suppression » (Requirement 17.4).
4. Si l'utilisateur clique **Oui**, la macro effectue
   `RMDir /r "$APPDATA\Kyro"`, ce qui efface :
   - `library.db` (Kyro_Library) ;
   - `posters/` (affiches copiées localement) ;
   - `logs/` (journaux rotatifs quotidiens) ;
   - les éventuels `setting_value` matérialisés.

## Côté MSI (WiX)

Le bundler MSI ne reçoit pas (encore) de hook équivalent : par défaut,
WiX n'efface que les composants déclarés par Tauri (le binaire installé
sous `Program Files`), pas le profil utilisateur. Le critère 17.4 est
donc respecté côté MSI sans configuration supplémentaire. Si un futur
besoin émerge (option « clean install » dans le MSI), on ajoutera un
fragment WiX dédié et on le branchera via `bundle.windows.wix.fragmentPaths`.

## Vérifier la configuration sans builder l'installeur

`cargo tauri build` est lent et nécessite WiX + NSIS installés.
Pour valider que `tauri.conf.json` reste conforme au schéma Tauri v2
après une modification, il suffit de lancer :

```powershell
cargo check --manifest-path src-tauri/Cargo.toml
```

`tauri-build` (utilisé dans `build.rs`) re-parse `tauri.conf.json`
au début de chaque build : un champ inconnu ou un GUID mal formé
fait échouer cette étape immédiatement.
