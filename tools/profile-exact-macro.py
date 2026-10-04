"""Opt-in local VizTracer tool for exact-macro extraction and compression.

Usage: python tools/profile-exact-macro.py compression --out <directory> --
         --baseline <certified-baseline> --screening --seconds 600
Everything after -- goes to the existing runner, whose --out is supplied here.
Profiling changes wall time; compare speed using uninstrumented benchmarks.
"""
from __future__ import annotations

import argparse
from collections import defaultdict
import hashlib
import importlib.metadata
import json
import math
from pathlib import Path
import queue
import sys
import threading
import time

ROOT = Path(__file__).resolve().parents[1]


def summarize_trace(data):
    """Observed exclusive wall time per thread, never a Python CPU estimate.

    Filters omit short calls and native children. Their duration remains in
    the caller's residual. Native subprocess waiting remains in Oracle.run.
    Thread totals overlap and must not be summed into elapsed wall time.
    """
    threads = defaultdict(list)
    for event in data.get('traceEvents', []):
        if event.get('ph') == 'X' and event.get('dur', 0) >= 0:
            threads[(event.get('pid'), event.get('tid'))].append(event)
    functions = defaultdict(lambda: dict(calls=0, inclusive_ms=0., residual_ms=0., max_ms=0.))
    for events in threads.values():
        stack = []
        def finish():
            event, _, covered, _ = stack.pop()
            values = functions[event['name']]
            duration = event['dur'] / 1000
            values['calls'] += 1
            values['inclusive_ms'] += duration
            values['residual_ms'] += max(0, event['dur'] - covered) / 1000
            values['max_ms'] = max(values['max_ms'], duration)
        for event in sorted(events, key=lambda e: (e['ts'], -e['dur'])):
            start, end = event['ts'], event['ts'] + event['dur']
            while stack and (start >= stack[-1][1] or end > stack[-1][1]):
                finish()
            if stack:
                parent = stack[-1]
                parent[2] += max(0, end - max(start, parent[3]))
                parent[3] = max(parent[3], end)
            stack.append([event, end, 0, start])
        while stack:
            finish()
    def category(name):
        if name.startswith('Oracle.run '):
            return 'native_replay_and_ipc'
        if 'Oracle.__init__.<locals>.receive ' in name:
            return 'receiver_wait_and_decode'
        if 'threading.py:' in name or 'queue.py:' in name:
            return 'thread_or_queue_wait'
        if name.startswith('JSON') or name.startswith(('loads ', 'dumps ')):
            return 'json'
        return 'project'
    ranked = sorted((dict(function=name, category=category(name), **values) for name, values in functions.items()),
                    key=lambda row: (-row['residual_ms'], row['function']))
    return dict(event_count=sum(map(len, threads.values())), thread_count=len(threads),
                overflow=bool(data.get('viztracer_metadata', {}).get('overflow')),
                top_residual=ranked[:40],
                top_project_residual=[row for row in ranked if row['category']=='project'][:40],
                boundary_inclusive=[row for row in ranked if row['category'] in
                    ('native_replay_and_ipc','receiver_wait_and_decode')],
                top_inclusive=sorted(ranked, key=lambda row: -row['inclusive_ms'])[:40])


def markdown_summary(summary):
    lines = ['# Exact macro Python profile', '',
        f'Phase: {summary["phase"]}. Profiled wall: {summary["profiled_wall_seconds"]:.3f}s. Runner exit: {summary["runner_exit_code"]}.',
        f'Trace complete: {summary["trace_complete"]}. Events: {summary["event_count"]}.', '',
        summary['interpretation'], '',
        '## Native and receiver boundaries (overlapping wall time)', '',
        '| Function | Calls | Inclusive ms |', '| --- | ---: | ---: |']
    for row in summary['boundary_inclusive']:
        lines.append(f'| {row["function"]} | {row["calls"]} | {row["inclusive_ms"]:.1f} |')
    lines += ['', '## Project functions (observed residual wall time)', '',
              '| Function | Calls | Inclusive ms | Residual ms |', '| --- | ---: | ---: | ---: |']
    for row in summary['top_project_residual'][:20]:
        name = row['function'].replace('|', '\\|')
        lines.append(f'| {name} | {row["calls"]} | {row["inclusive_ms"]:.1f} | {row["residual_ms"]:.1f} |')
    return '\n'.join(lines)+'\n'


