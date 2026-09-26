#!/bin/sh
set -eu

tag=latest
install_dir=${HOME:?HOME is required}/.local/bin

while [ "$#" -gt 0 ]; do
    case "$1" in
        --tag|--install-dir)
            [ "$#" -ge 2 ] || { echo "Missing value for $1" >&2; exit 2; }
            case "$1" in
                --tag) tag=$2 ;;
                --install-dir) install_dir=$2 ;;
            esac
            shift 2
            ;;
        --help)
            echo "Usage: agent-statusline-installer.sh [--tag TAG] [--install-dir DIR]"
            exit 0
            ;;
        *) echo "Unknown option: $1" >&2; exit 2 ;;
    esac
done

case "$(uname -s)/$(uname -m)" in
    Linux/x86_64) target=x86_64-unknown-linux-musl ;;
    Linux/aarch64) target=aarch64-unknown-linux-musl ;;
    *) echo "Unsupported OS/architecture: $(uname -s)/$(uname -m)" >&2; exit 1 ;;
esac

archive_name="agent-statusline-$target.zip"
if [ "$tag" = latest ]; then
    release_url=$(curl -fLsS -o /dev/null -w '%{url_effective}' \
        'https://github.com/scottlz0310/agent-statusline/releases/latest')
    case "$release_url" in
        https://github.com/scottlz0310/agent-statusline/releases/tag/*)
            tag=${release_url##*/}
            ;;
        *) echo "Could not resolve the latest Release tag" >&2; exit 1 ;;
    esac
fi
case "$tag" in
    *[!a-zA-Z0-9._-]*|'') echo "Invalid tag: $tag" >&2; exit 2 ;;
esac
base_url="https://github.com/scottlz0310/agent-statusline/releases/download/$tag"

temp_dir=$(mktemp -d)
stage=
cleanup() {
    [ -z "$stage" ] || rm -f -- "$stage"
    rm -rf -- "$temp_dir"
}
trap cleanup EXIT HUP INT TERM

curl -fLsS "$base_url/$archive_name" -o "$temp_dir/$archive_name"
curl -fLsS "$base_url/$archive_name.sha256" -o "$temp_dir/$archive_name.sha256"

expected=$(awk -v name="$archive_name" '
    NF == 2 && length($1) == 64 && ($2 == name || $2 == "*" name) {
        if ($1 ~ /^[0-9a-fA-F]+$/) { print tolower($1) }
    }
' "$temp_dir/$archive_name.sha256")
[ "$(printf '%s' "$expected" | wc -c | tr -d ' ')" -eq 64 ] || {
    echo "Invalid checksum for $archive_name" >&2
    exit 1
}
actual=$(sha256sum "$temp_dir/$archive_name" | awk '{ print $1 }')
[ "$actual" = "$expected" ] || {
    echo "SHA-256 mismatch for $archive_name" >&2
    exit 1
}

entry=$(unzip -Z1 "$temp_dir/$archive_name" | awk '
    $0 == "agent-statusline" { print; found = 1; exit }
    /^[^/]+\/agent-statusline$/ { nested = $0 }
    END { if (!found && nested != "") print nested }
')
[ -n "$entry" ] || { echo "agent-statusline not found in $archive_name" >&2; exit 1; }

mkdir -p -- "$install_dir"
stage=$(mktemp "$install_dir/.agent-statusline.XXXXXX")
unzip -p "$temp_dir/$archive_name" "$entry" > "$stage"
chmod 755 "$stage"

destination="$install_dir/agent-statusline"
backup="$install_dir/agent-statusline.old"
if [ -e "$destination" ]; then
    rm -f -- "$backup"
    mv -- "$destination" "$backup"
fi
if ! mv -- "$stage" "$destination"; then
    if [ -e "$backup" ] && [ ! -e "$destination" ]; then
        mv -- "$backup" "$destination"
    fi
    echo "Failed to replace $destination" >&2
    exit 1
fi
stage=

echo "Installed $destination"
case ":$PATH:" in
    *":$install_dir:"*) ;;
    *) echo "Add $install_dir to PATH to run agent-statusline." ;;
esac
