import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from benchmark_rendering import TIMINGS, percentile, summarize


class BenchmarkSummaryTests(unittest.TestCase):
    def test_percentile_keeps_tail_stalls(self):
        self.assertEqual(percentile([1] * 94 + [10] * 5 + [100], .95), 10)
        self.assertEqual(percentile([2], .95), 2)

    def test_summary_separates_workloads_and_preserves_transfer_counts(self):
        with tempfile.TemporaryDirectory() as name:
            directory = Path(name)
            (directory / "metadata.json").write_text('{"gpu": "test"}')
            (directory / "comparisons.json").write_text('[{"different_channels": 0}]')
            header = "scene,viewport_width,viewport_height,view,art,path," + ",".join(TIMINGS) + ",upload_bytes,atlas_init_bytes,draw_calls\n"
            rows = ["procedural,1920,1080,wide,native,single-crt," + ",".join([str(n)] * len(TIMINGS)) + ",267456,0,1\n" for n in [1, 2, 100]]
            rows.append("procedural,3840,2160,wide,hd10,single-crt," + ",".join(["3"] * len(TIMINGS)) + ",21396480,0,1\n")
            (directory / "frames.csv").write_text(header + "".join(rows))
            with patch("builtins.print"):
                report = summarize(directory)
            self.assertEqual(len(report["results"]), 2)
            self.assertEqual(report["results"][0]["frame_ms"]["median"], 2)
            self.assertEqual(report["results"][0]["frame_ms"]["p95"], 100)
            self.assertEqual(report["results"][1]["upload_bytes_per_frame"], 21396480)
            self.assertEqual(json.loads((directory / "summary.json").read_text()), report)


if __name__ == "__main__":
    unittest.main()
