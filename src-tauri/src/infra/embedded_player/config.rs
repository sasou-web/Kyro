//! Construction des options libmpv pour garantir l'isolation totale
//! vis-à-vis de la configuration mpv.net globale et de l'environnement
//! utilisateur.
//!
//! Ces options sont positionnées **avant** `mpv_initialize` (sauf
//! `start` qui est passée à la commande `loadfile` au moment du
//! chargement). Toute modification de cette liste doit être documentée
//! dans `docs/EMBEDDED_PLAYER.md` § « Garanties d'isolation ».

use std::path::Path;

/// Couple `(option, valeur)` à passer à `mpv_set_option_string` avant
/// `mpv_initialize`. Représentation purement textuelle pour rester
/// trivialement testable et indépendante de l'ordre d'application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MpvOption {
    pub name: &'static str,
    pub value: String,
}

impl MpvOption {
    pub fn new(name: &'static str, value: impl Into<String>) -> Self {
        Self {
            name,
            value: value.into(),
        }
    }
}

/// Construit la liste exacte des options d'isolation à appliquer pour
/// une session embarquée Kyro.
///
/// * `config_dir` — répertoire `%APPDATA%\Kyro\mpv-config` (créé en
///   amont par l'appelant). libmpv lira `mpv.conf`, `input.conf`, etc.
///   uniquement depuis ce dossier.
/// * `embed_hwnd` — HWND parent fourni par la fenêtre Tauri `player`.
///   Stocké en `isize` côté Rust pour rester `Send`.
///
/// Ordre des options :
///
/// 1. `config-dir` — DOIT être posé en premier pour que les autres
///    options ne soient pas écrasées par un éventuel `mpv.conf`
///    auto-chargé.
/// 2. Désactivations explicites (input, OSC, scripts).
/// 3. `wid` — embed Windows.
/// 4. Profil sane par défaut (hwdec, ytdl désactivé).
pub fn build_isolated_options(config_dir: &Path, embed_hwnd: isize) -> Vec<MpvOption> {
    vec![
        // 1. Isolation de configuration ----------------------------------
        MpvOption::new("config-dir", config_dir.to_string_lossy().into_owned()),
        // Empêche libmpv de charger un éventuel script utilisateur depuis
        // un autre dossier (p.ex. via la variable d'env `MPV_HOME`).
        MpvOption::new("load-scripts", "no"),
        // 2. Désactivation des entrées clavier mpv : Kyro pose son
        //    propre handling depuis la WebView Svelte.
        MpvOption::new("input-default-bindings", "no"),
        MpvOption::new("input-vo-keyboard", "no"),
        // Pas d'OSC mpv : l'UI est rendue par Svelte au-dessus.
        MpvOption::new("osc", "no"),
        // 3. Embed Windows : libmpv crée son propre child window
        //    sous le HWND fourni.
        MpvOption::new("wid", embed_hwnd.to_string()),
        // 4. Profil sane Kyro :
        // - hwdec auto-safe : décodage matériel quand fiable, sinon CPU.
        // - ytdl=no : pas de yt-dlp lancé silencieusement (zéro réseau).
        // - terminal=no : pas de stdout/stderr depuis libmpv (Kyro logue).
        // - keep-open=no : EOF natif → end-file event sans rester sur la
        //   dernière frame.
        MpvOption::new("hwdec", "auto-safe"),
        MpvOption::new("ytdl", "no"),
        MpvOption::new("terminal", "no"),
        MpvOption::new("keep-open", "no"),
    ]
}

/// Contenu par défaut du `mpv.conf` Kyro déposé au premier lancement
/// dans `config_dir`. Garde un set minimal et raisonnable ;
/// l'utilisateur peut éditer le fichier pour tweaker (mpv recharge à
/// la prochaine session).
pub const DEFAULT_MPV_CONF: &str = "\
# Configuration mpv générée par Kyro (player embarqué).
# Vous pouvez l'éditer pour personnaliser le rendu.

# Décodage matériel sûr (fallback CPU si problème).
hwdec=auto-safe

# Pas de réseau : Kyro fonctionne 100 %% local.
ytdl=no
network-timeout=0
load-scripts=no

# Sortie audio : auto (laisse mpv choisir WASAPI/DirectSound).
ao=auto

# Confort lecture.
keep-open=no
";

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn build_isolated_options_includes_config_dir_first() {
        let opts = build_isolated_options(&PathBuf::from(r"C:\kyro\mpv"), 12345);
        assert!(!opts.is_empty(), "options non vides");
        assert_eq!(
            opts[0].name, "config-dir",
            "config-dir doit être posé en premier pour que rien ne l'écrase"
        );
        assert_eq!(opts[0].value, r"C:\kyro\mpv");
    }

    #[test]
    fn build_isolated_options_disables_input_and_osc() {
        let opts = build_isolated_options(&PathBuf::from(r"C:\kyro\mpv"), 0);
        let map: std::collections::HashMap<_, _> =
            opts.iter().map(|o| (o.name, o.value.as_str())).collect();
        assert_eq!(map.get("input-default-bindings"), Some(&"no"));
        assert_eq!(map.get("input-vo-keyboard"), Some(&"no"));
        assert_eq!(map.get("osc"), Some(&"no"));
        assert_eq!(map.get("load-scripts"), Some(&"no"));
    }

    #[test]
    fn build_isolated_options_passes_embed_hwnd_as_decimal() {
        // libmpv attend `wid` sous forme décimale (ou hex avec 0x), on
        // utilise décimal pour rester aligné sur la doc client.h.
        let opts = build_isolated_options(&PathBuf::from(r"C:\kyro\mpv"), 0xDEAD_BEEF);
        let wid = opts.iter().find(|o| o.name == "wid").expect("wid présent");
        assert_eq!(wid.value, format!("{}", 0xDEAD_BEEFu64));
    }

    #[test]
    fn build_isolated_options_disables_network_and_terminal() {
        let opts = build_isolated_options(&PathBuf::from(r"C:\kyro\mpv"), 0);
        let map: std::collections::HashMap<_, _> =
            opts.iter().map(|o| (o.name, o.value.as_str())).collect();
        assert_eq!(map.get("ytdl"), Some(&"no"));
        assert_eq!(map.get("terminal"), Some(&"no"));
    }

    #[test]
    fn default_mpv_conf_is_non_empty_and_sets_hwdec() {
        assert!(!DEFAULT_MPV_CONF.is_empty());
        assert!(DEFAULT_MPV_CONF.contains("hwdec=auto-safe"));
        assert!(DEFAULT_MPV_CONF.contains("ytdl=no"));
    }
}
