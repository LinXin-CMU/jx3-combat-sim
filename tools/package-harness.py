#!/usr/bin/env python3
"""Build a local Windows ZIP from an explicit runtime allowlist (Python 3.11+).

Does not build, upload, deploy, or inspect user/provider configuration. --dry-run
validates inputs without creating output; --self-test uses synthetic inputs only.
The caller runs the release build and acceptance checks first.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import sys
import tempfile
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]
VERSION = '2.1.0-20261004.3'
ZIP_TIME = (1980, 1, 1, 0, 0, 0)
PORT_LAUNCH = (
    "$port = 0; if ($env:JX3_PORT -notmatch '\\A[0-9]{1,5}\\z' -or "
    "-not [int]::TryParse($env:JX3_PORT, [ref]$port) -or $port -lt 1 -or $port -gt 65535) "
    "{ [Console]::Error.WriteLine('JX3_PORT must be an integer from 1 to 65535.'); exit 2 }; "
    "$env:JX3_PORT = [string]$port; Write-Host ('Local server: http://127.0.0.1:' + $port); "
    "& '.\\jx3-combat-sim.exe'; exit $LASTEXITCODE"
)
MAX_FILE_BYTES = 256 * 1024 * 1024
MAX_TOTAL_BYTES = 512 * 1024 * 1024
# New UI assets must be reviewed and added here. Never recurse through frontend,
# docs, config, userdata, old archives, or raw equipment input directories.
FRONTEND_FILES = (
    'agent-provider-settings.js', 'agent.css', 'agent.js', 'app.js',
    'assistant-shell.css', 'assistant-shell.js', 'favicon.ico',
    'harness-run.js', 'harness-workspace.js', 'harness.css', 'harness.js',
    'index.html', 'login.html', 'macro-alignment.js', 'macro-assist.css',
    'macro-assist.js', 'macro-diagnostic.js', 'macro-draft-panes.js',
    'macro-editor.js', 'macro-layout.js', 'macro-repair-group.js',
    'macro-repair-panel.js', 'macro-repair-search.js', 'macro-repair.js',
    'macro-state-diff.js', 'shield-reset.js', 'skill-damage-description.js',
    'state.js', 'style.css',
    'macro-exact.js', 'macro-exact.css',
)
VERSIONS = ('2025_10_山海源流', '2026_04_暗影千机', '2026_10_苍生铸世测试服')
MOUNTS = ('分山劲', '铁骨衣')
DOC_FILES = ('EXACT_MACRO_SYNTHESIS.md', 'HARNESS_V2.md', 'baselines/2026-09-22-harness-v2.md',
             'portfolio/HARNESS_CASE.md')
EXACT_FILES = ('exact-macro-synth.py', 'exact-macro-worker.py', 'exact_macro_compress.py',
               'exact_macro_reorder.py', 'exact_macro_conditions.py', 'exact_macro_global.py', 'exact_macro_family.py', 'exact_macro_joint.py', 'exact_macro_repair.py',
               'exact_macro_region.py', 'exact_macro_region_sat.py', 'exact_macro_region_guards.py', 'exact_macro_learning.py', 'exact-macro-runtime-files.json',
               'requirements-exact-macro.txt')
# Only immediate numeric-ID runtime definitions in six approved directories.
# Templates and arbitrary nested directories never qualify.
SKILL_FILE = re.compile(r'[1-9][0-9]{0,9}_[\w·]+\.toml\Z')
TEXT_SUFFIXES = {'.toml', '.json', '.js', '.html', '.css', '.md', '.txt', '.cmd', '.py'}
TOKEN_PATTERN = re.compile(
    rb'\b(?:sk-[A-Za-z0-9_-]{20,}|gh[pousr]_[A-Za-z0-9_]{20,}|'
    rb'github_pat_[A-Za-z0-9_]{20,}|AKIA[0-9A-Z]{16})\b|'
    rb'-----BEGIN (?:[A-Z]+ )?PRIVATE KEY-----'
)
SECRET_ASSIGNMENT = re.compile(
    r'''(?i)["']?\b(?:api[_-]?key|access[_-]?token|refresh[_-]?token|client[_-]?secret|password)["']?\s*[:=]\s*["']([^"'\r\n]{8,})["']'''
)


class PackageError(Exception):
    """Safe diagnostic: no source contents or secret values."""


def safe_path(path, root, *, directory=False):
    """Reject escapes, symlinks, junctions, hard links and non-regular inputs."""
    path, root = Path(path).absolute(), Path(root).absolute()
    try:
        relative = path.relative_to(root)
    except ValueError:
        raise PackageError('Input is outside the approved root.') from None
    if '..' in relative.parts:
        raise PackageError('Input is outside the approved root.')
    current = root
    for part in (None, *relative.parts):
        if part is not None:
            current /= part
        try:
            info = current.lstat()
        except FileNotFoundError:
            raise PackageError(f'Required runtime input is missing: {relative.as_posix()}') from None
        if stat.S_ISLNK(info.st_mode) or getattr(info, 'st_file_attributes', 0) & 0x400:
            raise PackageError(f'Linked/reparse input is not allowed: {relative.as_posix()}')
        if current != path and not stat.S_ISDIR(info.st_mode):
            raise PackageError('An input parent is not a directory.')
    expected = stat.S_ISDIR if directory else stat.S_ISREG
    if not expected(info.st_mode):
        raise PackageError(f'Invalid runtime input type: {relative.as_posix()}')
    # Cargo itself hard-links its final executable to the deps artifact. Only
    # that exact build output is exempt; runtime data/config cannot alias saves.
    cargo_binary = relative.as_posix() == 'backend/target/release/jx3-combat-sim.exe'
    if not directory and info.st_nlink != 1 and not cargo_binary:
        raise PackageError(f'Hard-linked runtime input is not allowed: {relative.as_posix()}')
    return path


def archive_name(name):
    path = PurePosixPath(name)
    if (not name or path.is_absolute() or path.as_posix() != name
            or any(part in ('', '.', '..') or ':' in part or '\\' in part
                   or part.endswith(('.', ' ')) for part in path.parts)):
        raise PackageError('Invalid archive entry name.')
    return name


def discover_sources(root):
    """No fallback to source tables, old catalogs, local config, or saved state."""
    sources = {'backend/jx3-combat-sim.exe': root / 'backend/target/release/jx3-combat-sim.exe'}
    for name in FRONTEND_FILES:
        sources['frontend/' + name] = root / 'frontend' / name
    for name in DOC_FILES:
        sources['docs/' + name] = root / 'docs' / name
    for name in EXACT_FILES:
        sources['tools/' + name] = root / 'tools' / name
    for level in ('level130', 'level50'):
        name = f'backend/data/{level}/equip.json'
        sources[name] = root / name
    for version in VERSIONS:
        shared = ('recipes.toml',) if version == VERSIONS[0] else (
            'recipes.toml', 'team_buffs.toml', 'formations.toml')
        for name in shared:
            rel = f'backend/data/{version}/{name}'
            sources[rel] = root / rel
        for mount in MOUNTS:
            prefix = f'backend/data/{version}/{mount}'
            names = ('school.toml', 'talents.toml', 'defaults.json')
            if version == VERSIONS[2] and mount == '铁骨衣':
                names += ('recipes.toml', 'team_buffs.toml')
            for name in names:
                sources[f'{prefix}/{name}'] = root / prefix / name
            folder = safe_path(root / prefix / 'skills', root, directory=True)
            skills = sorted(p for p in folder.iterdir() if SKILL_FILE.fullmatch(p.name))
            if not skills:
                raise PackageError(f'No approved runtime skills in {prefix}.')
            for path in skills:
                sources[f'{prefix}/skills/{path.name}'] = path
    for path in sources.values():
        safe_path(path, root)
    return sources


def scan_payload(name, data):
    if TOKEN_PATTERN.search(data):
        raise PackageError(f'Potential credential in approved input: {name}; value suppressed.')
    if PurePosixPath(name).suffix in TEXT_SUFFIXES:
        try:
            text = data.decode('utf-8-sig')
        except UnicodeDecodeError:
            raise PackageError(f'Runtime text must be UTF-8: {name}') from None
        for match in SECRET_ASSIGNMENT.finditer(text):
            value = match.group(1)
            if not value.startswith(('${', '{{')) and not set(value) <= {'*', '•'}:
                raise PackageError(f'Potential literal credential in {name}; value suppressed.')
        if re.search(r'https?://[^/\s\"\']+:[^/\s\"\']+@', text):
            raise PackageError(f'URL credentials in {name}; value suppressed.')
        try:
            if name.endswith('.toml'):
                tomllib.loads(text)
            elif name.endswith('.json'):
                json.loads(text)
        except (ValueError, tomllib.TOMLDecodeError):
            raise PackageError(f'Invalid runtime data syntax: {name}') from None


def generated_files():
    # Fixed public endpoints and env names only: no build-machine config reads.
    profiles = ['# Generated public example. Never put a credential in this file.\n']
    for variant in ('flash', 'pro'):
        profiles.append(f'''[[profiles]]
id = "deepseek-v4-{variant}"
label = "DeepSeek V4 {variant.title()}"
adapter = "openai_compatible_chat"
model = "deepseek-v4-{variant}"
base_url = "https://api.deepseek.com"
api_key_env = "JX3_DEEPSEEK_API_KEY"
chat_compatibility = "deepseek"
enabled = true
''')
    profiles.append('''[[profiles]]
id = "offline"
label = "Offline protocol fixture"
adapter = "fake"
model = "deterministic-fixture-v1"
enabled = true
''')
    # Run inside backend: ./data and ../frontend always refer to this package.
    # Quoted assignments + delayed expansion disabled also support !/& in paths.
    launcher = '''@echo off
setlocal EnableExtensions DisableDelayedExpansion
if not exist "%~dp0backend\\jx3-combat-sim.exe" goto missing
if not exist "%~dp0frontend\\index.html" goto missing
if not exist "%~dp0backend\\data\\level130\\equip.json" goto missing
if not exist "%~dp0backend\\data\\level50\\equip.json" goto missing
if not exist "%~dp0config\\agent.providers.example.toml" goto missing
set "JX3_BIND=127.0.0.1"
if not defined JX3_PORT set "JX3_PORT=3318"
set "JX3_AGENT_CONFIG=%~dp0config\\agent.providers.example.toml"
set "JX3_USERDATA_DIR=%~dp0userdata"
set "JX3_ICON_CACHE_DIR=%~dp0userdata\\icon_cache"
set "JX3_ROUTER="
set "JX3_NO_BROWSER="
set "JX3_PUBLIC_DEPLOYMENT="
set "JX3_AUTH_PASSWORD="
set "JX3_KNOWLEDGE_ROOT="
set "JX3_KNOWLEDGE_CACHE_DIR="
set "ERRORLEVEL="
pushd "%~dp0backend" || goto missing
echo Keep this window open. Press Ctrl+C to stop the server.
"%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe" -NoLogo -NoProfile -NonInteractive -Command "__PORT_LAUNCH__"
set "HARNESS_EXIT_CODE=%ERRORLEVEL%"
popd
if "%HARNESS_EXIT_CODE%"=="0" exit /b 0
echo Server stopped with an error. Check the message above and the chosen port.
pause
exit /b %HARNESS_EXIT_CODE%
:missing
echo Release files are missing or inaccessible. Extract the entire ZIP first.
pause
exit /b 1
'''
    readme = '''苍云器灵 2.1.0 · 2026-09-22

Windows：将整个压缩包解压到新的可写目录，双击根目录 start.cmd。
启动器默认只监听本机 127.0.0.1:3318；不要直接运行 backend 内的 exe。
可提前设置 JX3_PORT 环境变量为 1..65535 的整数以更换端口；仍只监听本机。
保留启动窗口，Ctrl+C 停止。端口已占用时会明确报错，请勿重复启动。
点击悬浮球进入武学助手，原 AI 分析仍可并行使用。

在模型设置中配置自己的 DeepSeek 接口与 Key；Key 只保留在当前进程内存。
也可通过 JX3_DEEPSEEK_API_KEY 环境变量供内置 Flash / Pro 配置使用。
包内没有预置 Key、私人知识库、原始数据资料或既有用户数据。
离线 provider 仅用于协议测试，不具备真实模型推理能力。
启动器忽略外部部署/知识库路径，所有新用户数据写入本包的 userdata。
不要覆盖旧版本目录；迁移自己的数据前先备份，不要分享 userdata。
首次图标请求及模型调用需要网络；知识库检索未包含在本地包内。

详见 docs/HARNESS_V2.md 与 docs/baselines/2026-09-22-harness-v2.md。
有限搜索不证明全局最优。候选应用前可预览差异，应用后可撤销。
本包用于本机运行，不能直接作为公网部署包。
manifest.json 列出全部内容文件的 SHA-256（清单自身不自我列入）。
'''
    return {
        'start.cmd': launcher.replace('__PORT_LAUNCH__', PORT_LAUNCH).replace('\n', '\r\n').encode('ascii'),
        'README.txt': readme.encode('utf-8-sig'),
        'config/agent.providers.example.toml': '\n'.join(profiles).encode('utf-8'),
    }


def collect_payloads(root):
    sources = discover_sources(root)
    payloads = generated_files()
    total = sum(map(len, payloads.values()))
    for name, path in sorted(sources.items()):
        safe_path(path, root)
        before = path.stat()
        if before.st_size > MAX_FILE_BYTES:
            raise PackageError(f'Runtime input exceeds size limit: {name}')
        data = path.read_bytes()
        after = path.stat()
        if (before.st_size, before.st_mtime_ns, before.st_ino) != (
                after.st_size, after.st_mtime_ns, after.st_ino) or len(data) != before.st_size:
            raise PackageError(f'Input changed during packaging: {name}; retry after builds finish.')
        safe_path(path, root)
        total += len(data)
        if total > MAX_TOTAL_BYTES:
            raise PackageError('Approved inputs exceed the total package size limit.')
        payloads[name] = data
    seen = set()
    for name, data in payloads.items():
        archive_name(name)
        if name.casefold() in seen:
            raise PackageError('Duplicate archive entry on Windows.')
        seen.add(name.casefold())
        scan_payload(name, data)
    if not payloads['backend/jx3-combat-sim.exe'].startswith(b'MZ'):
        raise PackageError('Release executable is not Windows PE; run the Windows release build.')
    # Catch a new HTML/bootstrap dependency absent from the explicit UI list.
    for page in ('index.html', 'login.html'):
        text = payloads['frontend/' + page].decode('utf-8-sig')
        for match in re.finditer(r'''["'](?:\./)?([^"'\s<>]+\.(?:js|css|ico))(?:\?[^"']*)?["']''', text):
            name = match.group(1)
            if '://' not in name and 'frontend/' + name not in payloads:
                raise PackageError('An HTML asset is absent from the explicit frontend allowlist.')
    return payloads


def manifest_bytes(payloads):
    files = [{'path': name, 'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()}
             for name, data in sorted(payloads.items())]
    manifest = {'schema': 'harness-local-release/v2', 'version': VERSION,
                'manifest_excludes_itself': True, 'files': files}
    return (json.dumps(manifest, ensure_ascii=False, sort_keys=True, indent=2) + '\n').encode('utf-8')


def verify_archive(path, payloads, manifest):
    with zipfile.ZipFile(path) as archive:
        expected = dict(payloads, **{'manifest.json': manifest})
        if archive.namelist() != sorted(expected) or archive.testzip() is not None:
            raise PackageError('ZIP entry/CRC verification failed.')
        for name, data in expected.items():
            info = archive.getinfo(name)
            if (info.date_time != ZIP_TIME or info.file_size != len(data)
                    or hashlib.sha256(archive.read(name)).digest() != hashlib.sha256(data).digest()):
                raise PackageError('ZIP payload verification failed.')


def check_output(output):
    output = output.absolute()
    if output.suffix.lower() != '.zip':
        raise PackageError('Output must use the .zip extension.')
    # Reject dangling links too. Never clobber old releases or traverse junctions.
    for path in reversed((output, *output.parents)):
        try:
            info = path.lstat()
        except FileNotFoundError:
            continue
        if stat.S_ISLNK(info.st_mode) or getattr(info, 'st_file_attributes', 0) & 0x400:
            raise PackageError('Output path must not contain links or reparse points.')
        if path == output:
            raise PackageError('Output already exists; choose a new filename to preserve it.')
        if not stat.S_ISDIR(info.st_mode):
            raise PackageError('Output parent is not a directory.')
    return output


def write_package(output, payloads):
    """Stage, verify, then publish atomically without replacing existing files.

    Manifest bytes are independent of the machine, clock and output path. ZIP
    bytes are reproducible for identical inputs using the same Python/zlib build.
    """
    output = check_output(output)
    manifest = manifest_bytes(payloads)
    output.parent.mkdir(parents=True, exist_ok=True)
    check_output(output)
    descriptor, staging = tempfile.mkstemp(prefix='.harness-package-', suffix='.tmp', dir=output.parent)
    staging = Path(staging)
    try:
        with os.fdopen(descriptor, 'w+b') as stream:
            with zipfile.ZipFile(stream, 'w', compression=zipfile.ZIP_DEFLATED, compresslevel=6) as archive:
                for name, data in sorted(dict(payloads, **{'manifest.json': manifest}).items()):
                    info = zipfile.ZipInfo(archive_name(name), ZIP_TIME)
                    info.create_system = 0
                    info.external_attr = 0x20
                    archive.writestr(info, data, compress_type=zipfile.ZIP_DEFLATED, compresslevel=6)
            stream.flush()
            os.fsync(stream.fileno())
        verify_archive(staging, payloads, manifest)
        with staging.open('rb') as stream:
            digest = hashlib.file_digest(stream, 'sha256').hexdigest()
        size = staging.stat().st_size
        # Windows rename is atomic and refuses existing destinations. On POSIX,
        # use an exclusive same-filesystem hard link instead of overwriting rename.
        if os.name == 'nt':
            os.rename(staging, output)
        else:
            os.link(staging, output)
        return {'path': str(output), 'files': len(payloads) + 1, 'bytes': size,
                'sha256': digest, 'manifest_sha256': hashlib.sha256(manifest).hexdigest()}
    finally:
        # Remove only this invocation's exact staging file, including on errors
        # or Ctrl+C. Failed validation never leaves a partial release ZIP.
        staging.unlink(missing_ok=True)


def self_test():
    """Synthetic files only: never reads real binary/data/config/userdata."""
    import unittest
    from unittest.mock import patch

    class PackagingTests(unittest.TestCase):
        def setUp(self):
            self.temp = tempfile.TemporaryDirectory(prefix='harness-package-test-')
            self.addCleanup(self.temp.cleanup)
            self.root = Path(self.temp.name)
            self.payloads = {'README.txt': b'fixture', 'frontend/index.html': b'<html></html>'}

        def test_reproducible_zip_and_manifest(self):
            first, second = self.root / 'first.zip', self.root / 'second.zip'
            a = write_package(first, self.payloads)
            b = write_package(second, dict(reversed(list(self.payloads.items()))))
            self.assertEqual(a['sha256'], b['sha256'])
            self.assertEqual(first.read_bytes(), second.read_bytes())
            self.assertEqual(a['manifest_sha256'], b['manifest_sha256'])

        def test_failure_removes_partial_package(self):
            with patch.object(zipfile.ZipFile, 'writestr', side_effect=OSError('fixture failure')):
                with self.assertRaises(OSError):
                    write_package(self.root / 'failed.zip', self.payloads)
            self.assertEqual(list(self.root.iterdir()), [])

        def test_verification_failure_removes_staging(self):
            def fail(*args):
                raise PackageError('fixture failure')
            with patch.dict(write_package.__globals__, verify_archive=fail):
                with self.assertRaises(PackageError):
                    write_package(self.root / 'failed.zip', self.payloads)
            self.assertEqual(list(self.root.iterdir()), [])

        def test_existing_release_preserved(self):
            output = self.root / 'existing.zip'
            output.write_bytes(b'previous')
            with self.assertRaises(PackageError):
                write_package(output, self.payloads)
            self.assertEqual(output.read_bytes(), b'previous')

        def test_publication_race_preserves_other_writer(self):
            output = self.root / 'race.zip'
            original_verify = verify_archive
            def competing_writer(*args):
                original_verify(*args)
                output.write_bytes(b'other writer')
            with patch.dict(write_package.__globals__, verify_archive=competing_writer):
                with self.assertRaises(FileExistsError):
                    write_package(output, self.payloads)
            self.assertEqual(output.read_bytes(), b'other writer')
            self.assertEqual(list(self.root.iterdir()), [output])

        def test_credentials_never_echoed(self):
            for data in (b'sk-' + b'x' * 32, b'api_key = "fixture-secret-value"',
                         b'-----BEGIN PRIVATE KEY-----', b'https://user:password@example.test'):
                with self.assertRaises(PackageError) as error:
                    scan_payload('safe.toml', data)
                self.assertNotIn(data.decode(), str(error.exception))

        def test_safe_launcher_and_config(self):
            files = generated_files()
            for name, data in files.items():
                scan_payload(name, data)
            launcher = files['start.cmd'].decode('ascii')
            self.assertIn('setlocal EnableExtensions DisableDelayedExpansion', launcher)
            self.assertIn('set "JX3_BIND=127.0.0.1"', launcher)
            self.assertIn('pushd "%~dp0backend"', launcher)
            self.assertNotIn('%*', launcher)
            profiles = tomllib.loads(files['config/agent.providers.example.toml'].decode())['profiles']
            self.assertEqual(len(profiles), 3)
            self.assertTrue(all(p['enabled'] for p in profiles))
            self.assertTrue(all('api_key' not in p for p in profiles))

        @unittest.skipUnless(os.name == 'nt', 'Windows PowerShell port validation')
        def test_port_validation_does_not_execute_untrusted_values(self):
            import subprocess
            shell = Path(os.environ['SystemRoot']) / 'System32/WindowsPowerShell/v1.0/powershell.exe'
            command = PORT_LAUNCH.replace("& '.\\jx3-combat-sim.exe'; exit $LASTEXITCODE", 'exit 0')
            for value, expected in [('3318', 0), ('65535', 0), ('0', 2), ('65536', 2),
                                    ('3318; exit 0', 2), ('$(exit 0)', 2)]:
                environment = dict(os.environ, JX3_PORT=value)
                result = subprocess.run([str(shell), '-NoLogo', '-NoProfile', '-NonInteractive',
                                         '-Command', command], env=environment,
                                        capture_output=True, timeout=15)
                self.assertEqual(result.returncode, expected)

        def test_archive_paths_and_input_links_rejected(self):
            for name in ('../private', '/absolute', 'C:/secret', 'a\\b', 'a/../b', 'a//b', 'a/b.'):
                with self.assertRaises(PackageError):
                    archive_name(name)
            regular = self.root / 'source.txt'
            regular.write_bytes(b'fixture')
            linked = self.root / 'linked.txt'
            os.link(regular, linked)
            with self.assertRaises(PackageError):
                safe_path(linked, self.root)
            with self.assertRaises(PackageError):
                safe_path(self.root.parent / 'outside', self.root)

        def test_allowlist_excludes_private_material(self):
            paths = ['backend/target/release/jx3-combat-sim.exe']
            paths += ['frontend/' + name for name in FRONTEND_FILES]
            paths += ['docs/' + name for name in DOC_FILES]
            paths += ['tools/' + name for name in EXACT_FILES]
            paths += ['backend/data/' + level + '/equip.json' for level in ('level130', 'level50')]
            for version in VERSIONS:
                shared = ('recipes.toml',) if version == VERSIONS[0] else ('recipes.toml', 'team_buffs.toml', 'formations.toml')
                paths += [f'backend/data/{version}/{name}' for name in shared]
                for mount in MOUNTS:
                    paths += [f'backend/data/{version}/{mount}/{name}' for name in (
                        'school.toml', 'talents.toml', 'defaults.json', 'skills/13044_盾刀.toml')]
            paths += [f'backend/data/{VERSIONS[2]}/铁骨衣/{name}' for name in ('recipes.toml', 'team_buffs.toml')]
            for name in paths:
                path = self.root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b'MZfixture' if name.endswith('.exe') else b'{}' if name.endswith('.json') else b'')
            forbidden = ['userdata/agent_custom_provider.json', 'config/agent.providers.toml',
                         'backend/data/equip/source.json', 'backend/data/equip.json',
                         'frontend/private-notes.js', 'docs/operations.md',
                         f'backend/data/{VERSIONS[0]}/分山劲/skills/_template.toml',
                         f'backend/data/{VERSIONS[0]}/分山劲/skills/private/13044_secret.toml']
            for name in forbidden:
                path = self.root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b'sk-' + b'x' * 32)
            payloads = collect_payloads(self.root)
            self.assertTrue(all(name not in payloads for name in forbidden))
            self.assertTrue(all(b'sk-' not in data for data in payloads.values()))
            (self.root / 'backend/data/level50/equip.json').unlink()
            with self.assertRaises(PackageError):
                collect_payloads(self.root)

    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(PackagingTests))
    return 0 if result.wasSuccessful() else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=ROOT / f'dist/cangyun-harness-{VERSION}.zip')
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument('--dry-run', action='store_true', help='validate/hash approved inputs; create no files')
    mode.add_argument('--self-test', action='store_true', help='run synthetic safety/reproducibility tests')
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    try:
        output = check_output(args.output)
        payloads = collect_payloads(ROOT)
        if args.dry_run:
            manifest = manifest_bytes(payloads)
            result = {'dry_run': True, 'path': str(output), 'files': len(payloads) + 1,
                      'payload_bytes': sum(map(len, payloads.values())),
                      'manifest_sha256': hashlib.sha256(manifest).hexdigest()}
        else:
            result = write_package(output, payloads)
        print(json.dumps(result, ensure_ascii=False))
        return 0
    except (PackageError, OSError, zipfile.BadZipFile) as error:
        # Arbitrary OS exceptions can contain private paths. No source traceback.
        detail = str(error) if isinstance(error, PackageError) else type(error).__name__
        print(f'Packaging stopped: {detail}', file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print('Packaging cancelled; no incomplete release was published.', file=sys.stderr)
        return 130


if __name__ == '__main__':
    raise SystemExit(main())
