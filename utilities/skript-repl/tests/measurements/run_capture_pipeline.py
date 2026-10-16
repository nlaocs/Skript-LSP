"""Compare two compiled REPL test binaries; keep every batch and full observation."""
import csv
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

baseline, borrowed, destination = map(Path, sys.argv[1:])
binaries = {"baseline": baseline.resolve(strict=True), "borrowed": borrowed.resolve(strict=True)}
destination.mkdir(parents=True, exist_ok=False)
command_tail = [
    "session::capture_pipeline::measure_real_capture_pipeline",
    "--exact", "--ignored", "--nocapture", "--test-threads=1",
]
manifest = {
    "binaries": {
        mode: {"path": str(path), "sha256": hashlib.file_digest(path.open("rb"), "sha256").hexdigest()}
        for mode, path in binaries.items()
    },
    "command_tail": command_tail,
    "harness_sha256": hashlib.sha256(Path(__file__).with_name("capture_pipeline.rs").read_bytes()).hexdigest(),
    "samples_per_mode": 15,
    "warmups_per_case_per_process": 1,
    "repetitions_per_batch": 1,
    "order": [],
}
(destination / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
observations = {}
with (destination / "samples.csv").open("w", newline="") as output:
    writer = csv.writer(output)
    writer.writerow(["mode", "case", "sample", "repetitions", "parse_ns", "report_ns", "checksum"])
    for sample in range(15):
        order = ["baseline", "borrowed"] if sample % 2 == 0 else ["borrowed", "baseline"]
        for mode in order:
            env = dict(os.environ, CAPTURE_PIPELINE_SAMPLE=str(sample))
            result = subprocess.run(
                [str(binaries[mode]), *command_tail], env=env, text=True,
                stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            )
            (destination / f"{sample:02}-{mode}.log").write_text(result.stdout)
            manifest["order"].append({"sample": sample, "mode": mode, "exit": result.returncode})
            (destination / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
            assert result.returncode == 0, f"{mode}/{sample}: see saved log"
            rows, seen = [], set()
            for line in result.stdout.splitlines():
                if "pipeline-observation," in line:
                    case, encoded = line.partition("pipeline-observation,")[2].split(",", 1)
                    observation = json.loads(encoded)
                    if case in observations:
                        assert observation == observations[case], f"{mode}/{sample}/{case}: output differs"
                    else:
                        observations[case] = observation
                    seen.add(case)
                if "capture-pipeline," in line:
                    case, index, repetitions, parse_ns, report_ns, checksum = line.partition("capture-pipeline,")[2].split(",")
                    assert int(index) == sample and int(repetitions) == 1
                    rows.append([mode, case, sample, repetitions, parse_ns, report_ns, checksum])
            assert len(rows) == 3 and len(seen) == 3
            writer.writerows(rows)
            output.flush()
            print(f"sample={sample} mode={mode} matched=3", flush=True)
(destination / "observations.json").write_text(json.dumps(observations, indent=2) + "\n")
print("Completed: 90 batches; all full observations agree.")

