"""A new event witness may move within the accepted observed time window."""
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[3]


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT/'tools'/f'{name}.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


COMPRESS, REORDER = load('exact_macro_compress'), load('exact_macro_reorder')


class EventQueryTests(unittest.TestCase):
    def samples(self):
        rows = [dict(cursor=0,time=.01,truth=[True,False],executable=[True],allowed=[],wait_allowed=True),
                dict(cursor=0,time=.06,truth=[True,False],executable=[True],allowed=[0],wait_allowed=True),
                dict(cursor=0,time=.10,truth=[False,True],executable=[True],allowed=[0],wait_allowed=False)]
        return COMPRESS.Samples(rows,['rage<5','bufftime:x<.1'],[dict(name='A',fcast=False)],lambda:None)

    def test_query_uses_entire_event_window_not_old_cast_mask(self):
        samples = self.samples()
        query = REORDER.event_query(samples,0,0b100,samples.all,[])
        self.assertEqual(query.windows,(0b110,))
        self.assertEqual(query.negatives,0b001)
        self.assertEqual(query.path_id,samples.path_id)

    def test_earlier_witness_does_not_require_old_deadline_cast(self):
        samples = self.samples()
        samples.truth[0] = 0b010
        trial = [dict(action=0,atoms=[0])]
        self.assertFalse(samples.compatible(trial))
        self.assertTrue(samples.window_compatible(trial))

    def test_extended_generator_reaches_witness_excluded_by_fast_query(self):
        samples = self.samples()
        samples.truth[0] = 0b010
        source = [dict(action=0,atoms=[1])]
        expected = [dict(action=0,atoms=[0])]
        self.assertNotIn(expected,[trial for _,trial in COMPRESS.window_edits(source,samples)])
        normalize = COMPRESS.condition_module().normalize
        self.assertIn([normalize(expected[0])],
                      [[normalize(rule) for rule in trial] for _,trial in COMPRESS.event_window_edits(source,samples)])

    def test_early_wrong_cast_still_rejected(self):
        samples = self.samples()
        self.assertFalse(samples.window_compatible([dict(action=0,atoms=[0])]))

    def test_distinct_path_provenance_is_not_merged(self):
        left = self.samples()
        right = COMPRESS.Samples(left.rows,left.atoms,left.actions,left.check,{'candidate':'other'})
        self.assertNotEqual(left.path_id,right.path_id)


if __name__ == '__main__':
    unittest.main()
