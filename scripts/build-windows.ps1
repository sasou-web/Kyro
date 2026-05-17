<#
.SYNOPSIS
    Build script Windows pour Kyro (Tauri 2.x).

.DESCRIPTION
    Exécute `cargo tauri build` (cibles MSI + NSIS), copie les artefacts
    dans `dist/` à la racine du dépôt, puis vérifie que la taille du binaire
    principal `Kyro.exe` est inférieure ou égale à 50 Mo conformément au
    Requirement 1.3.

    Note : ffprobe est livré comme sidecar à côté de `Kyro.exe`, il n'est
    pas embarqué dans le binaire principal. La vérification de taille porte
    donc directement sur `Kyro.exe`. La taille du sidecar est uniquement
    rapportée à titre informatif dans le résumé.

.PARAMETER SkipBuild
    Lorsqu'il est positionné, le script ne lance pas `cargo tauri build` et
    se contente de collecter les artefacts existants. Utile pour relancer
    uniquement la phase de packaging/vérification.

.PARAMETER MaxBinaryBytes
    Limite (en octets) appliquée à la taille du binaire principal `Kyro.exe`.
    Par défaut 50 * 1024 * 1024.

.EXAMPLE
    pwsh -NoProfile -File scripts/build-windows.ps1

.EXAMPLE
    pwsh -NoProfile -File scripts/build-windows.ps1 -SkipBuild

.NOTES
    Requirements: 1.1 (binaire Windows Tauri), 1.3 (taille ≤ 50 Mo hors ffprobe).
#>

