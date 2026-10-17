//! Build script Kyro.
//!
//! En plus du build Tauri standard, ce script **copie `libmpv-2.dll`**
//! depuis `src-tauri/binaries/` vers le dossier `target/<profile>/`
//! où Cargo place l'exécutable. Cette copie est nécessaire en mode
//! développement (`cargo run`, `tauri dev`) parce que Windows charge
//! les DLL en priorité depuis le dossier de l'exécutable :
//! sans cette copie, `libloading::Library::new("libmpv-2.dll")`
//! échouerait au runtime sauf à avoir la DLL sur le `PATH` global.
//!
//! En mode production (`tauri build`), Tauri lit l'entrée
//! `bundle.resources` de `tauri.conf.json` et copie la DLL à côté de
//! `kyro.exe` dans l'installeur — donc cette logique de build script
//! ne change pas le comportement final, mais simplifie la vie des
//! développeurs.
//!
//! Si la DLL n'est pas présente dans `binaries/`, on imprime un
//! `cargo:warning` mais on **ne fait pas échouer** le build : Kyro
//! reste utilisable en mode `playback_mode = "external"` sans cette
//! DLL.

use std::path::PathBuf;

const LIBMPV_FILENAME: &str = "libmpv-2.dll";

fn main() {
    // Re-run du script si le binaire change ou disparaît.
    println!("cargo:rerun-if-changed=binaries/{LIBMPV_FILENAME}");

    if let Err(err) = copy_libmpv_to_target() {
        // Non-fatal : on reste sur le warning Cargo (visible dans la
        // sortie de build) sans interrompre la compilation.
        println!(
            "cargo:warning=Kyro: copie de {LIBMPV_FILENAME} ignorée ({err}). \
             Le mode embedded sera indisponible tant que la DLL n'est pas \
             présente dans target/<profile>/ ou à côté de kyro.exe."
        );
    }

    tauri_build::build();
}

fn copy_libmpv_to_target() -> Result<(), String> {
    let manifest_dir = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| "CARGO_MANIFEST_DIR absent".to_string())?;
    let source = manifest_dir.join("binaries").join(LIBMPV_FILENAME);
    if !source.is_file() {
        return Err(format!("source absente: {}", source.display()));
    }

    // OUT_DIR vaut typiquement
    // `target/<profile>/build/<crate>-<hash>/out`. On remonte de 3
    // niveaux pour atteindre `target/<profile>/`. C'est le même
    // pattern que `tauri-build` utilise pour résoudre l'emplacement
    // final de l'exécutable.
    let out_dir = std::env::var_os("OUT_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| "OUT_DIR absent".to_string())?;
    let target_dir = out_dir
        .ancestors()
        .nth(3)
        .ok_or_else(|| format!("impossible de remonter à target/ depuis {}", out_dir.display()))?
        .to_path_buf();

    let dest = target_dir.join(LIBMPV_FILENAME);

    // Copie idempotente : on saute si le fichier de destination est
    // déjà identique en taille (proxy bon marché ; la DLL ne change
    // que rarement et un re-copy à chaque build serait du gaspillage
    // I/O sur 111 Mo).
    if let Ok(dest_meta) = std::fs::metadata(&dest) {
        if let Ok(src_meta) = std::fs::metadata(&source) {
            if dest_meta.len() == src_meta.len() {
                return Ok(());
            }
        }
    }

    std::fs::copy(&source, &dest)
        .map_err(|e| format!("copie {} → {} : {e}", source.display(), dest.display()))?;
    println!(
        "cargo:warning=Kyro: {LIBMPV_FILENAME} copiée vers {}",
        dest.display()
    );
    Ok(())
}
