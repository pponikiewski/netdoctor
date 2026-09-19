#!/bin/bash
INPUT=$(cat)

# Precise patterns (ERE). Note: 'rm -rf /tmp/...' is ALLOWED — only root/home wipes are blocked.
BLOCKED_PATTERNS=(
  'rm[[:space:]]+-[a-zA-Z]*rf?[a-zA-Z]*[[:space:]]+/([[:space:]"]|$)'
  'rm[[:space:]]+-[a-zA-Z]*rf?[a-zA-Z]*[[:space:]]+~'
  'DROP[[:space:]]+(TABLE|DATABASE|SCHEMA)'
  'TRUNCATE[[:space:]]'
  'git[[:space:]]+push[[:space:]]+(-f|--force)([[:space:])"]|$)'
  'git[[:space:]]+reset[[:space:]]+--hard[[:space:]]+origin'
)

for pattern in "${BLOCKED_PATTERNS[@]}"; do
  if echo "$INPUT" | grep -qiE "$pattern"; then
    echo "Blocked by hook: dangerous pattern matched ($pattern). Ask the user to run this manually if truly needed." >&2
    exit 2
  fi
done

exit 0
