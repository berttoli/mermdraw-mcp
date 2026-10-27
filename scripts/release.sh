#!/usr/bin/env bash
# Release helpers used by .github/workflows/release.yml; they run the same way locally. Addons
# (berttoli/mermdraw-NAME) use this script unchanged: names come from Cargo.toml and the binary.
#
#   scripts/release.sh cargo-version          print the version in Cargo.toml
#   scripts/release.sh check TAG              validate TAG (vX.Y.Z or vX.Y.Z-rc.N) against Cargo.toml (must
#                                             match exactly, so rc builds report their rc version),
#                                             Cargo.lock and CHANGELOG.md; print version=/prerelease=
#   scripts/release.sh notes VERSION          print the CHANGELOG.md section for VERSION
#   scripts/release.sh package TARGET BIN OUT write OUT/NAME-TARGET.tar.gz (.zip for Windows), NAME being
#                                             BIN's file name: mermdraw-TARGET, mermdraw-mcp-TARGET, …
#   scripts/release.sh verify ARCHIVE         unpack ARCHIVE and smoke-test the binary inside
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export COPYFILE_DISABLE=1 # keep macOS metadata (._ files) out of archives

die() {
  echo "release: $*" >&2
  exit 1
}

cargo_version() {
  sed -n 's/^version = "\(.*\)"$/\1/p' "$ROOT/Cargo.toml" | head -1
}

package_name() {
  sed -n 's/^name = "\(.*\)"$/\1/p' "$ROOT/Cargo.toml" | head -1
}

lock_version() {
  awk -v n="name = \"$(package_name)\"" '$0 == n { found = 1; next } found && /^version = / { gsub(/"/, "", $3); print $3; exit }' "$ROOT/Cargo.lock"
}

cmd_check() {
  local tag="${1:?usage: release.sh check TAG}"
  [[ "$tag" =~ ^v([0-9]+\.[0-9]+\.[0-9]+)(-[0-9A-Za-z.-]+)?$ ]] || die "tag '$tag' is not vMAJOR.MINOR.PATCH[-PRERELEASE]"
  local version="${tag#v}" base="${BASH_REMATCH[1]}" pre="${BASH_REMATCH[2]}"
  local cargo lock
  cargo="$(cargo_version)"
  lock="$(lock_version)"
  # exact match: `mermdraw --version` of a v0.2.0-rc.1 build must say 0.2.0-rc.1
  [[ "$cargo" == "$version" ]] || die "Cargo.toml says $cargo but the tag is $tag (set version = \"$version\")"
  [[ "$lock" == "$cargo" ]] || die "Cargo.lock says $lock but Cargo.toml says $cargo (run cargo build)"
  grep -Eq "^## ${base//./\\.}( |\$)" "$ROOT/CHANGELOG.md" || die "CHANGELOG.md has no '## $base' section"
  local prerelease=false
  if [[ -n "$pre" ]]; then prerelease=true; fi
  echo "version=$version"
  echo "prerelease=$prerelease"
}

cmd_notes() {
  local version="${1:?usage: release.sh notes VERSION}"
  local base="${version%%-*}"
  if [[ "$version" != "$base" ]]; then
    printf '> Release candidate for %s. Try it and report problems before the stable release.\n\n' "$base"
  fi
  awk -v v="$base" '/^## /{ on = index($0, "## " v " ") == 1 || $0 == "## " v; next } on' "$ROOT/CHANGELOG.md" |
    sed -e '/./,$!d' # drop leading blank lines
}

