"""Local HTTP job adapter. stdin controls pause/resume/stop; stdout is JSON progress.

The server supplies fixed file paths, never shell commands. No total time limit.
Evidence stays in the server's per-job records directory. No user downloads.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import sys
import threading
import time


def emit(value):
    print(json.dumps(value, ensure_ascii=False), flush=True)


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser()
    parser.add_argument("scene", type=Path)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--compress", action=argparse.BooleanOptionalAction, default=True)
    args = parser.parse_args()
    try:
        spec = importlib.util.spec_from_file_location("exact_synth", Path(__file__).with_name("exact-macro-synth.py"))
        synth = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(synth)
    except ImportError:
        emit({"phase": "failed", "status": "dependency_missing", "reason": "精确合成需要 Python 的 z3-solver；请按精确合成文档安装依赖。"})
        return

    def cancel():
        # EOF also cancels: the server exiting must not leave an orphan solver.
        for line in sys.stdin:
            command = line.strip()
            if command == "pause":
                synth.RUNNING.clear()
            elif command == "resume":
                synth.RUNNING.set()
            elif command == "cancel":
                break
        synth.CANCELLED.set()
        synth.RUNNING.set()
        synth.interrupt_active_solver()

    def interrupt_paused_check():
        # Repeated observation closes the race between a pause command and
        # entering solver.check. No time limit is imposed on an active check.
        while True:
            if synth.CANCELLED.is_set() or not synth.RUNNING.is_set():
                synth.interrupt_active_solver()
            time.sleep(0.05)

    threading.Thread(target=cancel, daemon=True).start()
    threading.Thread(target=interrupt_paused_check, daemon=True).start()
    args.strategy = "prototype"
    args.sizes = ""  # The prototype constructs as many rules/terms as needed.
    args.solver_ms = 0  # Z3: unlimited; pause/stop interrupts the active check.
    args.iterations = sys.maxsize
    args.seconds = float("inf")
    args.web = True
    try:
        synth.run_job(args)
    except Exception:
        # run_job already records a safe structured report, including any
        # exact certificate obtained before compression stopped.
        if not (args.out / "report.json").exists():
            emit({"phase": "failed", "status": "error", "reason": "求解进程未能建立报告。"})
            return
    report = json.loads((args.out / "report.json").read_text(encoding="utf-8"))
    if (args.out / "best.json").exists():
        report["comparison"] = json.loads((args.out / "best.json").read_text(encoding="utf-8"))["comparison"]
    best = args.out / "compact-verified.json"
    macro = args.out / "compact.txt"
    if not best.exists():
        best, macro = args.out / "verified.json", args.out / "exact.txt"
    if not best.exists():
        best = args.out / "best-replay.json"
        if (args.out / "best.json").exists():
            saved = json.loads((args.out / "best.json").read_text(encoding="utf-8"))
            macro = args.out / "best.txt"
            macro.write_text(saved["macro"], encoding="utf-8")
        if not best.exists():
            best = args.out / "target.json"
    # The report/trajectories remain on disk. Only UI fields cross HTTP.
    result = {"report": {k:report[k] for k in ("status", "reason", "comparison", "total_solve_ms", "compression", "compression_stop") if k in report},
              "macro": macro.read_text(encoding="utf-8") if macro.exists() else None}
    (args.out / "result.json").write_text(json.dumps(result, ensure_ascii=False), encoding="utf-8")
    emit({"phase": "finished", "status": report["status"]})


if __name__ == "__main__":
    main()
