"""Profile accounting must distinguish nesting, threads, filtering and overflow."""
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location('profile_tool',ROOT/'tools/profile-exact-macro.py')
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ProfileToolTests(unittest.TestCase):
    def test_observed_children_subtract_once_and_threads_stay_separate(self):
        def event(name, start, duration, tid=1):
            return dict(name=name,ts=start,dur=duration,pid=1,tid=tid,ph='X')
        result = MODULE.summarize_trace({'traceEvents':[
            event('parent',0,10000), event('child',1000,4000),
            event('grandchild',2000,2000), event('child',6000,1000),
            event('reader',0,9000,2), dict(ph='M',name='thread_name'),
        ],'viztracer_metadata':{'overflow':True}})
        rows = {row['function']:row for row in result['top_residual']}
        self.assertEqual(rows['parent']['residual_ms'],5)
        self.assertEqual(rows['child']['residual_ms'],3)
        self.assertEqual(rows['reader']['residual_ms'],9)
        self.assertEqual(result['thread_count'],2)
        self.assertTrue(result['overflow'])


if __name__ == '__main__':
    unittest.main()
