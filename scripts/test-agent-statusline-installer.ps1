$ErrorActionPreference = 'Stop'

$installer = Join-Path $PSScriptRoot 'agent-statusline-installer.ps1'
$temp = Join-Path ([IO.Path]::GetTempPath()) ("agent-statusline-installer-test-" + [guid]::NewGuid().ToString('N'))
$archiveName = 'agent-statusline-x86_64-pc-windows-msvc.zip'
$global:installerTestArchivePath = Join-Path $temp $archiveName
$global:installerTestChecksumPath = Join-Path $temp "$archiveName.sha256"
$global:installerTestAssets = @()

function Invoke-RestMethod {
    return [pscustomobject]@{ tag_name = 'v0.2.0'; assets = $global:installerTestAssets }
}

function Invoke-WebRequest {
    param($Uri, $OutFile)
    if ($Uri -eq 'archive') {
        Copy-Item -LiteralPath $global:installerTestArchivePath -Destination $OutFile
    } else {
        Copy-Item -LiteralPath $global:installerTestChecksumPath -Destination $OutFile
    }
}

try {
    New-Item -ItemType Directory -Path $temp | Out-Null
    foreach ($case in 'valid', 'mismatch', 'missing-checksum', 'missing-binary') {
        $source = Join-Path $temp "source-$case"
        New-Item -ItemType Directory -Path $source -Force | Out-Null
        if ($case -ne 'missing-binary') {
            Set-Content -LiteralPath (Join-Path $source 'agent-statusline.exe') -Value 'new binary' -NoNewline
        } else {
            Set-Content -LiteralPath (Join-Path $source 'README.md') -Value 'no binary' -NoNewline
        }
        if (Test-Path -LiteralPath $global:installerTestArchivePath) {
            Remove-Item -LiteralPath $global:installerTestArchivePath
        }
        Compress-Archive -Path (Join-Path $source '*') -DestinationPath $global:installerTestArchivePath
        $hash = (Get-FileHash -LiteralPath $global:installerTestArchivePath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($case -eq 'mismatch') { $hash = '0' * 64 }
        Set-Content -LiteralPath $global:installerTestChecksumPath -Value "$hash *$archiveName"
        $global:installerTestAssets = @(
            [pscustomobject]@{ name = $archiveName; browser_download_url = 'archive' }
        )
        if ($case -ne 'missing-checksum') {
            $global:installerTestAssets += [pscustomobject]@{ name = "$archiveName.sha256"; browser_download_url = 'checksum' }
        }
        $installDir = Join-Path $temp "install-$case"
        New-Item -ItemType Directory -Path $installDir | Out-Null
        $destination = Join-Path $installDir 'agent-statusline.exe'
        Set-Content -LiteralPath $destination -Value 'old binary' -NoNewline

        $failed = $false
        $failure = $null
        try {
            & $installer -Tag v0.2.0 -InstallDir $installDir -NoModifyPath
        } catch {
            $failed = $true
            $failure = $_
        }
        if ($failed -ne ($case -ne 'valid')) {
            throw "Unexpected installer result: $case ($failure)"
        }
        $expected = if ($case -eq 'valid') { 'new binary' } else { 'old binary' }
        if ((Get-Content -LiteralPath $destination -Raw) -ne $expected) {
            throw "Existing binary was not preserved or replaced correctly: $case"
        }
    }
    Write-Host 'Windows installer tests passed.'
} finally {
    if (Test-Path -LiteralPath $temp) {
        Remove-Item -LiteralPath $temp -Recurse -Force
    }
}
