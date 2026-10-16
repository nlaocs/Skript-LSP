"""Compare two compiled REPL test binaries; keep every batch and full observation."""
import argparse
import csv
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("baseline", type=Path)
parser.add_argument("candidate", type=Path)
parser.add_argument("destination", type=Path)
parser.add_argument("--samples", type=int, default=15)
parser.add_argument("--candidate-label", choices=["borrowed", "candidate"], default="borrowed")
args = parser.parse_args()
if args.samples < 1:
    parser.error("--samples must be positive")
destination = args.destination
binaries = {"baseline": args.baseline.resolve(strict=True), args.candidate_label: args.candidate.resolve(strict=True)}
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
    "samples_per_mode": args.samples,
    "warmups_per_case_per_process": 1,
    "repetitions_per_batch": 1,
    "order": [],
}
(destination / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
observations = {}
with (destination / "samples.csv").open("w", newline="") as output, (destination / "first-samples.csv").open("w", newline="") as first_output:
    writer = csv.writer(output)
    writer.writerow(["mode", "case", "sample", "repetitions", "parse_ns", "report_ns", "checksum"])
    first_writer = csv.writer(first_output)
    first_writer.writerow(["mode", "case", "sample", "repetitions", "parse_ns", "report_ns", "checksum"])
    for sample in range(args.samples):
        order = ["baseline", args.candidate_label] if sample % 2 == 0 else [args.candidate_label, "baseline"]
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
            rows, first_rows, seen = [], [], set()
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
                if "capture-pipeline-first," in line:
                    case, index, repetitions, parse_ns, report_ns, checksum = line.partition("capture-pipeline-first,")[2].split(",")
                    assert int(index) == sample and int(repetitions) == 1
                    first_rows.append([mode, case, sample, repetitions, parse_ns, report_ns, checksum])
            assert len(rows) == 3 and len(seen) == 3
            if first_rows:
                assert len(first_rows) == 3
                assert {row[1]:row[-1] for row in rows} == {row[1]:row[-1] for row in first_rows}
            writer.writerows(rows)
            first_writer.writerows(first_rows)
            output.flush()
            first_output.flush()
            print(f"sample={sample} mode={mode} matched=3", flush=True)
(destination / "observations.json").write_text(json.dumps(observations, indent=2) + "\n")
print(f"Completed: {args.samples * 6} warm batches; all full observations agree.")

