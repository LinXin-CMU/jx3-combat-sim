"""Batch truth columns retain row identity, truthiness and cancellation."""
import importlib.util
from pathlib import Path
import random
import unittest

SPEC = importlib.util.spec_from_file_location('sample_columns_fixtures',
    Path(__file__).with_name('compression_test.py'))
FIX = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(FIX)


class SampleColumnsTests(unittest.TestCase):
    def test_batch_matches_scalar_on_byte_boundaries_and_partial_rows(self):
        rng = random.Random(149)
        for height, width, ragged in [(0, 3, False), (4, 0, False),
                (7, 9, False), (8, 63, False), (9, 65, False),
                (130, 193, False), (17, 35, True)]:
            atoms = ['rage=' + str(k) for k in range(width)]
            rows = [FIX.sample([rng.choice([0, 0, 1, 2, 255])
                    for _ in range(rng.randrange(width+1) if ragged else width)],
                    [1], [0] if s % 3 else [], s) for s in range(height)]
            expected = [sum((1 << s) for s, row in enumerate(rows)
                            if k < len(row['truth']) and row['truth'][k])
                        for k in range(width)]
            got = FIX.compress.Samples(rows, atoms, [dict(name='skill', fcast=False)], lambda: None)
            self.assertEqual(got.truth, expected)
            self.assertEqual(got.allowed[0], sum(1 << s for s in range(height) if s % 3))

    def test_column_conversion_keeps_cancellation_boundaries(self):
        calls = 0
        def check():
            nonlocal calls
            calls += 1
            if calls == 5:
                raise InterruptedError('stopped while transposing')
        rows = [FIX.sample([1]*129, [1], [0], i) for i in range(3)]
        with self.assertRaisesRegex(InterruptedError, 'transposing'):
            FIX.compress.Samples(rows, ['a']*129, [dict(name='skill', fcast=False)], check)


if __name__ == '__main__':
    unittest.main()
