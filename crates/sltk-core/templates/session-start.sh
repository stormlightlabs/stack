#!/usr/bin/env bash
# Prepares a Claude Code cloud session: installs the sltk release that
# .github/sltk.toml pins, runs `sltk install --yes` to install fab and arcl,
# then builds the local arcl database from .arcl/workspace when the clone has
# none. Local sessions are left alone; run `sltk install` there.
#
# sltk init writes this file, and it is the same in every repository. The
# version and the archive hashes live only in .github/sltk.toml.
set -euo pipefail

if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi
cd "$CLAUDE_PROJECT_DIR"

fail() {
  echo "session-start: $*" >&2
  exit 1
}

pin=.github/sltk.toml
if [ ! -f "$pin" ]; then
  fail "$pin is missing; run sltk init"
fi
version=$(sed -n 's/^version = "\([0-9][0-9.]*\)"$/\1/p' "$pin")
if [ -z "$version" ] || [[ $version == *$'\n'* ]]; then
  fail "$pin needs one line that reads version = \"x.y.z\""
fi

platform=$(uname -sm)
case "$platform" in
  "Darwin arm64") target=aarch64-apple-darwin ;;
  "Linux x86_64") target=x86_64-unknown-linux-gnu ;;
  *) fail "sltk has no build for $platform; it supports Darwin arm64 and Linux x86_64" ;;
esac
want=$(sed -n "s/^sha256-$target = \"\([0-9a-f]\{64\}\)\"$/\1/p" "$pin")
if [ -z "$want" ] || [[ $want == *$'\n'* ]]; then
  fail "$pin needs one line that reads sha256-$target = \"<sha256>\""
fi

# sltk goes in a directory of its own, named for its version, so putting it
# on PATH moves nothing else ahead of the image's tools.
bin="$HOME/.local/share/sltk/sltk-$version/bin"
sltk=$bin/sltk
if [ "$("$sltk" --version 2>/dev/null)" != "sltk $version" ]; then
  if command -v sha256sum >/dev/null 2>&1; then
    sha256() { sha256sum "$1"; }
  elif command -v shasum >/dev/null 2>&1; then
    sha256() { shasum -a 256 "$1"; }
  else
    fail "found neither sha256sum nor shasum on PATH, so the sltk archive cannot be checked"
  fi
  name=sltk-cli-$target
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  archive=$tmp/$name.tar.xz
  # SLTK_HOOK_ARCHIVE names a local archive to use in place of the download,
  # for tests. The hash check below applies to it all the same.
  if [ -n "${SLTK_HOOK_ARCHIVE:-}" ]; then
    cp "$SLTK_HOOK_ARCHIVE" "$archive"
  else
    curl --proto '=https' --tlsv1.2 -fsSL --connect-timeout 10 --max-time 120 \
      -o "$archive" \
      "https://github.com/stormlightlabs/stack/releases/download/v$version/$name.tar.xz"
  fi
  # dist's installer script is never run: nothing pins a hash for it.
  got=$(sha256 "$archive")
  got=${got%% *}
  if [ "$got" != "$want" ]; then
    fail "$name.tar.xz has sha256 $got, but $pin pins $want"
  fi
  tar -xJf "$archive" -C "$tmp" "$name/sltk"
  mkdir -p "$bin"
  chmod 755 "$tmp/$name/sltk"
  mv -f "$tmp/$name/sltk" "$sltk"
fi

# sltk installs fab and arcl where dist's installers put them.
cargo_bin=${CARGO_HOME:-$HOME/.cargo}/bin
path_line="export PATH=\"$bin:$cargo_bin:\$PATH\""
if [ -n "${CLAUDE_ENV_FILE:-}" ] && ! grep -qxF "$path_line" "$CLAUDE_ENV_FILE" 2>/dev/null; then
  echo "$path_line" >> "$CLAUDE_ENV_FILE"
fi
"$sltk" install --yes

# The database is local and ignored by git. Build it once from the committed
# workspace; a resumed session keeps the one it has. arcl init refuses a file
# that is not a database, such as one an interrupted init left, so a database
# that fails to open moves aside under a name .arcl/.gitignore ignores, and
# init builds a new one. The workspace in git holds the tasks either way.
arcl=$cargo_bin/arcl
broken=
if [ -e .arcl/arcl.db ] && ! "$arcl" tree >/dev/null 2>&1; then
  broken=.arcl/arcl.db-broken-$(date +%Y%m%dT%H%M%S)
  for f in .arcl/arcl.db .arcl/arcl.db-wal .arcl/arcl.db-shm; do
    if [ -e "$f" ]; then
      mv "$f" "$broken${f#.arcl/arcl.db}"
    fi
  done
fi
if [ ! -e .arcl/arcl.db ]; then
  if ! "$arcl" init --quiet; then
    if [ -n "$broken" ]; then
      fail ".arcl/arcl.db did not open and is now $broken; arcl init could not build a new one"
    fi
    fail "arcl init could not build .arcl/arcl.db"
  fi
  if [ -n "$broken" ]; then
    echo "session-start: .arcl/arcl.db did not open; moved it to $broken and rebuilt it" >&2
  fi
fi
"$arcl" --version
