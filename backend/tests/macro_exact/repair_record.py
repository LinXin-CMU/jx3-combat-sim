"""Replay a saved best candidate and exercise the production repair loop.

This measures continuation from a saved macro, NOT synthesis from scratch.
The source record is read-only; output must be a new directory.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import sys

root = Path(__file__).resolve().parents[3]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--record', type=Path, required=True)
parser.add_argument('--out', type=Path, required=True)
parser.add_argument('--exe', type=Path, required=True)
parser.add_argument('--iterations', type=int, default=sys.maxsize, help='Only for bounded offline experiments')
args = parser.parse_args()
if args.out.exists():
    parser.error('output must be a new directory')
read = lambda name: json.loads((args.record / name).read_text(encoding='utf-8'))
best = read('best.json')['macro'].strip()
rules = None
for candidate in args.record.glob('candidate-*.txt'):
    if candidate.read_text(encoding='utf-8').strip() == best:
        rules = read(candidate.name.replace('candidate-', 'iteration-').replace('.txt', '.json'))['rules']
        break
if rules is None:
    parser.error('record has no matching saved candidate/rules')
old_atoms, old_actions = read('atoms.json'), read('actions.json')
spec = importlib.util.spec_from_file_location('exact_synth', root / 'tools/exact-macro-synth.py')
synth = importlib.util.module_from_spec(spec)
spec.loader.exec_module(synth)
construct = synth.construct
first = True


def seeded(rows, atoms, actions, *positional, **options):
    global first
    if not first:
        return construct(rows, atoms, actions, *positional, **options)
    first = False
    # Only supply an initial hypothesis. run_job replays it from t=0 and builds
    # all subsequent counterexamples/candidates through its normal code path.
    mapped = [{'action': actions.index(old_actions[r['action']]),
               'atoms': [atoms.index(old_atoms[i]) for i in r['atoms']]} for r in rules]
    return {'status': 'candidate', 'rules': mapped, 'solve_ms': 0, 'method': 'saved_best_seed'}


synth.construct = seeded
synth.run_job(argparse.Namespace(
    scene=args.record / 'scene.json', exe=args.exe.resolve(), out=args.out,
    strategy='prototype', sizes='', seconds=float('inf'), solver_ms=0,
    iterations=args.iterations, compress=False))
