"""Thin observation catalogs must not change native macro replay or wait timing."""
import argparse
import importlib.util
import json
from pathlib import Path
import tempfile

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location('synth',ROOT/'tools/exact-macro-synth.py')
synth = importlib.util.module_from_spec(spec)
spec.loader.exec_module(synth)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--exe',type=Path,default=ROOT/'backend/target/release/jx3-combat-sim.exe')
    args = parser.parse_args()
    scene = json.loads((ROOT/'backend/tests/fixtures/exact_macro_short.json').read_text(encoding='utf-8-sig'))
    scene.update(horizon=24.1250001,acceptance='skills_and_time',time_tolerance_seconds=0.0625)
    passed = '\n'.join(['/cast 盾猛','/cast [rage<50] 盾压','/cast 盾刀','/cast [rage=85] 斩刀',
        '/cast [sun>54] 阵云结晦','/cast 月照连营','/cast [sun=48] 雁门迢递','/cast 绝刀',
        '/cast [rage=70&sun>50] 血怒','/cast [rage=85] 盾飞','/cast [sun=30] 盾回'])
    timer = '/cast [bufftime:血怒<3.1] 盾回\n' + passed
    oracle = synth.Oracle(args.exe.resolve())
    try:
        prepared = oracle.run(scene)
        assert prepared['status']=='ok'
        # Equal-length bridges omit only the large raw archive. Independent
        # full replay, truth guidance and every comparison remain identical.
        with tempfile.TemporaryDirectory(prefix='exact-archive-') as folder:
            archive = Path(folder)/'full.json'
            stored = oracle.run(dict(scene,candidate=passed,atoms=prepared['atoms'],
                                     archive_path=str(archive)))
            unstored = oracle.run(dict(scene,candidate=passed,atoms=prepared['atoms']))
            assert stored['comparison'] == unstored['comparison']
            assert stored['rows'] == unstored['rows']
            assert stored['actual_fingerprint'] == unstored['actual_fingerprint']
            full = json.loads(archive.read_text(encoding='utf-8'))
            assert full['comparison'] == unstored['comparison']
            assert len(full['actual']) == len(unstored['actual'])
        for candidate in [passed,timer,'/cast 血怒\n/cast 盾刀']:
            for stop in [False,True]:
                full = oracle.run(dict(scene,candidate=candidate,atoms=prepared['atoms'],stop_on_divergence=stop))
                thin = oracle.run(dict(scene,candidate=candidate,atoms=['rage<0'],stop_on_divergence=stop))
                assert thin['status']==full['status']
                assert thin['comparison']==full['comparison']
                assert thin['actual_fingerprint']==full['actual_fingerprint']
                assert [r['time'] for r in thin['rows']]==[r['time'] for r in full['rows']]
                assert all(len(r['truth'])==1 for r in thin['rows'])
        # A real channel is executed and finalized by the same engine too.
        channel = dict(scene,simulation=dict(scene['simulation'],sequence=['盾舞']),horizon=8.0)
        for stop in [False,True]:
            full = oracle.run(dict(channel,candidate='/fcast 盾舞\n/cast 血怒',stop_on_divergence=stop))
            thin = oracle.run(dict(channel,candidate='/fcast 盾舞\n/cast 血怒',atoms=['rage<0'],stop_on_divergence=stop))
            assert thin['comparison']==full['comparison']
            assert thin['actual_fingerprint']==full['actual_fingerprint']
        # The optimizer's AND-prefix/OR-suffix must match the native right-
        # associative grammar. Compare it with two adjacent AND rules using
        # the same action and all the same Buff wake-up thresholds.
        leaves = ['rage>60','bufftime:血怒<3.1','rage>80']
        rule = {'action':0,'atoms':[0],'any_atoms':[1,2]}
        line = synth.macro_text([rule],leaves,[{'name':'盾刀','fcast':False}])
        assert line == '/cast [rage>60&bufftime:血怒<3.1|rage>80] 盾刀'
        expanded = '/cast [rage>60&bufftime:血怒<3.1] 盾刀\n/cast [rage>60&rage>80] 盾刀'
        catalog = leaves + ['rage>60&bufftime:血怒<3.1|rage>80']
        joined = oracle.run(dict(scene,candidate=line+'\n'+passed,atoms=catalog))
        separate = oracle.run(dict(scene,candidate=expanded+'\n'+passed,atoms=catalog))
        assert joined['comparison']==separate['comparison']
        assert joined['actual_fingerprint']==separate['actual_fingerprint']
        assert [r['time'] for r in joined['rows']]==[r['time'] for r in separate['rows']]
        assert all(bool(r['truth'][3]) == (bool(r['truth'][0]) and
                    (bool(r['truth'][1]) or bool(r['truth'][2]))) for r in joined['rows'])
        # Alternating native operators extend beyond the old OR suffix form.
        leaves = ['rage<15','rage>60','bufftime:血怒<3.1','rage>80']
        chain = {'action':0,'atoms':[0,1,2,3],'ops':['|','&','|']}
        line = synth.macro_text([chain],leaves,[{'name':'盾刀','fcast':False}])
        expanded = '\n'.join(['/cast [rage<15] 盾刀',
                              '/cast [rage>60&bufftime:血怒<3.1] 盾刀',
                              '/cast [rage>60&rage>80] 盾刀'])
        catalog = leaves + [line.split('[',1)[1].split(']',1)[0]]
        joined = oracle.run(dict(scene,candidate=line+'\n'+passed,atoms=catalog))
        separate = oracle.run(dict(scene,candidate=expanded+'\n'+passed,atoms=catalog))
        assert joined['comparison']==separate['comparison']
        assert joined['actual_fingerprint']==separate['actual_fingerprint']
        assert [r['time'] for r in joined['rows']]==[r['time'] for r in separate['rows']]
        assert all(bool(r['truth'][4]) == (bool(r['truth'][0]) or
                   (bool(r['truth'][1]) and (bool(r['truth'][2]) or bool(r['truth'][3])))) for r in joined['rows'])
        print('PASS alternating native AND/OR: right-associated truth and complete native replay.')
        print('PASS generated AND-prefix/OR-suffix: native truth, action trajectory and wake-up timing.')
        print('PASS thin/full catalogs: passing macro, failed macro, native Buff wakeups, channels, full replay and early stop.')
    finally:
        oracle.close()


if __name__ == '__main__':
    main()
