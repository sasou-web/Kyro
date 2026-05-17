# Crée un fichier ICO minimal de 1x1 pixel pour que `tauri::generate_context!`
# puisse compiler. Sera remplacé par un véritable icône Kyro lors d'une tâche
# ultérieure (UI / packaging).
$ErrorActionPreference = "Stop"
$dir = Join-Path $PSScriptRoot "..\icons"
if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir | Out-Null }
$path = Join-Path $dir "icon.ico"

# Header ICO (6 octets)
$header = [byte[]]@(0, 0, 1, 0, 1, 0)
# Entrée ICONDIRENTRY (16 octets) : 1x1, 32 bpp, taille image = 40+4 = 44 octets, offset = 22
$entry = [byte[]]@(1, 1, 0, 0, 1, 0, 32, 0, 44, 0, 0, 0, 22, 0, 0, 0)
# BITMAPINFOHEADER (40 octets) : 1x2 (hauteur doublée pour le masque XOR/AND), 1 plane, 32 bpp, BI_RGB
$bmih = [byte[]]@(40, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 1, 0, 32, 0, 0, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)
# Pixel BGRA (1 pixel : gris-bleu Adwaita Dark approx #2a2b3d)
$pixel = [byte[]]@(0x3d, 0x2b, 0x2a, 0xff)

$bytes = $header + $entry + $bmih + $pixel
[IO.File]::WriteAllBytes($path, $bytes)
Write-Host ("Wrote {0} ({1} bytes)" -f $path, (Get-Item $path).Length)
