#!/usr/bin/env bash
# =============================================================================
# CHEAT AUDIT
#
# Sumber kanonik: ~/Workspace/scripts/audit-cheats.sh
# (salinan di-vendor ke tiap repo agar CI bisa menegakkan aturan ini) — deteksi cara-cara bikin compile gate ngapalin kegagalan.
# -----------------------------------------------------------------------------
# v3:
#   - Scope file: hanya file yang bisa memengaruhi gate (Cargo.toml, .cargo/**,
#     CI workflow, script, *.sh, Makefile, justfile, Dockerfile*, *.yml|yaml).
#     Source .rs diaudit terpisah, hanya untuk pola Rust-specific.
#   - Pola regex & label dipisah array (bukan string dengan delimiter), karena
#     regex memakai `|` untuk alternation dan delimiter memotongnya.
#   - `unsafe` hanya boleh dengan `#[expect(unsafe_code, reason = ...)]`.
#
# Output: file:line:match   (exit 0 bersih, exit 1 ada cheat)
# =============================================================================
set -uo pipefail

PROJ="${1:?usage: audit-cheats.sh <project-dir>}"
cd "$PROJ" || { echo "FAIL: cd $PROJ"; exit 1; }

EXCLUDES=(
  --exclude-dir=target --exclude-dir=vendor --exclude-dir=.git
  --exclude-dir=node_modules --exclude-dir=.venv --exclude-dir=.tools
  --exclude-dir=.cache --exclude-dir=downloads --exclude-dir=dist
  --exclude-dir=build --exclude-dir=.cargo-checkouts --exclude-dir=.idea
  --exclude-dir='mutants.out*' --exclude-dir=log --exclude-dir=logs
  --exclude-dir=fixtures --exclude-dir=testdata
)
# Audit ini mendeteksi dirinya sendiri: pola cheat (`|| true`, `-A warnings`,
# `RUSTFLAGS`, ...) muncul sebagai literal di dalam file ini. Filenya difilter
# dari hasil grep (bukan lewat --exclude, karenaampur `--include` + `--exclude`
# file-pattern membuat GNU grep mengabaikan `--include`).
SELF_FILTER="audit-cheats\.sh"
# CATATAN: jangan tambah `--exclude=<file-pattern>` di sini. GNU grep 3.12
# mengabaikan `--include` kalau `--exclude` file-pattern ikut dipakai; audit
# lalu memindai semua file dan memunculkan false positive.

GATE_FILES=(
  --include='Cargo.toml' --include='rust-toolchain.toml'
  --include='config.toml' --include='Makefile' --include='justfile'
  --include='*.sh' --include='*.bash' --include='Dockerfile*'
  --include='*.yml' --include='*.yaml' --include='*.toml'
)

found=0
emit() { printf '%s   [%s]\n' "$1" "$2"; found=$((found + 1)); }

# --- Pola build/CI: regex, lalu label (array paralel) ----------------------
PATTERNS=(
  '\|\|[[:space:]]*true'
  '\|\|[[:space:]]*echo'
  'cargo[^\n]*\b(clippy|check|build|test)\b[^\n]*--no-deps'
  'cargo[^\n]*--no-deps[^\n]*\b(clippy|check|build|test)\b'
  '\-A[[:space:]]*warnings'
  '--cap-lints'
  'RUSTFLAGS[=:]'
  'continue-on-error'
  'cargo[^\n]*\|\|'
  'cargo[^\n]*2>/dev/null'
  'cargo[^\n]*--quiet'
)
LABELS=(
  'mask-exit'
  'mask-exit'
  'skip-dep-lint'
  'skip-dep-lint'
  'allow-warnings'
  'cap-lints'
  'rustflags-override'
  'ci-ignore-failure'
  'masked-cargo'
  'suppress-cargo-stderr'
  'cargo-quiet'
)

for i in "${!PATTERNS[@]}"; do
  re="${PATTERNS[$i]}"; why="${LABELS[$i]}"
  while IFS= read -r hit; do
    [ -z "$hit" ] && continue
    emit "$hit" "$why"
  done < <(grep -rnE --binary-files=without-match "${EXCLUDES[@]}" \
      "${GATE_FILES[@]}" -- "$re" . 2>/dev/null | grep -v "$SELF_FILTER" | head -30)
done

