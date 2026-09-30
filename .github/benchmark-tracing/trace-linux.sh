#!/usr/bin/env bash
set -euo pipefail

output=$(realpath "$1")
shift
script_directory=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
perf=${PERF_BINARY:?PERF_BINARY must identify the installed perf executable}
symbol_cache="${RUNNER_TEMP:?RUNNER_TEMP must identify the runner temporary directory}/perf-buildids"

"$perf" version > "$output/perf-version.txt"
uname -a > "$output/kernel.txt"
lscpu > "$output/cpu.txt"
findmnt -T "$(dirname "$CARGO_TARGET_DIR")" > "$output/filesystem.txt"
cat /proc/sys/kernel/perf_event_paranoid > "$output/perf-event-paranoid.txt"
sudo "$perf" list > "$output/perf-events.txt"

python3 - "$symbol_cache" > "$output/host-modules.json" <<'PY'
import hashlib
import json
import pathlib
import shutil
import subprocess
import sys

cache = pathlib.Path(sys.argv[1])
modules = [
    pathlib.Path(subprocess.check_output(["rustup", "which", tool], text=True).strip()).resolve()
    for tool in ("rustc", "cargo")
]
drivers = list((modules[0].parent.parent / "lib").glob("librustc_driver*.so"))
if not drivers:
    raise ValueError("Rust compiler driver not found")
manifest = []
for path in modules + drivers:
    destination = cache / path.relative_to(path.anchor)
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(path, destination)
    with path.open("rb") as module:
        digest = hashlib.file_digest(module, "sha256").hexdigest()
    manifest.append({"path": str(path), "sha256": digest, "symbolFile": str(destination.relative_to(cache))})
print(json.dumps(manifest, indent=2))
PY

hardware_counters=false
stat_events=task-clock,context-switches,cpu-migrations,page-faults
if sudo "$perf" stat -e cycles,instructions -- sleep 0.1 2> "$output/hardware-counter-probe.txt" &&
  ! grep -Eq '<not supported>|<not counted>' "$output/hardware-counter-probe.txt"; then
  hardware_counters=true
  stat_events+=,cycles,instructions
else
  echo "::warning::Hardware counters are unavailable; recording software CPU samples and resource counters"
fi

events=()
missing_events=()
for event in \
  sched:sched_switch sched:sched_wakeup sched:sched_process_fork sched:sched_process_exit \
  block:block_rq_issue block:block_rq_complete \
  syscalls:sys_enter_openat syscalls:sys_exit_openat \
  syscalls:sys_enter_read syscalls:sys_exit_read \
  syscalls:sys_enter_write syscalls:sys_exit_write \
  syscalls:sys_enter_newfstatat syscalls:sys_exit_newfstatat; do
  if sudo test -f "/sys/kernel/tracing/events/${event/:/\/}/id"; then
    events+=(-e "$event")
    if [[ "$event" == syscalls:* ]]; then
      events+=(--exclude-perf)
    fi
  else
    missing_events+=("$event")
    echo "::warning::Tracepoint unavailable: $event"
  fi
done
for event in sched:sched_switch sched:sched_wakeup syscalls:sys_enter_openat; do
  if [[ " ${missing_events[*]} " == *" $event "* ]]; then
    echo "::error::Required scheduler/filesystem tracepoint unavailable: $event"
    exit 1
  fi
done

python3 - "$hardware_counters" "${missing_events[@]}" > "$output/trace-capabilities.json" <<'PY'
import json
import sys
print(json.dumps({
    "hardwareCounters": sys.argv[1] == "true",
    "cpuSampling": "cpu-clock:u at 49 Hz, DWARF stacks (8192 bytes)",
    "cpuScope": "build process tree",
    "schedulerIoScope": "system-wide, filter using build-interval.json",
    "recorderSyscallsExcluded": True,
    "symbolAvailability": "build-ID cache plus exact Rust host modules in cpu.perf.data.tar.bz2; see host-modules.json",
    "missingTracepoints": sys.argv[2:],
}, indent=2))
PY

