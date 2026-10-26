#!/usr/bin/env bash
# Frees what RSharp's builds and test runs leave behind. It only prints the plan unless --yes is given.
#
#   tools/clean.sh                    what a safe clean would remove
#   tools/clean.sh --yes              remove it: loose object files, test folders and old run output
#   tools/clean.sh --yes --deep       also cargo clean, and Passenger's sidecar build output
#   tools/clean.sh --yes --temp       also stale rsharp-* folders in the system temp folder
#   tools/clean.sh --yes --worktrees  also .claude/worktrees registered with git and left behind
#   tools/clean.sh --yes --all        all of the above, and the reviews' screenshots and test logs
#
# The safe clean keeps the build: rustc leaves one .rcgu.o per codegen unit beside every crate it
# compiles, and Cargo never reads them again -- a changed crate is compiled afresh and an unchanged one
# is not relinked. They are the bulk of target/. Cargo's fingerprints, rlibs, binaries, build-script
# output and sidecars stay, so the next build is incremental.
#
# target/template-tests is the live template tests' local feed, made again by
# python tools/prepare-template-tests.py. RSharp's own state (Local History, logs) is never touched.

set -euo pipefail
shopt -s nullglob

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
cd "$root"

if [ ! -f Cargo.toml ] || [ ! -d crates ]; then
    printf 'clean: %s is not the RSharp repository\n' "$root" >&2
    exit 1
fi

usage() {
    sed -n '2,17p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

run=0
deep=0
temp=0
worktrees=0
agent=0
temp_age=60
while (($#)); do
    case $1 in
        -y | --yes) run=1 ;;
        -n | --dry-run) run=0 ;;
        --deep) deep=1 ;;
        --temp) temp=1 ;;
        --temp-age)
            shift
            temp_age=${1:?--temp-age needs a number of minutes}
            case $temp_age in
                '' | *[!0-9]*)
                    printf 'clean: --temp-age takes a number of minutes\n' >&2
                    exit 2
                    ;;
            esac
            ;;
        --worktrees) worktrees=1 ;;
        --all)
            deep=1
            temp=1
            worktrees=1
            agent=1
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            printf 'clean: unknown option %s (try --help)\n' "$1" >&2
            exit 2
            ;;
    esac
    shift
done

freed=0

human() {
    awk -v k="${1:-0}" 'BEGIN {
        split("KB MB GB TB", unit, " ")
        i = 1
        while (k >= 1024 && i < 4) { k /= 1024; i++ }
        if (i == 1) printf "%d KB", k
        else printf "%.1f %s", k, unit[i]
    }'
}

size_kb() {
    local kb
    kb=$(du -sk "$1" 2>/dev/null | awk 'NR == 1 { print $1 }')
    printf '%s\n' "${kb:-0}"
}

remove_path() {
    local path=$1 label=${2:-$1} kb
    [ -e "$path" ] || [ -L "$path" ] || return 0
    kb=$(size_kb "$path")
    kb=${kb:-0}
    if ((run)); then
        if rm -rf -- "$path"; then
            freed=$((freed + kb))
            printf '  %-9s removed       %s\n' "$(human "$kb")" "$label"
        else
            printf '  %-9s kept          %s (could not remove)\n' "$(human "$kb")" "$label" >&2
        fi
    else
        freed=$((freed + kb))
        printf '  %-9s would remove  %s\n' "$(human "$kb")" "$label"
    fi
}

# rustc hard-links codegen units that came out identical, so counting every path counts the same blocks
# many times. Sum the distinct inodes' allocated 512-byte blocks instead.
object_kb() {
    local deps=$1
    if [ "$(uname -s)" = Darwin ]; then
        find "$deps" -maxdepth 1 -type f -name '*.o' -exec stat -f '%i %b' {} + 2>/dev/null
    else
        find "$deps" -maxdepth 1 -type f -name '*.o' -exec stat -c '%i %b' {} + 2>/dev/null
    fi | awk '!seen[$1]++ { blocks += $2 } END { printf "%d\n", blocks * 512 / 1024 }'
}