# --- Pola Rust-specific di source .rs --------------------------------------
RS_PATTERNS=(
  '#!\[allow\('
  '#\[allow\(clippy::'
  'clippy::allow'
)
RS_LABELS=(
  'crate-level-blanket-allow'
  'per-lint-allow'
  'clippy-allow'
)
for i in "${!RS_PATTERNS[@]}"; do
  re="${RS_PATTERNS[$i]}"; why="${RS_LABELS[$i]}"
  while IFS= read -r hit; do
    [ -z "$hit" ] && continue
    emit "$hit" "$why"
  done < <(grep -rnE --binary-files=without-match "${EXCLUDES[@]}" \
      --include='*.rs' -- "$re" . 2>/dev/null | grep -v "$SELF_FILTER" | head -30)
done

# --- unsafe block tanpa justifikasi ---------------------------------------
# `unsafe` itu boleh, tapi WAJIB ada `#[expect(unsafe_code, reason = ...)]` atau
# `#[allow(unsafe_code)]` di baris sebelumnya. Tanpa itu = tak terjustifikasi.
while IFS= read -r rsfile; do
  awk -v file="$rsfile" '
    {
      # POSIX awk tidak punya \b (word boundary) — pakai batas manual.
      if ($0 ~ /(^|[^_[:alnum:]])unsafe[[:space:]]*\{/) {
        if (!justified) {
          printf "%s:%d:%s   [unsafe-block-tanpa-ekspektasi]\n", file, FNR, $0
          n++
        }
      }
      justified = ($0 ~ /#\[(expect|allow)\(unsafe_code/) ? 1 : 0
    }
    END { exit (n > 0 ? 1 : 0) }
  ' "$rsfile"
  rc=$?
  if [ $rc -ne 0 ]; then
    found=$((found + 1))
  fi
done < <(find . \( -name target -o -name vendor -o -name .git -o -name node_modules \
             -o -name .tools -o -name 'mutants.out*' -o -name .venv \) -prune -o \
             -name '*.rs' -print 2>/dev/null)

# --- Konfigurasi cargo yang menimpa lint secara global ----------------------
for cfg in .cargo/config.toml .cargo/config; do
  [ -f "$cfg" ] || continue
  if grep -qE '^\s*rustflags|build\.rustflags' "$cfg" 2>/dev/null; then
    emit "$cfg" "global-rustflags-override"
  fi
  if grep -qE '^\s*(ci|check|build|verify)\s*=' "$cfg" 2>/dev/null; then
    # alias yang isinya nutupin error
    if grep -A2 -E '^\s*(ci|check|build|verify)\s*=' "$cfg" 2>/dev/null \
        | grep -qE '\|\||--no-deps|-A[[:space:]]*warnings'; then
      emit "$cfg" "check-alias-for-masking"
    fi
  fi
done

# --- [lints] di Cargo.toml: kalau clippy all = warn, itu celah ------------
if [ -f Cargo.toml ]; then
  if grep -qE '^\s*all[[:space:]]*=[[:space:]]*"warn"' Cargo.toml 2>/dev/null; then
    if grep -qE '^\[lints\.clippy\]' Cargo.toml 2>/dev/null; then
      emit "Cargo.toml" "lints-clippy-all-warn-not-deny"
    fi
  fi
  if grep -qE '^\s*unsafe_code[[:space:]]*=[[:space:]]*"(allow|warn)"' Cargo.toml 2>/dev/null; then
    emit "Cargo.toml" "unsafe-code-not-forbidden"
  fi
  if grep -qE '^\s*license\s*=' Cargo.toml 2>/dev/null; then :; else
    emit "Cargo.toml" "no-license-field"
  fi
fi

# --- CI workflow: apakah CI beneran fail saat gate gagal? ------------------
for wf in .github/workflows/*.y*ml; do
  [ -f "$wf" ] || continue
  if grep -qE 'cargo (check|build|test|clippy)' "$wf" 2>/dev/null; then
    if grep -qE 'continue-on-error|allow-failure|\|\|[[:space:]]*true' "$wf" 2>/dev/null; then
      emit "$wf" "ci-can-green-on-failure"
    fi
  fi
done

if [ "$found" -gt 0 ]; then
  printf 'CHEATS: %d\n' "$found"
  exit 1
fi
printf 'CHEATS: 0 (bersih)\n'
exit 0
