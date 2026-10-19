#!/bin/bash
# Prepend YAML front matter to generated documentation files.
#
# Usage: add-frontmatter.sh [options] <file>
#   --title <title>         Title for the front matter (required)
#   --icon <icon>           Icon for the front matter
#   --description <desc>    Description for the front matter
#   --output <path>         Output path (defaults to overwriting input file)

set -euo pipefail

TITLE=""
ICON=""
DESCRIPTION=""
OUTPUT=""
FILE=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --title) TITLE="$2"; shift 2 ;;
    --icon) ICON="$2"; shift 2 ;;
    --description) DESCRIPTION="$2"; shift 2 ;;
    --output) OUTPUT="$2"; shift 2 ;;
    *)
      if [[ -z "$FILE" ]]; then
        FILE="$1"
      else
        echo "Error: unexpected argument '$1'" >&2
        exit 1
      fi
      shift
      ;;
  esac
done

if [[ -z "$FILE" ]]; then
  echo "Error: input file is required" >&2
  echo "Usage: add-frontmatter.sh --title <title> [--icon <icon>] [--description <desc>] [--output <path>] <file>" >&2
  exit 1
fi

if [[ -z "$TITLE" ]]; then
  echo "Error: --title is required" >&2
  exit 1
fi

if [[ ! -f "$FILE" ]]; then
  echo "Error: file '$FILE' not found" >&2
  exit 1
fi

OUTPUT="${OUTPUT:-$FILE}"

# Ensure output directory exists
mkdir -p "$(dirname "$OUTPUT")"

# Build front matter
{
  echo "---"
  echo "title: ${TITLE}"
  [[ -n "$ICON" ]] && echo "icon: ${ICON}"
  [[ -n "$DESCRIPTION" ]] && echo "description: ${DESCRIPTION}"
  echo "---"
  echo ""
  cat "$FILE"
} > "${OUTPUT}.tmp"

mv "${OUTPUT}.tmp" "$OUTPUT"

# Clean up input file if output is different
if [[ "$OUTPUT" != "$FILE" ]]; then
  rm -f "$FILE"
fi

echo "Front matter added to ${OUTPUT}"
