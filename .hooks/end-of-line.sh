#! /bin/sh
# A hook script to ensure files have proper end-of-line characters.
# This script checks for files staged for commit and ensures they end with a newline character.
# If a file does not end with a newline, it appends one.

# Get the list of staged files
STAGED_FILES=$(git diff --cached --name-only)

# Loop through each staged file
for FILE in $STAGED_FILES; do
    # Check if the file exists and is a regular file
    if [ -f "$FILE" ]; then
        # Check if the last character of the file is a newline
        if [ -n "$(tail -c1 "$FILE" | tr -d '\n')" ]; then
            echo "Fixing end-of-line for $FILE"
            # Append a newline character to the file
            echo "" >> "$FILE"
            # Stage the modified file
            git add "$FILE"
        fi
    fi
done

echo "[End-of-Line Fixer] All done!"

exit
