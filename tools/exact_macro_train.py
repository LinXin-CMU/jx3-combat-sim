"""Offline native-feedback collection and anonymous structural transfer.

The corpus manifest owns source-family splits. Its scenes must already be
constructed from independent native rollouts and the source macro must pass
the complete frozen contract. Static syntax counts never become success labels.
This tool reads only explicit local corpus paths; it has no network/user API.
"""
import argparse
from collections import Counter, defaultdict
import hashlib
import importlib.util
import json
import math
from pathlib import Path
import random
import re
import subprocess
import sys
import time


ROOT = Path(__file__).resolve().parents[1]
STORE = ROOT / "backend/target/exact-macro-corpus"
_MODULES = {}


def _module(name, filename=None):
    if name not in _MODULES:
        spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename or name+".py"))
        value = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(value)
        _MODULES[name] = value
    return _MODULES[name]


def _hash(value):
    return hashlib.sha256(value.encode()).hexdigest()


def store_path(path):
    """Local corpus/trajectory artifacts must remain in the ignored store."""
    resolved = Path(path).resolve()
    try:
        resolved.relative_to(STORE.resolve())
    except ValueError:
        raise ValueError("offline corpus path must stay inside backend/target/exact-macro-corpus") from None
    if any(part.lower() in ("userdata", "agent_sessions") for part in resolved.parts):
        raise ValueError("real user data is not a corpus source/output")
    return resolved


def _write(path, value):
    path = store_path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2), encoding="utf-8")


def parse_macro(text, atoms=None, actions=None):
    """Single-page native chains only; unsupported syntax is rejected explicitly."""
    atoms, actions = list(atoms or []), list(actions or [])
    atom_index = {text: index for index, text in enumerate(atoms)}
    action_index = {(a["name"], bool(a["fcast"])): i for i, a in enumerate(actions)}
    rules = []
    for line in text.splitlines():
        line = line.strip()
        if not line:
            continue
        match = re.fullmatch(r"/(f?cast)\s+(.+)", line)
        if not match:
            raise ValueError("unsupported multi-page or non-cast corpus syntax")
        command, rest = match.groups()
        if rest.startswith("["):
            guarded = re.fullmatch(r"\[([^\[\]]+)\]\s+([^\[\]\s]+)", rest)
            if not guarded:
                raise ValueError("unsupported condition bracket syntax")
            guard, name = guarded.groups()
        elif any(c.isspace() for c in rest):
            guard, name = rest.rsplit(None, 1)
        else:
            guard, name = None, rest
        if any(c in name for c in "[]"):
            raise ValueError("unsupported action syntax")
        identity = name, command == "fcast"
        if identity not in action_index:
            action_index[identity] = len(actions)
            actions.append(dict(name=name, fcast=identity[1]))
        parts = re.split(r"([&|])", guard) if guard else []
        leaves, ops = parts[::2], parts[1::2]
        if any(not leaf or any(c in leaf for c in "()[] \t") for leaf in leaves):
            raise ValueError("unsupported condition leaf syntax")
        for leaf in leaves:
            if leaf not in atom_index:
                atom_index[leaf] = len(atoms)
                atoms.append(leaf)
        rule = dict(action=action_index[identity], atoms=[atom_index[leaf] for leaf in leaves])
        if ops:
            rule["ops"] = ops
        rules.append(rule)
    if not rules:
        raise ValueError("empty source macro")
    return rules, atoms, actions


def source_pages(text):
    """Preserve page boundaries for static counting; never flatten implicitly."""
    pages, current = [], []
    for line in text.splitlines():
        if line.strip().startswith("#page"):
            if current:
                pages.append("\n".join(current))
                current = []
            continue
        if line.strip():
            current.append(line)
    if current:
        pages.append("\n".join(current))
    return pages


