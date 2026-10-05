"""Exact HTTP integration test against an isolated test server, never real userdata."""
import argparse
import json
from pathlib import Path
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--base-url', default='http://127.0.0.1:3099')
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--pause-seconds', type=float, default=1.2)
    parser.add_argument('--no-compress', action='store_true')
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)

    def request(path, body=None):
        data = json.dumps(body, ensure_ascii=False).encode() if body is not None else None
        with urllib.request.urlopen(urllib.request.Request(args.base_url + path, data=data, headers={'Content-Type': 'application/json'}), timeout=30) as response:
            return json.load(response)

    original = request('/api/mounts/current')
    scene = json.loads((ROOT / 'backend/tests/fixtures/exact_macro_short.json').read_text(encoding='utf-8-sig'))
    job = None
    log = []
    try:
        request('/api/mounts/switch', dict(version=scene['version'], mount=scene['mount'], persist=False))
        job = request('/api/macro/exact', dict(scene, compress=not args.no_compress))
        path = '/api/macro/exact/' + job['id']
        deadline = time.monotonic() + 900  # Test timeout only; no solver wall limit.
        paused = False
        previous = None
        while not job['done']:
            if time.monotonic() > deadline:
                raise TimeoutError('integration test timed out')
            job = request(path)
            info = {k: job.get(k) for k in ('status', 'phase', 'elapsed_ms')}
            info['comparison'] = job.get('best', {}).get('comparison')
            sig = json.dumps([info['status'], info['phase'], info['comparison']], sort_keys=True)
            if sig != previous:
                log.append(info)
                print(json.dumps(info, ensure_ascii=False), flush=True)
                previous = sig
            if job.get('best') and not paused and not job['done']:
                request(path + '/pause', {})
                end = time.monotonic() + 40
                while job['phase'] != 'paused' and time.monotonic() < end:
                    time.sleep(0.2)
                    job = request(path)
                assert job['phase'] == 'paused', job['phase']
                frozen = (job['best'], job['progress'])
                hold_until = time.monotonic() + args.pause_seconds
                while time.monotonic() < hold_until:
                    time.sleep(min(1, max(0, hold_until-time.monotonic())))
                restored = request('/api/macro/exact')['job']
                assert restored['id'] == job['id'] and restored['phase'] == 'paused'
                assert (restored['best'], restored['progress']) == frozen
                request(path + '/resume', {})
                paused = True
                log.append({'pause_resume_and_restore': 'passed'})
                print('pause/resume/restore passed', flush=True)
            time.sleep(0.35)
        assert paused
        (args.out / 'job.json').write_text(json.dumps(job, ensure_ascii=False, indent=2), encoding='utf-8')
        (args.out / 'progress.json').write_text(json.dumps(log, ensure_ascii=False, indent=2), encoding='utf-8')
        if job.get('download_ready'):
            with urllib.request.urlopen(args.base_url + path + '/artifacts', timeout=30) as response:
                (args.out / 'evidence.zip').write_bytes(response.read())
        assert job['status'] == 'exact', job.get('reason', job.get('result', {}).get('report'))
        result = job['result']
        c = result['report']['comparison']
        assert c['reproduced'] and c['target_count'] == c['actual_count'] == 23
        assert c['max_time_error_on_order_prefix'] == 0
        (args.out / 'macro.txt').write_text(result['macro'], encoding='utf-8')
        # Verify stop releases admission even while a new task is paused.
        job = request('/api/macro/exact', dict(scene, compress=False))
        path = '/api/macro/exact/' + job['id']
        request(path + '/pause', {})
        request(path + '/cancel', {})
        end = time.monotonic() + 20
        while not job['done'] and time.monotonic() < end:
            time.sleep(0.25)
            job = request(path)
        assert job['done'] and job['status'] == 'cancelled', job
        print(json.dumps({'exact': c, 'pause_resume_restore': True, 'stop': True}), flush=True)
    finally:
        if job and not job['done']:
            request('/api/macro/exact/' + job['id'] + '/cancel', {})
        request('/api/mounts/switch', dict(version=original['version'], mount=original['mount'], persist=False))


if __name__ == '__main__':
    main()
