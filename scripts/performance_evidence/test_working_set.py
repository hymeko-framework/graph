import unittest
from working_set import WindowsWorkingSet


class PageAccounting(unittest.TestCase):
    def test_private_cow_and_shared_pages_are_separate(self):
        entry=lambda page,shared,count:(page<<12)|(int(shared)<<8)|(count<<5)
        r=WindowsWorkingSet.classify([entry(16,False,0),entry(17,True,2),entry(18,True,7),entry(50,True,1)],[(65536,12288,'engine.dll')])
        self.assertEqual(r['private_working_set_b'],4096)
        self.assertEqual(r['sharable_working_set_b'],12288)
        self.assertEqual(r['shared_with_other_processes_b'],8192)
        self.assertEqual(r['other_sharable_b'],4096)
        self.assertEqual(r['share_count_saturated_pages'],1)
        self.assertEqual(len(r['image_pages']),2)

    def test_cohort_deduplicates_only_compatible_image_mappings(self):
        base=dict(private_working_set_b=4096,private_committed_b=8192,
                  enumerated_working_set_b=8192,sharable_working_set_b=4096,
                  other_sharable_b=0,image_pages=[('engine.dll',65536,1)])
        r=WindowsWorkingSet.cohort([base,base])
        self.assertEqual(r['summed_working_set_b'],16384)
        self.assertEqual(r['unique_image_mapping_b'],4096)
        self.assertEqual(r['image_deduplicated_estimate_b'],12288)
        changed=dict(base,image_pages=[('different.exe',65536,1)])
        self.assertEqual(WindowsWorkingSet.cohort([base,changed])['unique_image_mapping_b'],8192)
        changed=dict(base,image_pages=[('engine.dll',131072,1)])
        self.assertEqual(WindowsWorkingSet.cohort([base,changed])['unique_image_mapping_b'],8192)
        with self.assertRaises(ValueError):WindowsWorkingSet.cohort([])
