<#
.SYNOPSIS
  エントリポイント・CLI層（コードビハインド）の行数が上限を超えていないか検証する（#9）。
.DESCRIPTION
  src/main.rs および src/cli.rs はカバレッジ計測から除外されているため（codecov.yml の ignore）、
  ここへドメインロジックを書くとテストを書かずに品質ゲートを通せてしまう。
  エントリポイントの薄さを保ち、肥大化・ロジック混入を機械的に検知するため、
  ファイルごとに行数の上限（ratchet）を設ける。

  上限は「現在の実測値」に設定する。リファクタリングが進んで行数が減ったら、その PR で上限も下げる
  （ratchet を締める）。逆に上限を超える追加が必要な場合は、同じ PR で上限を引き上げる。
  上限の引き上げ自体は禁止しないが、無意識に増やせないようにするのがこのチェックの目的である。
.EXAMPLE
  pwsh -File ./scripts/check-code-behind-size.ps1
#>
param(
    [string]$RepositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
)

$ErrorActionPreference = "Stop"

# ファイルごとの行数上限（リポジトリルートからの相対パス）。
# 値の更新は「抽出して減った」「意図して増やす」のいずれかを PR で説明できるときだけ行う。
$limits = [ordered]@{
    # エントリポイント・CLI引数解析・ディスパッチ・簡易出力
    "src/main.rs" = 300
    # clap CLI 引数定義
    "src/cli.rs"  = 114
}

$violations = New-Object System.Collections.Generic.List[string]

foreach ($entry in $limits.GetEnumerator()) {
    $relative = $entry.Key
    $limit = $entry.Value
    $path = Join-Path $RepositoryRoot $relative
    if (-not (Test-Path -LiteralPath $path)) {
        $violations.Add("$relative : 上限が登録されているファイルが存在しません（削除・移動したなら limits からも消してください）")
        continue
    }

    # 空行を含む物理行数で数える。Measure-Object -Line は空行を数えないため、
    # エディタや wc -l が示す行数と食い違い、上限の意味が分かりにくくなるのを防ぐ
    $actual = @(Get-Content -LiteralPath $path).Count
    $status = if ($actual -gt $limit) { "NG" } else { "ok" }
    Write-Host ("{0,-4} {1,6} / {2,-6} {3}" -f $status, $actual, $limit, $relative)

    if ($actual -gt $limit) {
        $violations.Add("$relative : $actual 行（上限 $limit 行、超過 $($actual - $limit) 行）")
    }
}

if ($violations.Count -gt 0) {
    Write-Host ""
    Write-Host "行数の上限を超えたコードビハインドがあります:" -ForegroundColor Red
    $violations | ForEach-Object { Write-Host "  - $_" }
    Write-Host ""
    Write-Host "ロジックや状態管理は engine/ や modules/ 等のドメインモジュールへ抽出してください。"
    Write-Host "意図した増加であれば、同じ PR で `$limits の値を更新し、理由を PR に記載してください。"
    exit 1
}

Write-Host ""
Write-Host "OK: すべてのコードビハインドが上限内です。"
