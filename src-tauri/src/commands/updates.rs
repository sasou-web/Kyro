//! Vérification de version et accès aux releases (v1.0).
//!
//! Kyro respecte une contrainte architecturale forte : **aucune
//! dépendance HTTP cliente** (Requirement 17.3, Property 13). Cela
//! interdit l'utilisation du plugin `tauri-plugin-updater` officiel
//! qui télécharge un manifest distant via `reqwest`.
//!
//! Approche retenue pour v1.0 : pas de mise à jour automatique. Le
//! frontend expose simplement :
//!
//! 1. la version actuelle (lue à la compilation depuis
//!    `CARGO_PKG_VERSION` côté Rust, exposée via [`get_app_version`]) ;
//! 2. un lien « Voir les releases » qui ouvre l'URL GitHub Releases
//!    dans le navigateur système via [`open_releases_page`].
//!
//! L'utilisateur télécharge le MSI manuellement, l'installeur Windows
//! gère l'upgrade par numéro de version (le `upgradeCode` WIX est fixe
//! et le bump de version dans `tauri.conf.json` déclenche une
//! réinstallation propre).
//!
//! Une future v1.1 pourra ajouter un updater HTTP **opt-in** derrière
//! la feature flag `network` (déjà réservée dans `Cargo.toml`).
//!
//! _Validates: Requirements 17.3, 6.4_

use crate::error::KyroError;

/// URL canonique de la page « Releases » GitHub de Kyro. Embarquée à
/// la compilation pour éviter toute lecture de configuration runtime
/// (le manifest produit doit être déterministe).
const RELEASES_URL: &str = "https://github.com/sasou-web/Kyro/releases";

/// Métadonnées minimales sur la version courante exposées au frontend.
/// Sert à afficher « Kyro v1.0.0 » dans la section Paramètres.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AppVersionInfo {
    /// Version sémantique compilée (`CARGO_PKG_VERSION`).
    pub version: String,
    /// URL canonique vers les releases publiques (lecture seule pour
    /// le frontend qui peut l'afficher dans un texte explicatif).
    pub releases_url: String,
}

/// Commande Tauri : retourne la version courante de Kyro.
///
/// Pure (zéro IO, zéro réseau). Le frontend l'appelle au montage de
/// la page Paramètres pour afficher « Version actuelle ».
#[tauri::command]
pub async fn get_app_version() -> Result<AppVersionInfo, KyroError> {
    Ok(AppVersionInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        releases_url: RELEASES_URL.to_string(),
    })
}

/// Commande Tauri : ouvre la page GitHub Releases dans le navigateur
/// système Windows.
///
/// Aucune requête réseau côté Kyro. On utilise `cmd /C start <url>`
/// qui délègue au shell Windows : le navigateur par défaut prend la
/// main et fait la requête HTTP de son côté. Aucune dépendance HTTP
/// n'est ajoutée à Kyro (Property 13 respectée).
///
/// La commande retourne aussitôt après le `spawn` ; aucun blocage UI.
#[tauri::command]
pub async fn open_releases_page() -> Result<(), KyroError> {
    // Sécurité : on n'accepte aucune URL fournie par le frontend, on
    // utilise exclusivement la constante `RELEASES_URL`. Évite tout
    // détournement (un acteur malveillant qui exfiltrerait via un IPC
    // détourné ne pourrait pas faire ouvrir une URL arbitraire).
    let url = RELEASES_URL;

    #[cfg(target_os = "windows")]
    {
        // `cmd /C start "" <url>` — le `""` est un titre de fenêtre
        // bidon nécessaire car `start` interprète son premier argument
        // comme le titre quand il commence par des guillemets.
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .map_err(|e| {
                KyroError::Validation(format!(
                    "impossible d'ouvrir le navigateur : {e}"
                ))
            })?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        // Stub non-Windows : Kyro cible Windows en production
        // (Requirement 1.1) ; on retourne un succès silencieux pour
        // que les CI Linux/macOS ne fassent pas échouer la build.
        let _ = url;
    }

    Ok(())
}
