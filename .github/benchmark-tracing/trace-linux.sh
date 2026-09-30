#!/usr/bin/env bash
set -euo pipefail

output=$(realpath "$1")
shift
script_directory=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
perf=${PERF_BINARY:?PERF_BINARY must identify the installed perf executable}

"$perf" version > "$output/perf-version.txt"
uname -a > "$output/kernel.txt"
lscpu > "$output/cpu.txt"
findmnt -T "$(dirname "$CARGO_TARGET_DIR")" > "$output/filesystem.txt"
cat /proc/sys/kernel/perf_event_paranoid > "$output/perf-event-paranoid.txt"
sudo "$perf" list > "$output/perf-events.txt"

hardware_counters=false
stat_events=task-clock,context-switches,cpu-migrations,page-faults
if sudo "$perf" stat -e cycles,instructions -- true 2> "$output/hardware-counter-probe.txt"; then
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
    "missingTracepoints": sys.argv[2:],
}, indent=2))
PY

# Nested recorders stop with the workload; no background profiler can outlive it.
set +e
sudo -E "$perf" record -a --clockid mono -m 64M -o "$output/scheduler-io.perf.data" "${events[@]}" -- \
  "$perf" record --clockid mono -e cpu-clock:u -F 49 --call-graph dwarf,8192 -m 64M -o "$output/cpu.perf.data" -- \
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

python3 "$script_directory/perf-records.py" "$output/cpu.perf.data" "$output/scheduler-io.perf.data" > "$output/perf-records.json"
python3 - "$output/perf-records.json" <<'PY'
import json
import sys
for record in json.load(open(sys.argv[1])):
    if record["lostEventsFromBuffers"] or record["lostSamples"]:
        print(f'::warning::Incomplete {record["file"]}: lost events/samples; see perf-records.json')
PY
"$perf" report --stdio -n --percent-limit 0.5 --sort comm,dso,symbol -i "$output/cpu.perf.data" > "$output/cpu-report.txt" 2> "$output/symbol-warnings.txt"
"$perf" report --stdio --header-only -i "$output/scheduler-io.perf.data" > "$output/scheduler-io-header.txt"
"$perf" sched latency -i "$output/scheduler-io.perf.data" > "$output/scheduler-latency.txt" 2> "$output/scheduler-warnings.txt"
"$perf" buildid-list -i "$output/cpu.perf.data" > "$output/build-ids.txt"
sudo -E "$perf" archive "$output/cpu.perf.data" > "$output/symbol-archive.log" 2>&1
sudo chown -R "$(id -u):$(id -g)" "$output"
exit "$build_status"