def validate_manifest(data, base):
    """Check family isolation before reading any scene or generating labels."""
    base = store_path(base)
    if not isinstance(data, dict) or data.get("schema_version") != 1 or not isinstance(data.get("cases"), list):
        raise ValueError("unsupported corpus manifest")
    cases, records = data["cases"], data.get("records", [])
    if not isinstance(records, list):
        raise ValueError("invalid static records")
    reserved = set(data.get("reserved_source_families", [])) | {"holdout:fixed325", "fixed325"}
    families, ids = {}, set()
    for item in cases + records:
        if not isinstance(item, dict) or not all(item.get(k) for k in ("source_family", "split", "macro_path", "version", "mount")):
            raise ValueError("missing corpus source/applicability metadata")
        family, split = str(item["source_family"]), item["split"]
        if split not in ("train", "validation", "test"):
            raise ValueError("invalid corpus split")
        if family in families and families[family] != split:
            raise ValueError("source family crosses train/validation/test")
        families[family] = split
        if split == "train" and (family in reserved or item.get("holdout") or item.get("reserved")):
            raise ValueError("reserved holdout source cannot train")
        for key in ("macro_path", "scene_path"):
            if key in item:
                store_path(base / item[key])
        if item in cases:
            if not item.get("case_id") or not item.get("scene_path") or item["case_id"] in ids:
                raise ValueError("missing/duplicate training case")
            ids.add(item["case_id"])
    # Exact normalized copies in different splits are also leakage, even when
    # a manifest mistakenly gives them different family identifiers.
    bodies = {}
    for item in cases + records:
        path = store_path(base / item["macro_path"])
        text = path.read_text(encoding="utf-8-sig")
        canonical = "\n".join(" ".join(line.split()) for line in text.splitlines() if line.strip())
        digest = _hash(canonical)
        if digest in bodies and bodies[digest] != item["split"]:
            raise ValueError("duplicate source macro crosses splits")
        bodies[digest] = item["split"]
    return cases, records


def inflate(rules):
    """Reverse local edits, with no claim of safety until native certification.

    Duplicate a row and replace the rightmost leaf q by q&q. Right-associated
    chains retain their complete tree; no distributive/AND simplification is
    performed. Every inflated source is still replayed fully before use.
    """
    compression = _module("exact_macro_compress")
    result = []
    for rule in rules:
        value = compression.clone([rule])[0]
        if value.get("any_atoms"):
            value = dict(action=value["action"], **_module("exact_macro_conditions").normalize(value))
        if value["atoms"]:
            if "ops" not in value:
                value["ops"] = ["&"] * (len(value["atoms"])-1)
            value["atoms"].append(value["atoms"][-1])
            value["ops"].append("&")
        result.extend([value, compression.clone([rule])[0]])
    return result


def _perturbations(rules):
    """A fair finite mix of deletions and priority changes, including failures."""
    compression = _module("exact_macro_compress")
    queues = [[], [], []]
    for i, rule in enumerate(rules):
        for k in range(len(rule["atoms"])):
            trial = compression.clone(rules)
            trial[i] = compression.remove_chain_atom(rule, k)
            queues[0].append(("collect_remove_leaf", trial))
        queues[1].append(("collect_delete_rule", compression.clone(rules[:i]+rules[i+1:])))
        if i+1 < len(rules):
            trial = compression.clone(rules)
            trial[i], trial[i+1] = trial[i+1], trial[i]
            queues[2].append(("collect_priority_swap", trial))
    for turn in range(max(map(len, queues), default=0)):
        for queue in queues:
            if turn < len(queue):
                yield queue[turn]


