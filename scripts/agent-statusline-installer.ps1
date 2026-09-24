# agent-statusline installer for Windows (PowerShell)
# Installs agent-statusline to %LOCALAPPDATA%\Programs\agent-statusline and configures PATH.

[CmdletBinding()]
param (
    [Parameter()]
    [string]$Tag = "latest",

    [Parameter()]
    [string]$InstallDir = $(Join-Path $env:LOCALAPPDATA "Programs\agent-statusline"),

    [Parameter()]
    [switch]$NoModifyPath
)

$ErrorActionPreference = "Stop"

$repo = "scottlz0310/agent-statusline"
Write-Host "=== agent-statusline Installer ===" -ForegroundColor Cyan

# 1. Resolve release metadata
if ($Tag -eq "latest") {
    $apiUrl = "https://api.github.com/repos/$repo/releases/latest"
} else {
    $apiUrl = "https://api.github.com/repos/$repo/releases/tags/$Tag"
}

Write-Host "Querying release information from $apiUrl..."
$headers = @{
    "User-Agent" = "agent-statusline-installer"
}
if ($env:GITHUB_TOKEN) {
    $headers["Authorization"] = "Bearer $env:GITHUB_TOKEN"
}

try {
    $release = Invoke-RestMethod -Uri $apiUrl -Headers $headers -UseBasicParsing
} catch {
    Write-Error "Failed to fetch release information from GitHub API: $_"
    exit 1
}

$releaseTag = $release.tag_name
Write-Host "Target release: $releaseTag" -ForegroundColor Green

# 2. Locate windows binary asset
$assetName = "agent-statusline-x86_64-pc-windows-msvc.zip"
$asset = $release.assets | Where-Object { $_.name -eq $assetName }

if (-not $asset) {
    Write-Error "Could not find asset '$assetName' in release '$releaseTag'."
    exit 1
}

$downloadUrl = $asset.browser_download_url
Write-Host "Downloading $assetName from $downloadUrl..."

$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("agent-statusline-install-" + [System.Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $tempDir -Force | Out-Null
$zipFile = Join-Path $tempDir $assetName

try {
    Invoke-WebRequest -Uri $downloadUrl -OutFile $zipFile -UseBasicParsing -Headers $headers

    Write-Host "Extracting archive to temporary directory..."
    $extractDir = Join-Path $tempDir "extracted"
    Expand-Archive -Path $zipFile -DestinationPath $extractDir -Force

    # Ensure destination directory exists
    if (-not (Test-Path -Path $InstallDir)) {
        New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    }

    $binSource = Join-Path $extractDir "agent-statusline.exe"
    if (-not (Test-Path -Path $binSource)) {
        # Check subdirectories if zip contains a root folder
        $sub = Get-ChildItem -Path $extractDir -Filter "agent-statusline.exe" -Recurse | Select-Object -First 1
        if ($sub) {
            $binSource = $sub.FullName
        } else {
            throw "agent-statusline.exe not found inside the downloaded archive."
        }
    }

    $binDest = Join-Path $InstallDir "agent-statusline.exe"
    $binStage = Join-Path $InstallDir "agent-statusline.exe.new"
    $oldDest = Join-Path $InstallDir "agent-statusline.exe.old"

    # Stage new binary in destination directory first to ensure disk space and write permissions
    if (Test-Path -Path $binStage) {
        Remove-Item -Path $binStage -Force -ErrorAction SilentlyContinue
    }
    Copy-Item -Path $binSource -Destination $binStage -Force

    $backedUpExisting = $false
    try {
        # If destination exe exists and is running/locked, rename it to .old
        if (Test-Path -Path $binDest) {
            if (Test-Path -Path $oldDest) {
                Remove-Item -Path $oldDest -Force -ErrorAction SilentlyContinue
            }
            Move-Item -Path $binDest -Destination $oldDest -Force
            $backedUpExisting = $true
        }

        # Activate the staged binary
        Move-Item -Path $binStage -Destination $binDest -Force
    } catch {
        # Rollback: restore .old if new binary activation failed
        if ($backedUpExisting -and (Test-Path -Path $oldDest) -and (-not (Test-Path -Path $binDest))) {
            Move-Item -Path $oldDest -Destination $binDest -Force -ErrorAction SilentlyContinue
        }
        throw "Failed to replace agent-statusline.exe: $_"
    } finally {
        if (Test-Path -Path $binStage) {
            Remove-Item -Path $binStage -Force -ErrorAction SilentlyContinue
        }
    }

    Write-Host "Installed agent-statusline.exe to $InstallDir" -ForegroundColor Green

    # 3. Add to User PATH if not already present
    if (-not $NoModifyPath) {
        $userPath = [System.Environment]::GetEnvironmentVariable("Path", "User")
        $pathEntries = if ($userPath) { $userPath -split ';' } else { @() }

        $normalizedInstallDir = $InstallDir.TrimEnd('\')
        $alreadyInPath = $pathEntries | Where-Object { $_.TrimEnd('\') -eq $normalizedInstallDir }

        if (-not $alreadyInPath) {
            Write-Host "Adding $InstallDir to User PATH..."
            $newPath = if ($userPath) { "$userPath;$InstallDir" } else { $InstallDir }
            [System.Environment]::SetEnvironmentVariable("Path", $newPath, "User")
            $env:Path = "$env:Path;$InstallDir"
            Write-Host "PATH updated successfully. (Restart your shell to apply globally)" -ForegroundColor Green
        } else {
            Write-Host "$InstallDir is already in User PATH." -ForegroundColor Gray
        }
    }

    Write-Host ""
    Write-Host "Installation completed successfully!" -ForegroundColor Green
    Write-Host "Run 'agent-statusline --help' or 'agent-statusline --version' to verify." -ForegroundColor Cyan
} finally {
    if (Test-Path -Path $tempDir) {
        Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}
