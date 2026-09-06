import copy
import json
import unittest
from pathlib import Path

from campaign import CampaignSummary, Cell, Evidence


class EvidenceContract(unittest.TestCase):
    def fixture(self):
        rows = [f"EVIDENCE_SAMPLE\tordinal={i}\twall_us={i + 1}\tchecksum=123" for i in range(9)]
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
        self.assertEqual({c.threads for c in cells if c.subject == "independent_query_batch"}, {1, 2, 4})

    def test_archived_campaign_rejects_corrupted_provenance_and_process_identity(self):
        root = Path(__file__).resolve().parents[2] / "reports/cross-platform-benchmarks/2026-09-06"
        original = json.loads((root / "windows/campaign.json").read_text(encoding="utf8"))
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
