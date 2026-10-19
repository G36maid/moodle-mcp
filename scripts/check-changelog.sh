#!/bin/sh
# A versioned section with no body extracts as blank release notes and
# fails the release job — catch the empty-header structure at commit
# time. [Unreleased] may legitimately be empty.
fail=0
for f in "$@"; do
  awk -v file="$f" '
    /^## \[/ {
      if (prev_is_header && prev !~ /^## \[Unreleased\]$/) {
        printf "%s: empty section before this line: %s\n", file, prev
        bad = 1
      }
      prev = $0; prev_is_header = 1; next
    }
    /[^ \t]/ { prev_is_header = 0 }
    END {
      if (prev_is_header && prev !~ /^## \[Unreleased\]$/) {
        printf "%s: empty section at end of file: %s\n", file, prev
        bad = 1
      }
      exit bad
    }
  ' "$f" || fail=1
done
exit $fail
