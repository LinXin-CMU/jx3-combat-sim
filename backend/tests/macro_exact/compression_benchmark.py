"""Same certified baseline/oracle/contract for compression timing comparisons.

An optional test observation limit applies only to this benchmark, never the UI.
All evidence stays local; no user account or HTTP endpoint is used.
"""
import argparse
import cProfile
import importlib.util
import json
from pathlib import Path
import pstats
import re
import time

ROOT = Path(__file__).resolve().parents[3]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--baseline', type=Path, required=True)
    parser.add_argument('--module', type=Path, default=ROOT/'tools/exact_macro_compress.py')
    parser.add_argument('--exe', type=Path, default=ROOT/'backend/target/release/jx3-combat-sim.exe')
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--seconds', type=float, default=90)
    parser.add_argument('--profile', action='store_true')
    parser.add_argument('--screening', action='store_true')
    parser.add_argument('--deep',action='store_true',help='Experimental whole-program counterexample search')
    parser.add_argument('--joint',action='store_true',help='Experimental competing-action counterexample search')
    parser.add_argument('--macro', type=Path, help='Certified input macro; defaults to exact.txt')
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    synth = load('synth',ROOT/'tools/exact-macro-synth.py')
    module = load('compress',args.module.resolve())
    scene = json.loads((args.baseline/'compression-contract.json').read_text(encoding='utf-8'))['scene']
    atoms = json.loads((args.baseline/'atoms.json').read_text(encoding='utf-8'))
    actions = json.loads((args.baseline/'actions.json').read_text(encoding='utf-8'))
    index = {atom:i for i,atom in enumerate(atoms)}
    rules = []
    for line in (args.macro or args.baseline/'exact.txt').read_text(encoding='utf-8').splitlines():
        command, rest = line.split(' ',1)
        guard, name = rest[1:].split('] ',1) if rest.startswith('[') else ('',rest)
        action = next(i for i,a in enumerate(actions) if a['name']==name and a['fcast']==(command=='/fcast'))
        parts = re.split('([&|])',guard) if guard else []
        rule = {'action':action,'atoms':[index[a] for a in parts[::2]]}
        if '|' in parts[1::2]:
            rule['ops'] = parts[1::2]
        rules.append(rule)
    oracle = synth.Oracle(args.exe.resolve())
    def full_verify(text, screening=False):
        result = oracle.run(dict(scene,candidate=text,atoms=['rage<0'] if screening else atoms,stop_on_divergence=True))
        first_ms = result.get('timings_ms',{}).get('round_trip',0)
        if result['comparison']['reproduced']:
            result = oracle.run(dict(scene,candidate=text,atoms=atoms,
                archive_path=str((args.out/'full-last-passed.json').resolve())))
            result.setdefault('timings_ms',{})['verification_total'] = first_ms + result['timings_ms'].get('round_trip',0)
        else:
            result.setdefault('timings_ms',{})['verification_total'] = first_ms
        return result
    baseline = full_verify(synth.macro_text(rules,atoms,actions))
    assert module.certified(baseline)
    started = time.perf_counter()
    report = {'initial_chars':module.char_count(synth.macro_text(rules,atoms,actions)),
              'curve':[], 'trials':[], 'solvers':[], 'summary':{}, 'status':'running'}
    profiler = cProfile.Profile() if args.profile else None
    def check():
        if time.perf_counter()-started > args.seconds:
            raise TimeoutError('test observation ended')
    def verify(text,n,kind):
        check()
        result = full_verify(text,args.screening)
        report['trials'].append({'n':n,'kind':kind,'chars':module.char_count(text),
            'passed':module.certified(result),'elapsed_s':time.perf_counter()-started,
            'timings_ms':result.get('timings_ms',{})})
        return result
    def accepted(text,result,summary):
        report['curve'].append({'elapsed_s':time.perf_counter()-started,**summary})
        report['comparison'] = result['comparison']
        (args.out/'best.txt').write_text(text,encoding='utf-8')
        synth.write(args.out/'best-verified.json',result)
        print(json.dumps({'elapsed_s':round(time.perf_counter()-started,3),
                          'chars':summary['best_chars'],'trials':summary['trial_count']},ensure_ascii=False),flush=True)
    def progress(summary):
        report['summary'] = summary
    def counterexample(text,result,n,kind):
        observed = result if not args.screening else full_verify(text)
        assert observed['comparison']==result['comparison']
        assert observed['actual_fingerprint']==result['actual_fingerprint']
        if args.screening:
            report['trials'][-1]['feedback_ms'] = observed['timings_ms'].get('round_trip',0)
        synth.write(args.out/f'counterexample-{n:04d}.json',observed)
        return observed
    try:
        if profiler:
            profiler.enable()
        module.compress(rules,atoms,actions,baseline,lambda r:synth.macro_text(r,atoms,actions),
                        verify,check,lambda solver:solver.check(),accepted,progress,
                        lambda info,smt:report['solvers'].append(info),feedback=counterexample,deep_search=args.deep,
                        joint_search=args.joint)
        report['status'] = 'scope_exhausted'
    except TimeoutError:
        report['status'] = 'observation_ended'
    finally:
        if profiler:
            profiler.disable()
            profiler.dump_stats(str(args.out/'profile.pstats'))
            with (args.out/'profile.txt').open('w',encoding='utf-8') as output:
                pstats.Stats(profiler,stream=output).sort_stats('tottime').print_stats(25)
        report['elapsed_s'] = time.perf_counter()-started
        report['oracle_s'] = sum(t['timings_ms'].get('verification_total',0) for t in report['trials'])/1000
        oracle.close()
        synth.write(args.out/'benchmark.json',report)
        print(json.dumps({k:report[k] for k in ['status','elapsed_s','oracle_s','summary']},ensure_ascii=False),flush=True)


if __name__ == '__main__':
    main()
