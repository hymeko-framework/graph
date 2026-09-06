import copy
import json
import tempfile
import unittest
from pathlib import Path

from campaign import PHASES, Campaign, CampaignSummary, Cell, Evidence


class EvidenceContract(unittest.TestCase):
    def test_phase_breakdown_is_explicit_and_file_boundaries_are_required(self):
        phases = [p[0] for p in PHASES]
        self.assertEqual(len(phases), 10)
        self.assertEqual(len(Cell.matrix("parsing", phases)), 160)
        with self.assertRaises(ValueError):
            Cell.matrix("parsing", ["unknown"])
        source = self.fixture().replace(
            "EVIDENCE_CELL\t",
            "EVIDENCE_CELL\tsection=parsing\tsubject=file_to_dijkstra\t",
        )
        source += "\nEVIDENCE_SOURCE\tbytes=100\tfnv1a64=123\tprojection=graph_fixture_v1\tfile_io=true\tfile_cache=warm"
        self.assertEqual(Evidence.parse(source)["SOURCE"][0]["file_cache"], "warm")
        for changed in (
            source.replace("file_io=true", "file_io=false"),
            source.replace("file_cache=warm", "file_cache=cold"),
        ):
            with self.assertRaises(ValueError):
                Evidence.parse(changed)

    def test_checkpoint_is_atomic_and_rejects_changed_provenance(self):
        current = {
            "source_commit": "a" * 40,
            "binaries": {"informed": {"sha256": "b" * 64}},
            "suite": "parsing",
            "platform": "windows",
            "runtime": {},
            "runs": 3,
            "smoke": False,
            "cells": [],
        }
        saved = copy.deepcopy(current)
        saved["cells"] = [{"run": 1, "id": "parse"}]
        self.assertEqual(Campaign.resume_checkpoint(current, saved), saved)
        for key in (
            "source_commit",
            "binaries",
            "suite",
            "platform",
            "runtime",
            "runs",
            "smoke",
        ):
            changed = copy.deepcopy(saved)
            changed[key] = "changed"
            with self.assertRaises(ValueError):
                Campaign.resume_checkpoint(current, changed)
        with tempfile.TemporaryDirectory() as directory:
            campaign = Campaign.__new__(Campaign)
            campaign.output = Path(directory)
            campaign.save_checkpoint(saved)
            self.assertEqual(
                json.loads((campaign.output / "checkpoint.json").read_text()), saved
            )
            self.assertFalse((campaign.output / "checkpoint.pending.json").exists())
        saved["cells"] *= 2
        with self.assertRaises(ValueError):
            Campaign.resume_checkpoint(current, saved)

    def fixture(self):
        rows = [
            f"EVIDENCE_SAMPLE\tordinal={i}\twall_us={i + 1}\tchecksum=123"
            for i in range(9)
        ]
        rows.append(
            "EVIDENCE_CELL\tstatus=OK\tsamples=9\tchecksum=123\twall_q1_us=3\twall_median_us=5\twall_q3_us=7\twall_worst_us=9"
        )
        return "\n".join(rows)

    def test_accepts_matching_raw_and_summary(self):
        self.assertEqual(len(Evidence.parse(self.fixture())["SAMPLE"]), 9)

    def test_rejects_drift_missing_duplicate_and_fabricated_summary(self):
        source = self.fixture()
        for changed in (
            source.replace("checksum=123", "checksum=999", 1),
            "\n".join(source.splitlines()[1:]),
            source.replace("ordinal=1", "ordinal=0"),
            source.replace("wall_median_us=5", "wall_median_us=6"),
            source.replace("wall_us=1", "wall_us=nan"),
            source + "\n" + source.splitlines()[-1],
        ):
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                Evidence.parse(changed)

    def test_matrix_has_complete_existing_search_suite(self):
        cells = Cell.matrix()
        self.assertEqual(sum(c.suite == "informed" for c in cells), 48)
        self.assertEqual(len(cells), len({c.key for c in cells}))
        self.assertEqual(
            {c.threads for c in cells if c.subject == "independent_query_batch"},
            {1, 2, 4},
        )

    def test_parsing_covers_every_existing_source_topology_and_phase(self):
        matrix = Cell.matrix("parsing")
        originals = {
            c.workload if not c.workload.startswith("grid200_d") else "grid200_d80"
            for c in Cell.matrix()
        }
        self.assertEqual({c.workload for c in matrix}, originals)
        self.assertEqual(len(matrix), 5 * len(originals))
        selected = Cell.matrix("parsing", ["parse_ast"])
        self.assertEqual({c.workload for c in selected}, originals)
        self.assertTrue(all(c.subject == "parse_ast" for c in selected))
        changed = {
            "source_commit": "a",
            "binaries": {},
            "suite": "parsing",
            "platform": "arm64",
            "runtime": {},
            "runs": 3,
            "smoke": False,
            "phases": ["parse_ast"],
            "cells": [],
        }
        with self.assertRaises(ValueError):
            Campaign.resume_checkpoint(changed, {**changed, "phases": ["compile"]})
        self.assertTrue(all(":" not in c.key for c in matrix))
        with self.assertRaises(ValueError):
            Cell.matrix("unknown")

    def test_parsing_requires_source_identity_and_positive_bytes(self):
        source = self.fixture().replace(
            "EVIDENCE_CELL\t", "EVIDENCE_CELL\tsection=parsing\t"
        )
        valid = (
            source
            + "\nEVIDENCE_SOURCE\tbytes=100\tfnv1a64=123\tprojection=graph_fixture_v1"
        )
        self.assertEqual(Evidence.parse(valid)["SOURCE"][0]["bytes"], "100")
        for invalid in (
            source,
            valid.replace("bytes=100", "bytes=0"),
            valid.replace("fnv1a64=123", "fnv1a64=456"),
        ):
            with self.assertRaises(ValueError):
                Evidence.parse(invalid)

    def test_archived_campaign_rejects_corrupted_provenance_and_process_identity(self):
        root = (
            Path(__file__).resolve().parents[2]
            / "reports/cross-platform-benchmarks/2026-09-06"
        )
        original = json.loads(
            (root / "windows/campaign.json").read_text(encoding="utf8")
        )
        for fault in ("source", "checksum", "repetition", "missing"):
            summary = CampaignSummary(root)
            summary.accept(original)
            changed = copy.deepcopy(original)
            if fault == "source":
                changed["source_commit"] = "0" * 40
            elif fault == "checksum":
                changed["cells"][0]["records"]["CELL"][0]["checksum"] = "0"
            elif fault == "repetition":
                changed["cells"][0]["run"] = 2
            else:
                changed["cells"].pop()
            with self.subTest(fault=fault), self.assertRaises(ValueError):
                summary.accept(changed)


if __name__ == "__main__":
    unittest.main()