def main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    split = argv.index('--') if '--' in argv else len(argv)
    own, forwarded = argv[:split], argv[split+1:]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('phase', choices=('extraction', 'compression', 'pipeline'))
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--min-us', type=float, default=100., help='Recorded-call duration filter; microseconds')
    parser.add_argument('--entries', type=int, default=500_000, help='Trace ring capacity; overflow is explicitly reported')
    parser.add_argument('--include-c', action='store_true', help='Also trace C calls; increases overhead')
    args = parser.parse_args(own)
    if not forwarded:
        parser.error('pass runner arguments after --; see --help')
    if not math.isfinite(args.min_us) or args.min_us < 0 or args.entries <= 0:
        parser.error('min-us must be nonnegative and entries must be positive')
    if any(arg == '--out' or arg.startswith('--out=') for arg in forwarded):
        parser.error('runner output is automatically stored in <out>/run')
    if '--profile' in forwarded:
        parser.error('do not combine cProfile and VizTracer in one measurement')
    if args.phase == 'extraction' and '--compress' in forwarded:
        parser.error('use pipeline to profile both stages')
    try:
        from viztracer import VizTracer
    except ImportError:
        parser.error('install optional profiler: python -m pip install -r tools/requirements-exact-macro-profile.txt')
    output = args.out.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if any((output/name).exists() for name in ('trace.json','trace.html','summary.json','run')):
        parser.error('use a fresh output directory to preserve previous evidence')
    script = ROOT/('backend/tests/macro_exact/compression_benchmark.py'
                   if args.phase == 'compression' else 'tools/exact-macro-synth.py')
    target_args = [str(script), *forwarded, '--out', str(output/'run')]
    if args.phase == 'extraction':
        target_args.append('--no-compress')
    # Include thread entry frames: excluding threading.py on CPython 3.14
    # also suppresses the receiver callback below Thread.run. Queue and JSON
    # frames separate pipe waiting from decoding in that receiver thread.
    includes = [str(ROOT/'tools'), str(ROOT/'backend/tests/macro_exact'),
                threading.__file__, queue.__file__, str(Path(json.__file__).parent)]
    extra_sources = []
    for index, arg in enumerate(forwarded):
        override = None
        if arg == '--module' and index+1 < len(forwarded):
            override = Path(forwarded[index+1]).resolve()
        elif arg.startswith('--module='):
            override = Path(arg.split('=',1)[1]).resolve()
        if override is not None:
            includes.append(str(override.parent))
            extra_sources.extend([override, *sorted(override.parent.glob('exact*.py'))])
    sources = [Path(__file__).resolve(), script, *sorted((ROOT/'tools').glob('exact*.py')), *extra_sources]
    source_hashes = {(str(path.relative_to(ROOT)) if path.is_relative_to(ROOT) else str(path)):
        hashlib.sha256(path.read_bytes()).hexdigest() for path in sources}
    tracer = VizTracer(tracer_entries=args.entries, verbose=0,
        include_files=includes,
        ignore_c_function=not args.include_c, ignore_frozen=True,
        min_duration=args.min_us, file_info=False,
        log_func_args=False, log_func_retval=False, log_print=False)
    code, failure = 0, None
    original_argv = sys.argv
    # Compile before starting, as VizTracer's own script runner does. On
    # CPython 3.14, tracing runpy.run_path with file filters can silently
    # produce only thread metadata and no function events.
    entry_code = compile(script.read_bytes(), str(script), 'exec')
    namespace = dict(__name__='__main__', __file__=str(script), __package__=None, __spec__=None)
    started = time.perf_counter()
    try:
        sys.argv = target_args
        tracer.start()
        exec(entry_code, namespace)
    except SystemExit as error:
        code = error.code if isinstance(error.code, int) else int(error.code is not None)
        failure = None if code == 0 else 'SystemExit'
    except KeyboardInterrupt:
        code, failure = 130, 'KeyboardInterrupt'
    except Exception as error:
        code, failure = 1, type(error).__name__
        raise
    finally:
        tracer.stop(stop_option='flush_as_finish')
        elapsed = time.perf_counter() - started
        sys.argv = original_argv
        tracer.parse()
        tracer.save(str(output/'trace.json'))
        tracer.save(str(output/'trace.html'))
        summary = summarize_trace(tracer.data)
        summary.update(phase=args.phase, runner_exit_code=code, failure_type=failure,
            profiled_wall_seconds=elapsed, viztracer_version=importlib.metadata.version('viztracer'),
            min_duration_us=args.min_us, entries=args.entries, include_c=args.include_c,
            includes=includes,
            trace_complete=not summary['overflow'] and summary['event_count'] > 0,
            argument_values_recorded=False, return_values_recorded=False, source_embedded=False,
            source_sha256=source_hashes,
            interpretation='Inclusive and residual observed wall time, not CPU time. Oracle.run includes Rust and IPC wait; receive includes blocking pipe reads. Filtered children remain in residual. Threads overlap. Profiling adds overhead.')
        outcome_path = output/'run'/('benchmark.json' if args.phase == 'compression' else 'report.json')
        if outcome_path.exists():
            outcome = json.loads(outcome_path.read_text(encoding='utf-8'))
            summary['runner_result'] = dict(status=outcome.get('status'),
                best_chars=outcome.get('summary',outcome.get('compression',{})).get('best_chars'),
                comparison=outcome.get('comparison'))
        (output/'summary.json').write_text(json.dumps(summary,ensure_ascii=False,indent=2),encoding='utf-8')
        (output/'summary.md').write_text(markdown_summary(summary),encoding='utf-8')
        print(json.dumps(dict(profile_summary=str(output/'summary.json'),
                              timeline=str(output/'trace.html'),overflow=summary['overflow'],
                              trace_complete=summary['trace_complete']),ensure_ascii=False))
    return code or (0 if summary['trace_complete'] else 2)


if __name__ == '__main__':
    raise SystemExit(main())