# rustc's per-codegen-unit objects in target/*/deps. Build-script objects under target/*/build stay:
# they are the C libraries the build links against.
remove_objects() {
    local deps kb before after
    for deps in target/*/deps; do
        [ -d "$deps" ] || continue
        [ -n "$(find "$deps" -maxdepth 1 -type f -name '*.o' -print -quit)" ] || continue
        if ((run)); then
            before=$(size_kb "$deps")
            find "$deps" -maxdepth 1 -type f -name '*.o' -delete
            after=$(size_kb "$deps")
            kb=$((before - after))
            if ((kb > 0)); then
                freed=$((freed + kb))
                printf '  %-9s removed       %s/*.o\n' "$(human "$kb")" "$deps"
            fi
        else
            kb=$(object_kb "$deps")
            freed=$((freed + kb))
            printf '  %-9s would remove  %s/*.o\n' "$(human "$kb")" "$deps"
        fi
    done
}

clean_test_leftovers() {
    local path
    for path in target/template-tests target/unit-tests-proof target/unit-tests-proof-state target/tmp; do
        remove_path "$path" "$path/"
    done
    for path in target/watch-*; do
        remove_path "$path" "$path/"
    done
    for path in target/*.log target/*.png; do
        remove_path "$path" "$path"
    done
}

clean_build() {
    local kb
    kb=$(size_kb target)
    kb=${kb:-0}
    if ((run)); then
        printf '  %-9s removing      target/ (cargo clean)\n' "$(human "$kb")"
        if command -v cargo >/dev/null 2>&1; then
            cargo clean --target-dir "$root/target"
        else
            rm -rf -- "$root/target"
        fi
        freed=$((freed + kb))
    else
        freed=$((freed + kb))
        printf '  %-9s would remove  target/ (cargo clean)\n' "$(human "$kb")"
    fi
    # Passenger's sidecars are built with dotnet into the submodule; cargo clean does not reach them.
    remove_path vendor/passenger/artifacts "vendor/passenger/artifacts/ (dotnet build output)"
}

system_temp() {
    local base
    if [ "$(uname -s)" = Darwin ]; then
        base=$(getconf DARWIN_USER_TEMP_DIR 2>/dev/null || printf '%s\n' "${TMPDIR:-/tmp}")
    else
        base=${TMPDIR:-/tmp}
    fi
    printf '%s\n' "${base%/}"
}

# Test folders are named rsharp-* and dropped when their test ends; a killed run leaves them behind.
# Only ones older than --temp-age minutes go, so a test that is running right now keeps its folder.
clean_temp() {
    local base path count=0 kb=0
    base=$(system_temp)
    [ -d "$base" ] || return 0
    while IFS= read -r -d '' path; do
        count=$((count + 1))
        kb=$((kb + $(size_kb "$path")))
        if ((run)); then
            rm -rf -- "$path" || printf 'clean: could not remove %s\n' "$path" >&2
        fi
    done < <(find "$base" -maxdepth 1 -name 'rsharp-*' -mmin +"$temp_age" -print0 2>/dev/null)
    if ((count > 0)); then
        freed=$((freed + kb))
        if ((run)); then
            printf '  %-9s removed       %d stale test folders in %s\n' "$(human "$kb")" "$count" "$base"
        else
            printf '  %-9s would remove  %d stale test folders in %s\n' "$(human "$kb")" "$count" "$base"
        fi
    fi
}

# git refuses to remove a worktree with uncommitted changes; it is reported and left alone.
clean_worktrees() {
    local path kb
    command -v git >/dev/null 2>&1 || {
        printf 'clean: git is not installed; worktrees left alone\n' >&2
        return 0
    }
    while IFS= read -r path; do
        case $path in
            "$root"/.claude/worktrees/*)
                kb=$(size_kb "$path")
                kb=${kb:-0}
                if ((run)); then
                    if git worktree remove "$path" 2>/dev/null; then
                        freed=$((freed + kb))
                        printf '  %-9s removed       %s/\n' "$(human "$kb")" "${path#"$root"/}"
                    else
                        printf '  %-9s kept          %s/ (git refused: uncommitted changes?)\n' \
                            "$(human "$kb")" "${path#"$root"/}" >&2
                    fi
                else
                    freed=$((freed + kb))
                    printf '  %-9s would remove  %s/\n' "$(human "$kb")" "${path#"$root"/}"
                fi
                ;;
        esac
    done < <(git worktree list --porcelain | sed -n 's/^worktree //p')
    if ((run)); then
        git worktree prune
        rmdir "$root/.claude/worktrees" 2>/dev/null || true
    fi
}

clean_agent_leftovers() {
    local path kb=0
    remove_path .impeccable/review ".impeccable/review/ (build screenshots)"
    for path in .superpowers/sdd/*/*-tests.log; do
        kb=$((kb + $(size_kb "$path")))
        if ((run)); then rm -f -- "$path"; fi
    done
    if ((kb > 0)); then
        freed=$((freed + kb))
        if ((run)); then
            printf '  %-9s removed       .superpowers/sdd/*/*-tests.log\n' "$(human "$kb")"
        else
            printf '  %-9s would remove  .superpowers/sdd/*/*-tests.log\n' "$(human "$kb")"
        fi
    fi
}

remove_ds_store() {
    local path kb=0
    while IFS= read -r -d '' path; do
        kb=$((kb + $(size_kb "$path")))
        if ((run)); then rm -f -- "$path"; fi
    done < <(find "$root" -maxdepth 3 \( -path "$root/.git" -o -path "$root/vendor" \) -prune -o \
        -name .DS_Store -print0)
    if ((kb > 0)); then
        freed=$((freed + kb))
        if ((run)); then
            printf '  %-9s removed       .DS_Store files\n' "$(human "$kb")"
        else
            printf '  %-9s would remove  .DS_Store files\n' "$(human "$kb")"
        fi
    fi
}

if ((run)); then
    printf 'RSharp cleanup\n\n'
else
    printf 'RSharp cleanup — dry run (add --yes to remove)\n\n'
fi

if ((deep)); then
    clean_build
    remove_ds_store
else
    remove_objects
    clean_test_leftovers
    remove_ds_store
fi

if ((temp)); then
    clean_temp
fi
if ((worktrees)); then
    clean_worktrees
fi
if ((agent)); then
    clean_agent_leftovers
fi

printf '\n%s %s\n' "$( ((run)) && echo 'Freed' || echo 'Would free' )" "$(human "$freed")"
if ((!run)); then
    printf 'Run again with --yes to remove it.\n'
elif ((deep)); then
    printf 'The next build is a full rebuild.\n'
else
    printf 'Cargo fingerprints, libraries, binaries and sidecars stayed, so the next build is incremental.\n'
fi
