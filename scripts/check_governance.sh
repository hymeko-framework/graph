#!/usr/bin/env bash
# Verify this repository's governance: the shared V&V architecture block, and the ownership
# declaration that makes `graph` canonical.
#
# WHY IT READS COMMITTED BLOBS, NOT THE WORKING TREE. A gate that inspects the working tree passes
# on uncommitted edits and says nothing about what a fresh clone gets. `tensor/main` and
# `nagare/main` carry no CLAUDE.md or CORE.YAML at all while their feature branches do -- a state no
# working-tree check would ever have reported. This reads `<rev>:<path>`, so what it certifies is
# what someone actually receives.
#
# Exit codes
#   0  every check ran and passed, and a receipt was printed
#   1  a check ran and failed
#   2  the check could not run -- missing rev, unreadable blob, no reference block
#
# "Could not run" is never 0. That distinction is the one `check_contract_sync.sh` failed to make
# for eleven days, being green both when it found drift and when it could not execute at all.
set -uo pipefail

REV="${1:-HEAD}"
REF_FILE="${2:-}"
BEGIN='<!-- BEGIN SHARED V&V ARCHITECTURE -->'
END='<!-- END SHARED V&V ARCHITECTURE -->'

die() { echo "governance: $*" >&2; exit 2; }
CHECKS=0; FAIL=0
ok()  { CHECKS=$((CHECKS+1)); printf '  ok    %s\n' "$*"; }
bad() { CHECKS=$((CHECKS+1)); FAIL=$((FAIL+1)); printf '  FAIL  %s\n' "$*"; }

git rev-parse --verify -q "$REV" >/dev/null || die "rev '$REV' does not resolve"

blob() { git show "$REV:$1" 2>/dev/null; }

# --- the two governance files must exist AT THE REV, not merely on disk --------------------------
for f in CLAUDE.md CORE.YAML; do
  if blob "$f" >/dev/null 2>&1; then ok "$f present at $REV"; else bad "$f ABSENT at $REV -- a clone of this rev gets no governance"; fi
done

# --- the V&V block: exactly one pair, non-empty --------------------------------------------------
extract_vv() {
  local src="$1" nb ne
  nb=$(grep -cF "$BEGIN" <<<"$src"); ne=$(grep -cF "$END" <<<"$src")
  [[ "$nb" == 1 ]] || { echo "expected 1 BEGIN marker, found $nb" >&2; return 1; }
  [[ "$ne" == 1 ]] || { echo "expected 1 END marker, found $ne" >&2; return 1; }
  local out; out=$(awk -v b="$BEGIN" -v e="$END" 'index($0,b){f=1;next} index($0,e){f=0} f' <<<"$src")
  [[ -n "${out//[[:space:]]/}" ]] || { echo "the V&V block is empty" >&2; return 1; }
  printf '%s\n' "$out"
}

CL="$(blob CLAUDE.md || true)"
if [[ -z "$CL" ]]; then
  bad "CLAUDE.md unreadable at $REV; the V&V block cannot be checked"
else
  if VV="$(extract_vv "$CL")"; then
    ok "V&V block well-formed ($(wc -l <<<"$VV") lines, sha $(sha256sum <<<"$VV" | cut -c1-12))"
    if [[ -n "$REF_FILE" ]]; then
      [[ -f "$REF_FILE" ]] || die "reference file $REF_FILE not found"
      REF="$(extract_vv "$(cat "$REF_FILE")")" || die "the reference file has no well-formed V&V block"
      if [[ "$VV" == "$REF" ]]; then ok "V&V block byte-identical to $REF_FILE"
      else bad "V&V block DIFFERS from $REF_FILE"; fi
    fi
  else
    bad "V&V block missing, duplicated or empty at $REV"
  fi
fi

# --- ownership declaration ----------------------------------------------------------------------
CY="$(blob CORE.YAML || true)"
if [[ -z "$CY" ]]; then
  bad "CORE.YAML unreadable at $REV"
else
  grep -q '^  - hymeko_graph$'  <<<"$CY" && ok "CORE.YAML declares hymeko_graph core"  || bad "CORE.YAML does not declare hymeko_graph core"
  grep -q '^  - hymeko_pgraph$' <<<"$CY" && ok "CORE.YAML declares hymeko_pgraph core" || bad "CORE.YAML does not declare hymeko_pgraph core"
  grep -q 'Friedler outside hymeko_pgraph' <<<"$CY" && ok "Friedler ownership stated" || bad "Friedler ownership not stated"
fi
grep -q 'may not vendor it' <<<"$CL" && ok "vendoring prohibition stated" || bad "vendoring prohibition not stated"
grep -q 'owns no compute IR and no backend' <<<"$CL" && ok "IR/backend ownership disclaimed to core" || bad "IR/backend disclaimer missing"

echo "──────────────────────────────────────"
[[ "$CHECKS" -gt 0 ]] || die "no check executed; refusing to report health"
printf 'GOVERNANCE-CHECK-RAN rev=%s checks=%d failures=%d\n' "$(git rev-parse --short "$REV")" "$CHECKS" "$FAIL"
[[ "$FAIL" -eq 0 ]] || exit 1
exit 0
