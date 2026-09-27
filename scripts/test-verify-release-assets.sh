#!/bin/sh
set -eu

verify=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)/verify-release-assets.sh
temp=$(mktemp -d)
trap 'rm -rf -- "$temp"' EXIT
targets='x86_64-pc-windows-msvc x86_64-unknown-linux-musl aarch64-unknown-linux-musl'

# <dir> <target> <entry>...: 指定エントリを持つ ZIP と checksum を作る
make_archive() {
    dir=$1
    archive="agent-statusline-$2.zip"
    shift 2
    python3 - "$dir/$archive" "$@" <<'PY'
import sys
import zipfile

with zipfile.ZipFile(sys.argv[1], 'w') as archive:
    for entry in sys.argv[2:]:
        info = zipfile.ZipInfo(entry)
        info.external_attr = 0o755 << 16
        archive.writestr(info, '#!/bin/sh\necho "agent-statusline 0.2.0"\n')
PY
    (cd "$dir" && sha256sum -b "$archive" > "$archive.sha256")
}

make_valid() {
    dir="$temp/$1"
    mkdir -p "$dir"
    for target in $targets; do
        bin=agent-statusline
        case "$target" in *-windows-*) bin=agent-statusline.exe ;; esac
        make_archive "$dir" "$target" "$bin"
    done
    printf 'ps1' > "$dir/agent-statusline-installer.ps1"
    printf 'sh' > "$dir/agent-statusline-installer.sh"
}

linux=agent-statusline-x86_64-unknown-linux-musl.zip
# name|expected result|version|target to run|mutation
cases="valid|pass|0.2.0||:
run-target|pass|0.2.0|x86_64-unknown-linux-musl|:
version-mismatch|fail|0.3.0|x86_64-unknown-linux-musl|:
unknown-target|fail|0.2.0|x86_64-apple-darwin|:
missing-asset|fail|0.2.0||rm \"\$dir/agent-statusline-installer.sh\"
extra-asset|fail|0.2.0||printf x > \"\$dir/sha256.sum\"
empty-installer|fail|0.2.0||: > \"\$dir/agent-statusline-installer.ps1\"
checksum-mismatch|fail|0.2.0||printf '%064d *$linux\n' 0 > \"\$dir/$linux.sha256\"
checksum-name-mismatch|fail|0.2.0||sed -i 's/ \\*.*/ *other.zip/' \"\$dir/$linux.sha256\"
nested-binary|fail|0.2.0||make_archive \"\$dir\" x86_64-unknown-linux-musl dir/agent-statusline
extra-entry|fail|0.2.0||make_archive \"\$dir\" x86_64-unknown-linux-musl agent-statusline README.md
windows-name|fail|0.2.0||make_archive \"\$dir\" x86_64-pc-windows-msvc agent-statusline"

printf '%s\n' "$cases" | while IFS='|' read -r name expected version run_target mutation; do
    make_valid "$name"
    dir="$temp/$name"
    eval "$mutation"
    if sh "$verify" "$dir" "$version" $run_target >/dev/null 2>&1; then
        result=pass
    else
        result=fail
    fi
    [ "$result" = "$expected" ] || {
        echo "Unexpected $result: $name" >&2
        exit 1
    }
done
echo 'Release asset verification tests passed.'
