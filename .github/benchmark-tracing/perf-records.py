import collections
import json
import pathlib
import struct
import sys

reports = []
for name in sys.argv[1:]:
    path = pathlib.Path(name)
    counts = collections.Counter()
    commands = set()
    lost_events = 0
    lost_samples = 0
    with path.open("rb") as trace:
        header = trace.read(56)
        if len(header) != 56 or header[:8] != b"PERFILE2":
            raise ValueError(f"unsupported perf file header: {path}")
        data_offset, data_size = struct.unpack_from("<QQ", header, 40)
        end = data_offset + data_size
        if end > path.stat().st_size:
            raise ValueError(f"truncated perf data: {path}")
        trace.seek(data_offset)
        while trace.tell() < end:
            record = trace.read(8)
            if len(record) != 8:
                raise ValueError(f"truncated perf record header: {path}")
            record_type, _, size = struct.unpack("<IHH", record)
            if size < 8 or trace.tell() + size - 8 > end:
                raise ValueError(f"invalid perf record size: {path}")
            counts[record_type] += 1
            if record_type in (2, 3, 13):
                payload = trace.read(size - 8)
                if record_type == 2:
                    lost_events += struct.unpack_from("<Q", payload, 8)[0]
                elif record_type == 13:
                    lost_samples += struct.unpack_from("<Q", payload)[0]
                else:
                    commands.add(payload[8:].split(b"\0", 1)[0].decode("utf-8", errors="replace"))
            else:
                trace.seek(size - 8, 1)
    if counts[9] == 0:
        raise ValueError(f"no samples in perf data: {path}")
    reports.append(
        {
            "file": path.name,
            "samples": counts[9],
            "lostEventsFromBuffers": lost_events,
            "lostSamples": lost_samples,
            "processNames": sorted(commands),
            "recordCounts": dict(counts),
        }
    )
print(json.dumps(reports, indent=2))
