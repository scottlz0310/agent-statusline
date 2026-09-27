#!/bin/sh
# Usage: verify-release-assets.sh <asset-dir> <version> [<target-to-run>]
# Release に添付する asset 一式（3 ZIP、3 checksum、2 installer）を検証する。
# <target-to-run> を指定した場合は、その ZIP のバイナリを実行して version を確認する。
set -eu

[ "$#" -ge 2 ] && [ "$#" -le 3 ] || {
    echo 'Usage: verify-release-assets.sh <asset-dir> <version> [<target-to-run>]' >&2
    exit 2
}
dir=$1
version=$2
run_target=${3:-}
targets='x86_64-pc-windows-msvc x86_64-unknown-linux-musl aarch64-unknown-linux-musl'

fail() {
    echo "Release asset verification failed: $*" >&2
    exit 1
}

if [ -n "$run_target" ]; then
    case " $targets " in
        *" $run_target "*) ;;
        *) fail "unknown target to run: $run_target" ;;
    esac
fi

expected=$(
    for target in $targets; do
        printf 'agent-statusline-%s.zip\nagent-statusline-%s.zip.sha256\n' "$target" "$target"
    done
    printf 'agent-statusline-installer.ps1\nagent-statusline-installer.sh\n'
)
actual=$(ls -A -- "$dir")
[ "$(printf '%s\n' "$expected" | LC_ALL=C sort)" = "$(printf '%s\n' "$actual" | LC_ALL=C sort)" ] ||
    fail "asset set mismatch; expected: $(echo $expected); actual: $(echo $actual)"

for name in $expected; do
    [ -s "$dir/$name" ] || fail "$name is empty"
done

for target in $targets; do
    archive="agent-statusline-$target.zip"
    bin=agent-statusline
    case "$target" in *-windows-*) bin=agent-statusline.exe ;; esac

    hash=$(sha256sum "$dir/$archive" | awk '{ print $1 }')
    [ "$(cat "$dir/$archive.sha256")" = "$hash *$archive" ] || fail "checksum mismatch for $archive"

    [ "$(unzip -Z1 "$dir/$archive")" = "$bin" ] || fail "$archive must contain only $bin at its root"

    if [ "$target" = "$run_target" ]; then
        run_dir=$(mktemp -d)
        unzip -q "$dir/$archive" -d "$run_dir"
        chmod +x "$run_dir/$bin"
        reported=$("$run_dir/$bin" --version)
        rm -rf -- "$run_dir"
        [ "$reported" = "agent-statusline $version" ] ||
            fail "$archive reports '$reported', expected 'agent-statusline $version'"
    fi
done

echo "Release assets verified for $version."
