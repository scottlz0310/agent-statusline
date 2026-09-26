#!/bin/sh
set -eu

installer=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)/agent-statusline-installer.sh
temp=$(mktemp -d)
trap 'rm -rf -- "$temp"' EXIT
mkdir -p "$temp/mock-bin" "$temp/fixtures" "$temp/home"
export FIXTURE_DIR="$temp/fixtures"
export HOME="$temp/home"
export PATH="$temp/mock-bin:$PATH"

cat > "$temp/mock-bin/curl" <<'MOCK'
#!/bin/sh
format=
while [ "$#" -gt 0 ]; do
    case "$1" in
        -o) output=$2; shift 2 ;;
        -w) format=$2; shift 2 ;;
        *) url=$1; shift ;;
    esac
done
if [ "$format" = '%{url_effective}' ]; then
    printf '%s' 'https://github.com/scottlz0310/agent-statusline/releases/tag/v0.2.0'
    exit 0
fi
cp "$FIXTURE_DIR/${url##*/}" "$output"
MOCK
chmod +x "$temp/mock-bin/curl"

archive=agent-statusline-x86_64-unknown-linux-musl.zip
make_archive() {
    python3 - "$FIXTURE_DIR/$archive" "$1" <<'PY'
import sys
import zipfile

with zipfile.ZipFile(sys.argv[1], 'w') as archive:
    archive.writestr(sys.argv[2], b'new binary')
PY
    hash=$(sha256sum "$FIXTURE_DIR/$archive" | awk '{ print $1 }')
    printf '%s *%s\n' "$hash" "$archive" > "$FIXTURE_DIR/$archive.sha256"
}

run_case() {
    name=$1
    expected=$2
    install_dir="$temp/install-$name"
    mkdir -p "$install_dir"
    printf 'old binary' > "$install_dir/agent-statusline"
    if sh "$installer" --tag v0.2.0 --install-dir "$install_dir"; then
        [ "$expected" = 'new binary' ] || { echo "Unexpected success: $name" >&2; exit 1; }
    else
        [ "$expected" = 'old binary' ] || { echo "Unexpected failure: $name" >&2; exit 1; }
    fi
    [ "$(cat "$install_dir/agent-statusline")" = "$expected" ] || {
        echo "Incorrect binary after $name" >&2
        exit 1
    }
}

make_archive agent-statusline
mkdir -p "$HOME/.config/agent-statusline"
printf 'old receipt' > "$HOME/.config/agent-statusline/agent-statusline-receipt.json"
printf 'old env helper' > "$HOME/.config/agent-statusline/env"
printf '. "$HOME/.config/agent-statusline/env"\n' > "$HOME/.profile"
run_case valid 'new binary'
install_dir="$temp/install-latest"
mkdir -p "$install_dir"
printf 'old binary' > "$install_dir/agent-statusline"
sh "$installer" --install-dir "$install_dir"
[ "$(cat "$install_dir/agent-statusline")" = 'new binary' ]
[ "$(cat "$HOME/.config/agent-statusline/agent-statusline-receipt.json")" = 'old receipt' ]
[ "$(cat "$HOME/.config/agent-statusline/env")" = 'old env helper' ]
grep -F '. "$HOME/.config/agent-statusline/env"' "$HOME/.profile" >/dev/null
printf '%064d *%s\n' 0 "$archive" > "$FIXTURE_DIR/$archive.sha256"
run_case mismatch 'old binary'
rm "$FIXTURE_DIR/$archive.sha256"
run_case missing-checksum 'old binary'
make_archive README.md
run_case missing-binary 'old binary'

cat > "$temp/mock-bin/uname" <<'MOCK'
#!/bin/sh
case "$1" in
    -s) echo Linux ;;
    -m) echo aarch64 ;;
esac
MOCK
chmod +x "$temp/mock-bin/uname"
archive=agent-statusline-aarch64-unknown-linux-musl.zip
make_archive agent-statusline
run_case aarch64 'new binary'

cat > "$temp/mock-bin/uname" <<'MOCK'
#!/bin/sh
case "$1" in
    -s) echo Darwin ;;
    -m) echo x86_64 ;;
esac
MOCK
chmod +x "$temp/mock-bin/uname"
if sh "$installer" --install-dir "$temp/install-unsupported"; then
    echo 'Unsupported target was accepted' >&2
    exit 1
fi
[ ! -e "$temp/install-unsupported/agent-statusline" ]
echo 'Linux installer tests passed.'