def collect_case(case, base, oracle, oracle_version, out, *, max_trials=40, seconds=15):
    base, out = store_path(base), store_path(out)
    compression = _module("exact_macro_compress")
    learning = _module("exact_macro_learning")
    synth = _module("exact_macro_synth_train", "exact-macro-synth.py")
    scene = json.loads(store_path(base / case["scene_path"]).read_text(encoding="utf-8-sig"))
    if scene.get("version") != case["version"] or scene.get("mount") != case["mount"]:
        raise ValueError("case scene applicability differs from manifest")
    text = store_path(base / case["macro_path"]).read_text(encoding="utf-8-sig").strip()
    prepared = oracle.run(scene)
    if prepared.get("status") != "ok":
        raise ValueError("native target preparation failed")
    # Materialize the same environment once; never relax its endpoint/tolerance.
    scene["simulation"] = prepared["simulation"]
    contract = _hash(json.dumps(scene, sort_keys=True, separators=(",", ":")))
    case_dir = out / ("case-"+_hash(str(case["case_id"]))[:16])
    case_dir.mkdir(parents=True, exist_ok=True)
    # The original multi-page macro is an independently certified target
    # source, not a flattened compression baseline.
    original = oracle.run(dict(scene, candidate=text, atoms=prepared["atoms"]))
    if not compression.certified(original):
        raise ValueError("corpus original source failed complete native certification")
    synth.write(case_dir / "original-source-verified.json", original)
    start_method = "single_page_source"
    if any(line.strip().startswith("#page") for line in text.splitlines()):
        # Page-agnostic joining is merely ONE speculative native candidate.
        # It becomes usable only after a full replay on the unchanged contract.
        flat = "\n".join(source_pages(text))
        flat_result = oracle.run(dict(scene, candidate=flat, atoms=prepared["atoms"]))
        if compression.certified(flat_result):
            text, start_method = flat, "explicit_join_candidate_full_certified"
        else:
            frozen_path = case_dir / "frozen-scene.json"
            _write(frozen_path, scene)
            extraction = case_dir / "extraction"
            with (case_dir / "extraction.log").open("w", encoding="utf-8") as log:
                completed = subprocess.run([sys.executable, str(Path(__file__).with_name("exact-macro-synth.py")),
                    str(frozen_path), "--exe", str(oracle.proc.args[0]), "--out", str(extraction),
                    "--strategy", "prototype", "--no-compress", "--seconds", str(seconds)],
                    stdout=log, stderr=subprocess.STDOUT, cwd=ROOT, timeout=seconds+15,
                    creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
            exact = extraction / "exact.txt"
            if completed.returncode or not exact.exists():
                raise ValueError("single-page extraction did not obtain a certified source")
            text, start_method = exact.read_text(encoding="utf-8"), "stage_one_extracted_candidate"
    source, atoms, actions = parse_macro(text, prepared["atoms"], prepared["actions"])

    def replay(program, full=False):
        body = synth.macro_text(program, atoms, actions)
        result = oracle.run(dict(scene, candidate=body, atoms=atoms, stop_on_divergence=not full))
        if not full and result.get("comparison", {}).get("reproduced"):
            first = result.get("timings_ms", {}).get("round_trip", 0)
            result = oracle.run(dict(scene, candidate=body, atoms=atoms))
            result.setdefault("timings_ms", {})["verification_total"] = first+result["timings_ms"].get("round_trip", 0)
        return result

    baseline = replay(source, full=True)
    if not compression.certified(baseline):
        raise ValueError("corpus source macro failed complete native certification")
    expanded = inflate(source)
    inflated = replay(expanded, full=True)
    if not compression.certified(inflated):
        raise ValueError("reverse inflation failed complete native certification")
    synth.write(case_dir / "source-verified.json", baseline)
    synth.write(case_dir / "inflated-verified.json", inflated)
    source_text, expanded_text = synth.macro_text(source, atoms, actions), synth.macro_text(expanded, atoms, actions)
    for filename, body in (("source.txt", source_text), ("inflated.txt", expanded_text), ("exact.txt", expanded_text)):
        (case_dir / filename).write_text(body, encoding="utf-8")
    _write(case_dir / "atoms.json", atoms)
    _write(case_dir / "actions.json", actions)
    _write(case_dir / "compression-contract.json", dict(scene=scene,
        baseline_macro_sha256=_hash(expanded_text), comparison=inflated["comparison"],
        baseline_archive="inflated-verified.json", source_group=str(case["source_family"]),
        source_method=start_method, character_count="UTF-16 code units including brackets/spaces/newlines"))
    session = learning.LearningSession(contract, oracle_version, str(case["source_family"]),
                                       enabled=False, training=False, split=case["split"])
    examples, seen = [], set()
    started = time.perf_counter()

    def check():
        if time.perf_counter()-started >= seconds or len(examples) >= max_trials:
            raise TimeoutError("offline case observation ended")

    def observe(kind, candidate, guide, before):
        body = synth.macro_text(candidate, atoms, actions)
        identity = _hash(body)
        if identity in seen:
            return
        seen.add(identity)
        context = guide.learning_context(candidate)
        values = learning.features(guide.source_rules, candidate, atoms, before, compression.char_count(body), context)
        record = session.prepare(identity, guide.path_id, kind, values, before, compression.char_count(body))
        result = replay(candidate)
        feedback = session.feedback(record, result, binding=record["binding"])
        examples.append(dict(case_id=str(case["case_id"]), source_family=str(case["source_family"]),
            split=case["split"], version=case["version"], mount=case["mount"], record=record, feedback=feedback))

    def guide_for(result, rules):
        guide = compression.Samples(result["rows"], atoms, actions, check,
            dict(contract_id=contract, macro_sha256=_hash(synth.macro_text(rules, atoms, actions)), kind="certified_training_source"))
        guide.source_rules = compression.clone(rules)
        return guide

    try:
        # Both sources retain their own complete states and path identities.
        # Original risky edits supply genuine negatives; inflation edits supply
        # realistic safe compression examples rather than static fake labels.
        guides = [(source, guide_for(baseline, source)), (expanded, guide_for(inflated, expanded))]
        streams = [iter(_perturbations(program)) for program, _ in guides]
        remaining = [True, True]
        while any(remaining):
            for i, (program, guide) in enumerate(guides):
                check()
                if not remaining[i]:
                    continue
                item = next(streams[i], None)
                if item is None:
                    remaining[i] = False
                    continue
                kind, candidate = item
                observe(kind, candidate, guide, compression.char_count(synth.macro_text(program, atoms, actions)))
    except TimeoutError:
        pass
    _write(case_dir / "examples.json", examples)
    return examples, dict(case_id=case["case_id"], split=case["split"], source_family=case["source_family"],
        contract_hash=contract, source_chars=compression.char_count(synth.macro_text(source, atoms, actions)),
        inflated_chars=compression.char_count(synth.macro_text(expanded, atoms, actions)),
        target_count=baseline["comparison"]["target_count"], labels=Counter(e["feedback"]["category"] for e in examples),
        start_method=start_method, baseline_directory=str(case_dir.resolve()),
        collection_seconds=time.perf_counter()-started)


def _validated_examples(examples):
    learning = _module("exact_macro_learning")
    families, seen = {}, set()
    result = []
    for example in examples:
        family, split = str(example["source_family"]), example["split"]
        if split not in ("train", "validation", "test") or family in families and families[family] != split:
            raise ValueError("example source family crosses splits")
        families[family] = split
        record, feedback = example["record"], example["feedback"]
        if record["binding"] != feedback["binding"] or record["binding"]["source_group"] != family:
            raise ValueError("example feedback provenance mismatch")
        if set(record["binding"]) != set(learning.BINDING_NAMES) or any(not str(v) for v in record["binding"].values()):
            raise ValueError("missing example provenance")
        learning._vector(record["pre_replay_features"], learning.FEATURE_NAMES)
        label, category = feedback["label"], feedback["category"]
        expected = {"certified": 1, "native_full_failure": 0, "native_divergence": 0}.get(category)
        if label != expected:
            raise ValueError("label disagrees with native feedback category")
        identity = tuple(record["binding"][key] for key in learning.BINDING_NAMES)
        if identity in seen:
            continue
        seen.add(identity)
        if label is not None:
            result.append(example)
    return result


def fit_prior(examples, prior, *, epochs=48, seed=0, structural_families=()):
    """Fit train verdicts and bind every train-family structural contributor.

    Static families have no fabricated labels. They still belong to training
    provenance when collection fails or yields no known native verdict.
    """
    learning = _module("exact_macro_learning")
    examples = list(examples)
    if isinstance(structural_families, (str, bytes)):
        raise ValueError("structural families must be a collection of train identities")
    structural_families = set(structural_families)
    if any(not isinstance(family, str) or not family for family in structural_families):
        raise ValueError("invalid structural family identity")
    if structural_families & {str(e["source_family"]) for e in examples if e["split"] != "train"}:
        raise ValueError("structural source family crosses train/validation/test")
    examples = _validated_examples(examples)
    train = [e for e in examples if e["split"] == "train"]
    if not train:
        raise ValueError("no genuine train-split native labels")
    if {e["feedback"]["label"] for e in train} != {0, 1}:
        raise ValueError("trained predictor requires both real native label classes")
    weights = [-0.5]+[0.0]*(len(learning.FEATURE_NAMES)-1)
    costs = [math.log1p(100)]+[0.0]*(len(weights)-1)
    cost_examples = [e for e in train if e["feedback"]["validation_cost_ms"] is not None]
    rng = random.Random(seed)
    for epoch in range(max(1, int(epochs))):
        order = list(train)
        rng.shuffle(order)
        for example in order:
            vector = learning._vector(example["record"]["pre_replay_features"], learning.FEATURE_NAMES)
            rate = 0.6/(1+sum(v*v for v in vector))/math.sqrt(1+epoch/8)
            error = example["feedback"]["label"]-learning._sigmoid(learning._dot(weights, vector))
            weights = [w+rate*(error*v-0.002*w) for w, v in zip(weights, vector)]
            elapsed = example["feedback"]["validation_cost_ms"]
            if elapsed is not None:
                elapsed = learning._finite(elapsed)
                if elapsed < 0:
                    raise ValueError("negative native verification time")
                error = max(-3.0, min(3.0, math.log1p(elapsed)-learning._dot(costs, vector)))
                costs = [w+rate*(error*v-0.002*w) for w, v in zip(costs, vector)]
    families = sorted({_hash(family) for family in structural_families}
                      | {_hash(e["source_family"]) for e in train})
    contexts = sorted({(e["version"], e["mount"]) for e in train})
    positives = sum(e["feedback"]["label"] for e in train)
    result = dict(schema="exact-macro-offline-prior", prior_version=learning.PRIOR_VERSION,
        feature_schema_version=learning.FEATURE_SCHEMA_VERSION, feature_names=list(learning.FEATURE_NAMES),
        model_version=learning.MODEL_VERSION, native_semantics=learning.NATIVE_SEMANTICS,
        oracle_versions=sorted({e["record"]["binding"]["oracle_version"] for e in train}),
        applicable_contexts=[dict(version=v, mount=m) for v, m in contexts],
        weights=weights, cost_weights=costs, trained_labels=len(train), cost_labels=len(cost_examples),
        cost_log_mean=sum(math.log1p(e["feedback"]["validation_cost_ms"]) for e in cost_examples)/max(1, len(cost_examples)),
        min_labels=1, structural_prior=learning.validate_structural_prior(prior),
        training_summary=dict(train_family_hashes=families, train_examples=len(train),
                              positive_labels=positives, negative_labels=len(train)-positives))
    # Exercise the exact runtime parser before writing a model artifact.
    learning.LearningSession.from_prior(result, contract_hash="export-check", oracle_version=result["oracle_versions"][0],
        source_group="validation:export", version=contexts[0][0], mount=contexts[0][1])
    return result


def evaluate_prior(data, examples, split="validation"):
    """Frozen held-out predictions/rank proxies, never a runtime speed claim."""
    learning = _module("exact_macro_learning")
    examples = [e for e in _validated_examples(examples) if e["split"] == split]
    predictions, groups = [], defaultdict(list)
    for example in examples:
        record = example["record"]
        session = learning.LearningSession.from_prior(data, contract_hash=record["binding"]["contract_hash"],
            oracle_version=record["binding"]["oracle_version"], source_group=example["source_family"],
            version=example["version"], mount=example["mount"])
        probability, cost, _ = session._predict(record["pre_replay_features"])
        baseline = learning.LearningSession("heuristic", record["binding"]["oracle_version"], training=False)
        base_probability, base_cost, _ = baseline._predict(record["pre_replay_features"])
        saving = max(0, record["chars_before"]-record["chars_after"])
        item = dict(candidate_hash=record["binding"]["candidate_hash"], path_id=record["binding"]["path_id"],
            source_family_hash=_hash(example["source_family"]), label=example["feedback"]["label"],
            probability=probability, heuristic_probability=base_probability, predicted_cost_ms=cost,
            learned_score=probability*saving/cost, heuristic_score=base_probability*saving/base_cost,
            static_screen=bool(record['pre_replay_features']['static_compatible'])
                          if record['pre_replay_features']['static_compatible_known'] else None,
            saving=saving, measured_cost_ms=example["feedback"]["validation_cost_ms"])
        predictions.append(item)
        groups[record["binding"]["path_id"]].append(item)
    def metrics(key):
        n = max(1, len(predictions))
        positives = [r[key] for r in predictions if r["label"]]
        negatives = [r[key] for r in predictions if not r["label"]]
        return dict(log_loss=-sum(r["label"]*math.log(max(1e-9, r[key]))+(1-r["label"])*math.log(max(1e-9, 1-r[key])) for r in predictions)/n,
            brier=sum((r[key]-r["label"])**2 for r in predictions)/n,
            accuracy=sum((r[key]>=0.5)==bool(r["label"]) for r in predictions)/n,
            auc=(sum((p>q)+0.5*(p==q) for p in positives for q in negatives)/(len(positives)*len(negatives))) if positives and negatives else None)
    ranking = {}
    for key in ("learned_score", "heuristic_score"):
        chosen = [max(rows, key=lambda r: (r[key], r["candidate_hash"])) for rows in groups.values()]
        ranking[key] = dict(groups=len(chosen), top1_pass_fraction=sum(r["label"] for r in chosen)/max(1, len(chosen)),
            top1_certified_saving=sum(r["saving"]*r["label"] for r in chosen))
    screened = [r for r in predictions if r['static_screen'] is not None]
    static_metrics = dict(labels=len(screened),
        accuracy=sum(r['static_screen']==bool(r['label']) for r in screened)/len(screened) if screened else None,
        brier=sum((int(r['static_screen'])-r['label'])**2 for r in screened)/len(screened) if screened else None)
    return dict(split=split, labels=len(predictions), families=len({e["source_family"] for e in examples}),
        frozen=True, learned=metrics("probability"), heuristic=metrics("heuristic_probability"),
        static_screen=static_metrics, ranking=ranking, predictions=predictions)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--exe", type=Path, default=ROOT/"backend/target/release/jx3-combat-sim.exe")
    parser.add_argument("--max-trials", type=int, default=40)
    parser.add_argument("--seconds-per-case", type=float, default=15)
    parser.add_argument("--epochs", type=int, default=48)
    args = parser.parse_args()
    if args.max_trials <= 0 or args.seconds_per_case <= 0 or not math.isfinite(args.seconds_per_case):
        parser.error("positive finite observation limits required")
    args.manifest, args.out = store_path(args.manifest), store_path(args.out)
    manifest = json.loads(args.manifest.read_text(encoding="utf-8-sig"))
    base = args.manifest.resolve().parent
    cases, records = validate_manifest(manifest, base)
    _write(args.out/"manifest-snapshot.json", manifest)
    learning = _module("exact_macro_learning")
    programs = []
    structural_families = set()
    seen = set()
    for item in records or cases:
        if item["split"] != "train":
            continue
        text = (base/item["macro_path"]).read_text(encoding="utf-8-sig")
        identity = item["source_family"], _hash("\n".join(" ".join(line.split()) for line in text.splitlines() if line.strip()))
        if identity in seen:
            continue
        seen.add(identity)
        for page in source_pages(text):
            rules, atoms, _ = parse_macro(page)
            programs.append((rules, atoms))
            structural_families.add(str(item["source_family"]))
    prior = learning.structural_prior(programs)
    _write(args.out/"structural-prior.json", prior)
    oracle_version = hashlib.sha256(args.exe.read_bytes()).hexdigest()
    synth = _module("exact_macro_synth_train", "exact-macro-synth.py")
    oracle = synth.Oracle(args.exe.resolve())
    examples, summaries, rejected = [], [], []
    try:
        for case in cases:
            try:
                batch, summary = collect_case(case, base, oracle, oracle_version, args.out,
                    max_trials=args.max_trials, seconds=args.seconds_per_case)
                examples.extend(batch)
                summaries.append(summary)
                print(json.dumps(summary, ensure_ascii=False), flush=True)
            except (ValueError, subprocess.TimeoutExpired) as error:
                rejected.append(dict(case_id=case["case_id"], reason=str(error)))
    finally:
        oracle.close()
        _write(args.out/"examples.json", examples)
        _write(args.out/"collection.json", dict(cases=summaries, rejected=rejected, manifest_sha256=hashlib.sha256(args.manifest.read_bytes()).hexdigest()))
    model = fit_prior(examples, prior, epochs=args.epochs, structural_families=structural_families)
    _write(args.out/"model.json", model)
    validation = evaluate_prior(model, examples)
    testing = evaluate_prior(model, examples, "test")
    _write(args.out/"validation.json", validation)
    _write(args.out/"test.json", testing)
    print(json.dumps(dict(model=str(args.out/"model.json"), train=model["training_summary"],
        validation={k:v for k,v in validation.items() if k!="predictions"}), ensure_ascii=False), flush=True)


if __name__ == "__main__":
    main()