cmd_package() {
  local target="${1:?usage: release.sh package TARGET BIN OUT}" bin="${2:?}" out="${3:?}"
  [[ -f "$bin" ]] || die "no binary at $bin"
  local exe name stage
  exe="$(basename "$bin")"
  name="${exe%.exe}-$target"
  stage="$(mktemp -d)"
  mkdir -p "$stage/$name" "$out"
  out="$(cd "$out" && pwd)"
  cp "$bin" "$stage/$name/"
  cp "$ROOT/README.md" "$ROOT/LICENSE" "$ROOT/CHANGELOG.md" "$stage/$name/"
  if [[ "$target" == *windows* ]]; then
    rm -f "$out/$name.zip"
    if command -v 7z >/dev/null; then
      (cd "$stage" && 7z a -tzip "$out/$name.zip" "$name" >/dev/null)
    else
      (cd "$stage" && zip -qr "$out/$name.zip" "$name")
    fi
    echo "$out/$name.zip"
  else
    tar -C "$stage" -czf "$out/$name.tar.gz" "$name"
    echo "$out/$name.tar.gz"
  fi
  rm -rf "$stage"
}

cmd_verify() {
  local archive="${1:?usage: release.sh verify ARCHIVE}"
  [[ -f "$archive" ]] || die "no archive at $archive"
  local file name dir
  file="$(basename "$archive")"
  name="${file%.zip}"
  name="${name%.tar.gz}"
  local exe # the binary's name: NAME-TARGET minus the target
  exe="$(sed -E 's/-(x86_64|aarch64|i686|armv7|riscv64gc)-.*$//' <<<"$name")"
  [[ "$exe" != "$name" ]] || die "no target in the archive name $file"
  dir="$(mktemp -d)"
  case "$file" in
    *.zip) if command -v unzip >/dev/null; then unzip -q "$archive" -d "$dir"; else 7z x -o"$dir" "$archive" >/dev/null; fi ;;
    *.tar.gz) tar -xzf "$archive" -C "$dir" ;;
    *) die "unknown archive type: $file" ;;
  esac

  local unexpected
  unexpected="$( (cd "$dir" && find . -mindepth 1) | grep -vE "^\./$name(/($exe|$exe\.exe|README\.md|LICENSE|CHANGELOG\.md))?\$" || true)"
  [[ -z "$unexpected" ]] || die "unexpected files in $file: $unexpected"
  for f in README.md LICENSE CHANGELOG.md; do
    [[ -f "$dir/$name/$f" ]] || die "$f missing from $file"
  done
  local bin="$dir/$name/$exe"
  [[ -f "$bin.exe" ]] && bin="$bin.exe"
  [[ -f "$bin" ]] || die "no $exe binary in $file"
  echo "ok   $file contains the binary, README, LICENSE and CHANGELOG"

  if [[ "$name" == *linux-musl* ]]; then
    local kind
    kind="$(file -b "$bin")"
    [[ "$kind" == *"statically linked"* || "$kind" == *"static-pie linked"* ]] || die "$file is not statically linked: $kind"
    echo "ok   statically linked"
  fi

  if [[ "$name" == *x86_64-apple-darwin* && "$(uname -m)" == arm64 ]] && ! arch -x86_64 /usr/bin/true 2>/dev/null; then
    echo "::warning::Rosetta 2 is unavailable, so the x86_64 macOS binary was packaged but not run"
    return
  fi

  local expected out
  expected="$exe $(cargo_version)"
  out="$("$bin" --version)"
  [[ "$out" == "$expected" ]] || die "--version printed '$out', expected '$expected'"
  echo "ok   $out"

  if [[ "$bin" == *.exe ]]; then
    if [[ "$exe" == mermdraw ]]; then
      out="$("$bin" ls "$ROOT/examples/architecture.md")"
      [[ "$out" == *"3 diagrams"* ]] || die "mermdraw ls failed: $out"
      echo "ok   mermdraw ls"
    fi
  else
    "$ROOT/tests/smoke.sh" "$bin"
  fi
  rm -rf "$dir"
}

cmd="${1:-}"
shift || true
case "$cmd" in
  cargo-version) cargo_version ;;
  check) cmd_check "$@" ;;
  notes) cmd_notes "$@" ;;
  package) cmd_package "$@" ;;
  verify) cmd_verify "$@" ;;
  *) die "usage: release.sh cargo-version | check TAG | notes VERSION | package TARGET BIN OUT | verify ARCHIVE" ;;
esac
