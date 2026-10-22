#!/bin/sh
# The edge, measured: the release build on solutions of 1, 3 and 10 projects (one file of 20,000 lines in the
# first), each run with a state folder of its own. Appends to bench/results.csv by commit and prints the runs.
# BENCH_SIZES picks the solutions ("1 3 10" by default). Needs dotnet and roslyn-language-server. Keep the
# window in front while it runs: macOS draws nothing for a hidden window, which shows as stalls in the results.
set -eu
cd "$(dirname "$0")/.."
cargo build --release --quiet
sizes=${BENCH_SIZES:-"1 3 10"}
root=target/bench-solutions
commit=$(git rev-parse --short HEAD)$(git diff --quiet || echo "+dirty")
mkdir -p bench
[ -f bench/results.csv ] || echo "date,commit,projects,firstFrameMs,solutionLoadedMs,ideRssMb,totalRssMb,keyToFrameMedianMs,keyToFrameP95Ms,stalls" > bench/results.csv

make_solution() {
    n=$1; dir=$root/Bench$n
    [ -f "$dir/Bench$n.slnx" ] && return
    mkdir -p "$dir"
    printf '<Solution>\n' > "$dir/Bench$n.slnx"
    for p in $(seq 1 "$n"); do
        mkdir -p "$dir/P$p"
        printf '<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net10.0</TargetFramework><Nullable>enable</Nullable></PropertyGroup></Project>\n' > "$dir/P$p/P$p.csproj"
        printf '  <Project Path="P%s/P%s.csproj" />\n' "$p" "$p" >> "$dir/Bench$n.slnx"
        for f in $(seq 1 10); do
            python3 -c "
print('namespace P$p;')
print('public class C$f')
print('{')
for m in range(40):
    print(f'    public int M{m}(int x) {{ var total = x; for (var i = 0; i < {m}; i++) {{ total += i * $f; }} return total; }}')
print('}')" > "$dir/P$p/C$f.cs"
        done
    done
    python3 -c "
print('namespace P1;')
print('public class Big')
print('{')
for m in range(4996):
    print(f'    public string M{m}(string s) => s + \"{m}\";')
print('}')" > "$dir/P1/Big.cs"
    printf '</Solution>\n' >> "$dir/Bench$n.slnx"
    (cd "$dir" && dotnet restore "Bench$n.slnx" --verbosity quiet)
}

for n in $sizes; do
    make_solution "$n"
    state=$(mktemp -d)
    result=$(RSHARP_STATE=$state target/release/rsharp "$PWD/$root/Bench$n/Bench$n.slnx" --file P1/Big.cs --caret 3:5 --bench | tail -1)
    rm -rf "$state"
    echo "$n projects: $result"
    python3 -c "
import json, sys, datetime
r = json.loads(sys.argv[1])
row = [datetime.date.today().isoformat(), sys.argv[2], sys.argv[3]] + ['' if r.get(k) is None else f'{r[k]:.1f}' for k in
       ['firstFrameMs', 'solutionLoadedMs', 'ideRssMb', 'totalRssMb', 'keyToFrameMedianMs', 'keyToFrameP95Ms']] + [str(r.get('stalls', 0))]
print(','.join(row))" "$result" "$commit" "$n" >> bench/results.csv
done