# Nested recorders stop with the workload; no background profiler can outlive it.
set +e
sudo -E "$perf" record -a --clockid mono -m 64M -o "$output/scheduler-io.perf.data" "${events[@]}" -- \
  "$perf" --buildid-dir "$symbol_cache/.debug" record --clockid mono -e cpu-clock:u -F 49 --call-graph dwarf,8192 -m 64M -o "$output/cpu.perf.data" -- \
  "$perf" stat -x , -e "$stat_events" -o "$output/perf-stat.csv" -- \
  sudo -E -u "$(id -un)" env "HOME=$HOME" "PATH=$PATH" \
  python3 "$script_directory/measure-build.py" "$output/build-interval.json" "$@" \
  2> "$output/perf-record.log" | tee "$output/build.log"
statuses=("${PIPESTATUS[@]}")
set -e
sudo chown -R "$(id -u):$(id -g)" "$output"
if [[ "${statuses[1]}" -ne 0 ]]; then
  echo "::error::Writing the build log failed"
  exit "${statuses[1]}"
fi
if [[ ! -f "$output/build-interval.json" ]]; then
  cat "$output/perf-record.log"
  echo "::error::Profiler failed before launching the build"
  exit 1
fi
build_status=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["exitCode"])' "$output/build-interval.json")
if [[ "${statuses[0]}" -ne "$build_status" ]]; then
  cat "$output/perf-record.log"
  echo "::error::Profiler failed independently of the build"
  exit 1
fi

trap 'echo "::error::Linux tracing failed at line $LINENO: $BASH_COMMAND" >&2' ERR
sudo chown -R "$(id -u):$(id -g)" "$symbol_cache"
python3 "$script_directory/perf-records.py" "$output/cpu.perf.data" "$output/scheduler-io.perf.data" > "$output/perf-records.json"
python3 - "$output/perf-records.json" <<'PY'
import json
import sys
for record in json.load(open(sys.argv[1])):
    if record["lostEventsFromBuffers"] or record["lostSamples"]:
        print(f'::warning::Incomplete {record["file"]}: lost events/samples; see perf-records.json')
PY
"$perf" report --symfs "$symbol_cache" --stdio -n --percent-limit 0.5 --sort comm,dso,symbol -i "$output/cpu.perf.data" > "$output/cpu-report.txt" 2> "$output/symbol-warnings.txt"
"$perf" report --symfs "$symbol_cache" --stdio --no-children --call-graph none --percent-limit 0 \
  --comms clang-cl,lld-link --sort comm,dso,symbol -i "$output/cpu.perf.data" > "$output/native-cpu-report.txt" 2>> "$output/symbol-warnings.txt"
if ! grep -Eq '\[\.\] (clang|llvm|lld)::' "$output/native-cpu-report.txt"; then
  echo "::error::Linux native compiler symbols did not resolve"
  exit 1
fi
"$perf" report --stdio --header-only -i "$output/scheduler-io.perf.data" > "$output/scheduler-io-header.txt"
"$perf" sched latency -i "$output/scheduler-io.perf.data" > "$output/scheduler-latency.txt" 2> "$output/scheduler-warnings.txt"
"$perf" buildid-list -i "$output/cpu.perf.data" > "$output/build-ids.txt"
# Ubuntu omits perf-archive; preserve its portable build-ID cache layout directly.
if ! tar -cjf "$output/cpu.perf.data.tar.bz2" -C "$symbol_cache" . > "$output/symbol-archive.log" 2>&1; then
  cat "$output/symbol-archive.log"
  echo "::error::Exporting Linux host symbols failed"
  exit 1
fi
tar -tjf "$output/cpu.perf.data.tar.bz2" > "$output/symbol-archive-contents.txt"
if ! grep -Fq './.debug/.build-id/' "$output/symbol-archive-contents.txt"; then
  echo "::error::Linux symbol archive has no portable build-ID cache"
  exit 1
fi
if ! grep -q librustc_driver "$output/symbol-archive-contents.txt"; then
  cat "$output/symbol-archive.log"
  echo "::error::Linux host-symbol archive does not include rustc"
  exit 1
fi
sudo chown -R "$(id -u):$(id -g)" "$output"
exit "$build_status"
