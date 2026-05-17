; ----------------------------------------------------------------------------
; nsis-uninstall-hooks.nsh
;
; Hook NSIS branché par Tauri v2 via `bundle.windows.nsis.installerHooks`
; (cf. `tauri.conf.json`). Tauri inclut ce fichier dans son template
; `installer.nsi` et invoque les macros `NSIS_HOOK_*` quand elles sont
; définies.
;
; Objectif (sous-tâche 17.5, Requirement 17.4 « Persistance locale et
; confidentialité ») :
;   « WHEN l'utilisateur désinstalle Kyro, THE Kyro SHALL conserver
;     Kyro_Library dans le répertoire de données utilisateur sauf si
;     l'utilisateur demande explicitement la suppression des données. »
;
; Comportement implémenté :
;   1. Par défaut, la désinstallation NE TOUCHE PAS à `%APPDATA%\Kyro\`.
;      Tauri ne place rien dans ce dossier au moment de l'install — il
;      n'est créé qu'au runtime par le backend Kyro (DB, logs, posters).
;      Donc aucune action de l'uninstaller standard ne le supprime ;
;      ce hook se contente d'AJOUTER une option opt-in.
;   2. Pendant la désinstallation interactive, on présente une boîte
;      `MB_YESNO` (par défaut sur « Non ») demandant à l'utilisateur
;      s'il souhaite ALSO supprimer ses données.
;   3. En désinstallation silencieuse (`/S`), `/SD IDNO` force la
;      réponse par défaut « Non » : on conserve les données. Ceci
;      respecte le critère 17.4 (« sauf si l'utilisateur demande
;      explicitement la suppression »).
;
; Le hook utilisé est `NSIS_HOOK_PREUNINSTALL` : il s'exécute avant
; que l'uninstaller ne retire les fichiers d'installation. C'est le
; meilleur moment pour présenter l'invite parce que l'utilisateur
; vient juste de lancer la désinstallation et est encore en contexte.
; ----------------------------------------------------------------------------

!macro NSIS_HOOK_PREUNINSTALL
  ; Label statique : Tauri inclut ce hook une seule fois dans son
  ; template `installer.nsi`, donc pas de risque de collision.
  ; Eviter `${__LINE__}` qui produisait des labels invalides
  ; (« kyro_keep_userdata_750.6 ») dans NSIS 3.11.
  MessageBox MB_YESNO|MB_DEFBUTTON2|MB_ICONQUESTION \
    "Souhaitez-vous également supprimer vos données utilisateur ?$\n$\nDossier concerné :$\n$APPDATA\Kyro$\n$\n« Oui » supprime définitivement la bibliothèque, les affiches, les journaux et les paramètres locaux.$\n« Non » (par défaut) conserve toutes vos données." \
    /SD IDNO IDNO kyro_keep_userdata
  ; L'utilisateur a explicitement demandé la suppression : on retire
  ; tout le dossier de données Kyro (DB SQLite, posters, logs, settings).
  RMDir /r "$APPDATA\Kyro"
  kyro_keep_userdata:
!macroend