[CmdletBinding()]
param(
    [switch] $SkipBuild,
    [long]   $MaxBinaryBytes = (50 * 1024 * 1024),

    # ----- Signature de code (optionnelle) ---------------------------------
    # Active la signature Authenticode des artefacts via `signtool` quand
    # un certificat est fourni. Activé automatiquement si la variable
    # d'environnement `KYRO_SIGN_CERT` pointe sur un .pfx valide.
    #
    # `KYRO_SIGN_CERT_PASSWORD` peut contenir le mot de passe du
    # certificat (sinon `signtool` le demandera de manière interactive).
    [string] $SignCertPath = $env:KYRO_SIGN_CERT,
    [string] $SignCertPassword = $env:KYRO_SIGN_CERT_PASSWORD,
    [string] $TimestampUrl = 'http://timestamp.digicert.com'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# ----------------------------------------------------------------------------
# Helpers
# ----------------------------------------------------------------------------

function Write-Section {
    param([string] $Title)
    Write-Host ''
    Write-Host ('=== {0} ===' -f $Title) -ForegroundColor Cyan
}

function Format-Size {
    param([long] $Bytes)
    if ($Bytes -lt 1KB) { return ('{0} o' -f $Bytes) }
    if ($Bytes -lt 1MB) { return ('{0:N1} Kio' -f ($Bytes / 1KB)) }
    if ($Bytes -lt 1GB) { return ('{0:N2} Mio' -f ($Bytes / 1MB)) }
    return ('{0:N2} Gio' -f ($Bytes / 1GB))
}

function Resolve-WorkspaceRoot {
    # Le script est attendu à <root>/scripts/build-windows.ps1
    $scriptDir = Split-Path -Parent $PSCommandPath
    return (Resolve-Path (Join-Path $scriptDir '..')).Path
}

function Invoke-CargoTauriBuild {
    param([string] $WorkspaceRoot)

    Write-Section 'cargo tauri build'

    Push-Location $WorkspaceRoot
    try {
        # Tauri v2 accepte --bundles pour cibler explicitement msi et nsis.
        # On laisse cargo gérer la sortie sur stdout/stderr (pas de capture).
        & cargo tauri build --bundles msi,nsis
        if ($LASTEXITCODE -ne 0) {
            throw "cargo tauri build a échoué avec le code $LASTEXITCODE."
        }
    }
    finally {
        Pop-Location
    }
}

function Copy-Artifacts {
    param(
        [string] $WorkspaceRoot,
        [string] $DistDir
    )

    Write-Section 'Collecte des artefacts MSI / NSIS'

    if (-not (Test-Path -LiteralPath $DistDir)) {
        New-Item -ItemType Directory -Path $DistDir | Out-Null
    }

    $bundleRoot = Join-Path $WorkspaceRoot 'src-tauri\target\release\bundle'
    $msiDir     = Join-Path $bundleRoot 'msi'
    $nsisDir    = Join-Path $bundleRoot 'nsis'

    $copied = New-Object System.Collections.Generic.List[object]

    foreach ($pair in @(
            @{ Dir = $msiDir;  Pattern = '*.msi' },
            @{ Dir = $nsisDir; Pattern = '*.exe' }
        )) {
        if (-not (Test-Path -LiteralPath $pair.Dir)) {
            Write-Warning ("Dossier de bundle introuvable : {0}" -f $pair.Dir)
            continue
        }

        $found = Get-ChildItem -LiteralPath $pair.Dir -Filter $pair.Pattern -File -ErrorAction SilentlyContinue
        if (-not $found) {
            Write-Warning ("Aucun artefact {0} trouvé dans {1}" -f $pair.Pattern, $pair.Dir)
            continue
        }

        foreach ($artifact in $found) {
            $dest = Join-Path $DistDir $artifact.Name
            Copy-Item -LiteralPath $artifact.FullName -Destination $dest -Force
            $copied.Add([pscustomobject]@{
                Name = $artifact.Name
                Size = (Get-Item -LiteralPath $dest).Length
                Path = $dest
            }) | Out-Null
        }
    }

    return ,$copied
}

function Get-FfprobeSidecarSize {
    param([string] $WorkspaceRoot)

    # Le sidecar ffprobe est livré à côté de Kyro.exe (externalBin) et
    # n'est PAS embarqué dans le binaire principal. On rapporte sa taille
    # uniquement à titre informatif dans le résumé.
    $candidates = @(
        'src-tauri\binaries\ffprobe-x86_64-pc-windows-msvc.exe',
        'src-tauri\binaries\ffprobe.exe'
    )

    foreach ($rel in $candidates) {
        $full = Join-Path $WorkspaceRoot $rel
        if (Test-Path -LiteralPath $full -PathType Leaf) {
            return [pscustomobject]@{
                Path = $full
                Size = (Get-Item -LiteralPath $full).Length
            }
        }
    }

    Write-Warning 'Sidecar ffprobe introuvable ; sa taille ne sera pas rapportée.'
    return [pscustomobject]@{ Path = $null; Size = 0L }
}

function Resolve-MainBinary {
    param([string] $WorkspaceRoot)

    $candidates = @(
        'src-tauri\target\release\Kyro.exe',
        'src-tauri\target\release\kyro.exe'
    )

    foreach ($rel in $candidates) {
        $full = Join-Path $WorkspaceRoot $rel
        if (Test-Path -LiteralPath $full -PathType Leaf) {
            return $full
        }
    }

    throw ("Binaire principal introuvable. Cherché : {0}" -f ($candidates -join ', '))
}

# ----------------------------------------------------------------------------
# Signature Authenticode optionnelle (`signtool`).
#
# Active la signature seulement si :
#   1. un chemin de certificat `.pfx` est fourni (paramètre ou env var) ;
#   2. le fichier existe ;
#   3. `signtool.exe` est disponible (Windows SDK).
#
# Best-effort : un échec de signature n'interrompt pas le build (on
# warn) — la signature est strictement additive, l'archive reste
# fonctionnelle sans elle.
# ----------------------------------------------------------------------------

function Find-SignTool {
    # signtool est typiquement dans `%ProgramFiles(x86)%\Windows Kits\10\bin\<sdk>\x64\signtool.exe`.
    # On essaie d'abord le PATH (le plus courant si le SDK est installé proprement).
    $cmd = Get-Command 'signtool.exe' -ErrorAction SilentlyContinue
    if ($cmd) {
        return $cmd.Source
    }

    # Fallback : scan classique des emplacements Kit Windows 10/11.
    $roots = @(
        "${env:ProgramFiles(x86)}\Windows Kits\10\bin",
        "${env:ProgramFiles}\Windows Kits\10\bin"
    ) | Where-Object { $_ -and (Test-Path -LiteralPath $_) }

    foreach ($root in $roots) {
        $found = Get-ChildItem -LiteralPath $root -Recurse -Filter 'signtool.exe' -ErrorAction SilentlyContinue |
            Where-Object { $_.FullName -like '*\x64\signtool.exe' } |
            Sort-Object FullName -Descending |
            Select-Object -First 1
        if ($found) {
            return $found.FullName
        }
    }

    return $null
}

function Invoke-SignArtifacts {
    param(
        [string] $CertPath,
        [string] $CertPassword,
        [string] $TimestampUrl,
        [System.Collections.Generic.List[object]] $Artifacts,
        [string] $MainBinaryPath
    )

    if (-not $CertPath) {
        Write-Host '(signature: KYRO_SIGN_CERT non défini — étape ignorée)' -ForegroundColor DarkGray
        return $false
    }

    if (-not (Test-Path -LiteralPath $CertPath -PathType Leaf)) {
        Write-Warning ("Signature ignorée : certificat introuvable à `{0}`." -f $CertPath)
        return $false
    }

    $signTool = Find-SignTool
    if (-not $signTool) {
        Write-Warning 'Signature ignorée : signtool.exe introuvable (Windows SDK non installé ?).'
        return $false
    }

    Write-Section 'Signature Authenticode'
    Write-Host ('signtool : {0}' -f $signTool)
    Write-Host ('cert     : {0}' -f $CertPath)
    Write-Host ('horodate : {0}' -f $TimestampUrl)

    # Liste des fichiers à signer : binaire principal + tous les
    # artefacts MSI/NSIS du dist.
    $targets = New-Object System.Collections.Generic.List[string]
    $targets.Add($MainBinaryPath) | Out-Null
    foreach ($a in $Artifacts) {
        $targets.Add($a.Path) | Out-Null
    }

    # Arguments communs.
    $args = @(
        'sign',
        '/fd', 'SHA256',
        '/td', 'SHA256',
        '/tr', $TimestampUrl,
        '/f', $CertPath
    )
    if ($CertPassword) {
        $args += @('/p', $CertPassword)
    }

    $signedCount = 0
    foreach ($target in $targets) {
        Write-Host ("→ signature : {0}" -f (Split-Path -Leaf $target))
        $localArgs = $args + @($target)
        & $signTool @localArgs
        if ($LASTEXITCODE -eq 0) {
            $signedCount++
        }
        else {
            Write-Warning ("signtool a retourné le code {0} pour {1}." -f $LASTEXITCODE, $target)
        }
    }

    Write-Host ('Signature : {0} / {1} fichiers signés.' -f $signedCount, $targets.Count) -ForegroundColor Green
    return ($signedCount -gt 0)
}

# ----------------------------------------------------------------------------
# Génération des sommes SHA-256 pour vérification d'intégrité côté
# utilisateur. Format compatible avec `sha256sum -c` (Linux / Git Bash) :
#
#     <hash>  *<filename>
#
# Le fichier `dist/SHA256SUMS.txt` est régénéré à chaque build.
# ----------------------------------------------------------------------------

function Write-Sha256Sums {
    param(
        [string] $DistDir,
        [System.Collections.Generic.List[object]] $Artifacts,
        [string] $MainBinaryPath
    )

    Write-Section 'Génération SHA-256 (intégrité)'

    $sumsPath = Join-Path $DistDir 'SHA256SUMS.txt'
    $lines = New-Object System.Collections.Generic.List[string]

    # Ajout du binaire principal en plus des artefacts. On copie aussi
    # le `Kyro.exe` brut dans dist/ pour que l'utilisateur ait accès à
    # la version « portable » (sans installeur) si souhaité.
    $portableDest = Join-Path $DistDir 'Kyro-portable.exe'
    Copy-Item -LiteralPath $MainBinaryPath -Destination $portableDest -Force

    $allFiles = @($portableDest) + ($Artifacts | ForEach-Object { $_.Path })

    foreach ($f in $allFiles) {
        $hash = (Get-FileHash -LiteralPath $f -Algorithm SHA256).Hash.ToLowerInvariant()
        $name = Split-Path -Leaf $f
        $lines.Add(("{0}  *{1}" -f $hash, $name)) | Out-Null
        Write-Host ("{0}  {1}" -f $hash.Substring(0, 12), $name)
    }

    $lines | Set-Content -LiteralPath $sumsPath -Encoding ASCII
    Write-Host ('Écrit : {0}' -f $sumsPath) -ForegroundColor Green
}

function Assert-BinarySize {
    param(
        [string] $BinaryPath,
        [long]   $FfprobeSize,
        [long]   $MaxBytes
    )

    Write-Section 'Vérification taille (Requirement 1.3)'

    $binarySize = (Get-Item -LiteralPath $BinaryPath).Length

    Write-Host ('Binaire             : {0}' -f $BinaryPath)
    Write-Host ('Taille              : {0} ({1} octets)' -f (Format-Size $binarySize), $binarySize)
    Write-Host ('Seuil               : {0} ({1} octets)' -f (Format-Size $MaxBytes), $MaxBytes)
    Write-Host ('Sidecar ffprobe     : {0} (livré à côté, non embarqué)' -f (Format-Size $FfprobeSize))

    if ($binarySize -gt $MaxBytes) {
        Write-Error ("Taille du binaire ({0}) supérieure au seuil de {1}." -f (Format-Size $binarySize), (Format-Size $MaxBytes))
        exit 1
    }

    Write-Host '✓ Taille conforme au Requirement 1.3' -ForegroundColor Green

    return [pscustomobject]@{
        BinarySize  = $binarySize
        FfprobeSize = $FfprobeSize
        MaxBytes    = $MaxBytes
    }
}

function Write-Summary {
    param(
        [pscustomobject]   $SizeReport,
        [System.Collections.Generic.List[object]] $Artifacts,
        [string]           $DistDir
    )

    Write-Section 'Résumé'

    Write-Host ('Dossier dist        : {0}' -f $DistDir)

    if ($Artifacts.Count -eq 0) {
        Write-Host '(aucun artefact copié)'
    }
    else {
        $rows = foreach ($a in $Artifacts) {
            [pscustomobject]@{
                Artefact = $a.Name
                Taille   = (Format-Size $a.Size)
            }
        }
        $rows | Format-Table -AutoSize | Out-String | Write-Host
    }

    Write-Host ('Binaire             : {0} (seuil {1})' -f (Format-Size $SizeReport.BinarySize), (Format-Size $SizeReport.MaxBytes))
    Write-Host ('  - sidecar ffprobe : {0} (livré à côté, non embarqué)' -f (Format-Size $SizeReport.FfprobeSize))
}

# ----------------------------------------------------------------------------
# Main
# ----------------------------------------------------------------------------

$workspaceRoot = Resolve-WorkspaceRoot
$distDir       = Join-Path $workspaceRoot 'dist'

Write-Host ('Workspace : {0}' -f $workspaceRoot)
Write-Host ('Dist      : {0}' -f $distDir)

if ($SkipBuild) {
    Write-Warning 'SkipBuild: cargo tauri build n''est pas exécuté.'
}
else {
    Invoke-CargoTauriBuild -WorkspaceRoot $workspaceRoot
}

$artifacts   = Copy-Artifacts -WorkspaceRoot $workspaceRoot -DistDir $distDir
$ffprobe     = Get-FfprobeSidecarSize -WorkspaceRoot $workspaceRoot
$binaryPath  = Resolve-MainBinary -WorkspaceRoot $workspaceRoot
$sizeReport  = Assert-BinarySize -BinaryPath $binaryPath -FfprobeSize $ffprobe.Size -MaxBytes $MaxBinaryBytes

# Signature Authenticode optionnelle (best-effort, no-op si pas
# configuré). Doit être faite AVANT Write-Sha256Sums pour que les
# hashes reflètent le binaire signé.
$signed = Invoke-SignArtifacts `
    -CertPath $SignCertPath `
    -CertPassword $SignCertPassword `
    -TimestampUrl $TimestampUrl `
    -Artifacts $artifacts `
    -MainBinaryPath $binaryPath

# Sommes SHA-256 pour vérification d'intégrité côté utilisateur.
# Toujours générées, signées ou pas.
Write-Sha256Sums -DistDir $distDir -Artifacts $artifacts -MainBinaryPath $binaryPath

Write-Summary -SizeReport $sizeReport -Artifacts $artifacts -DistDir $distDir

if ($signed) {
    Write-Host '✓ Artefacts signés (Authenticode).' -ForegroundColor Green
}
else {
    Write-Host '(non signé — voir docs/INSTALL.md sur le warning SmartScreen)' -ForegroundColor DarkYellow
}

exit 0
