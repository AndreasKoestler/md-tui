#!/usr/bin/env bash
# Open mdt (md-tui) on a markdown file inside a tmux popup and surface its
# Sidemark YAML dump to the outer shell after the popup closes.
#
# mdt writes both its ratatui TUI and its final YAML dump to stdout, so
# redirecting stdout in the popup would blank the TUI. Instead we set
# MDT_DUMP_PATH so mdt writes the YAML to a file on clean exit; once the
# popup closes we cat that file.
#
# Usage:
#   mdt-popup.sh <file> [username]
#   mdt-popup.sh <file> [-u <username>]
set -euo pipefail

prog=${0##*/}

usage() {
    printf 'usage: %s <file> [username]\n       %s <file> [-u <username>]\n' \
        "$prog" "$prog" >&2
    exit 2
}

[[ $# -ge 1 ]] || usage
file=$1; shift

username=""
case ${1-} in
    -u|--username) [[ $# -ge 2 ]] || usage; username=$2 ;;
    "") ;;
    *)             username=$1 ;;
esac

[[ -f $file ]]      || { printf '%s: no such file: %s\n' "$prog" "$file" >&2; exit 1; }
[[ -n ${TMUX-} ]]   || { printf '%s: must be run inside a tmux session\n' "$prog" >&2; exit 1; }
command -v mdt >/dev/null \
                    || { printf '%s: mdt not on PATH\n' "$prog" >&2; exit 127; }

abs_file=$(cd -- "$(dirname -- "$file")" && pwd)/$(basename -- "$file")
dump=$(mktemp -t mdt-dump.XXXXXX)
trap 'rm -f -- "$dump"' EXIT

# Inline env var + %q-quoted args: tmux server scrubs the environment, so
# MDT_DUMP_PATH has to live inside the shell-command string itself.
cmd=$(printf 'MDT_DUMP_PATH=%q mdt %q' "$dump" "$abs_file")
[[ -n $username ]] && cmd+=$(printf ' -u %q' "$username")

tmux popup -E -w 90% -h 90% "$cmd"

[[ -s $dump ]] && cat -- "$dump"
