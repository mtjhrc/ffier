#!/usr/bin/env python3
"""Native test orchestration for Linux and Windows (including MSVC C tests)."""
from pathlib import Path
import argparse
import ctypes
import json
import os
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
TESTS = ROOT / 'tests'
TARGET = ROOT / 'target'
WINDOWS = sys.platform == 'win32'
WORKSPACE_CLIENT_FEATURES = ','.join([
    'ffier-test-lib-via-cdylib/ffier-dynlookup',
    'ffier-test-lib-via-cdylib/ffier-strong',
    'ffier-test-foreign-lib-via-cdylib/ffier-dynlookup',
    'ffier-test-compat/ffier-strong',
    'ffier-test-windows-handles-client/ffier-strong',
])
ENV = os.environ.copy()
ENV['PATH'] = str(TARGET / 'debug') + os.pathsep + ENV.get('PATH', '')


def run(*args, **kwargs):
    command = [str(arg) for arg in args]
    command[0] = shutil.which(command[0], path=ENV.get("PATH")) or command[0]
    print('+', subprocess.list2cmdline(command), flush=True)
    return subprocess.run(command, cwd=ROOT, env=ENV, check=True, **kwargs)


def cargo(action, *args, **kwargs):
    run('cargo', action, '--manifest-path', ROOT / 'Cargo.toml', *args, **kwargs)


def python(script, *args):
    run(sys.executable, TESTS / script, *args)


def dll(name):
    if WINDOWS:
        return name + '.dll'
    if sys.platform == 'darwin':
        return 'lib' + name + '.dylib'
    return 'lib' + name + '.so'


def msvc_environment():
    if not WINDOWS or shutil.which('cl', path=ENV['PATH']):
        return
    vswhere = Path(ENV.get('PROGRAMFILES(X86)', r'C:\Program Files (x86)')) / 'Microsoft Visual Studio/Installer/vswhere.exe'
    if not vswhere.is_file():
        raise RuntimeError('Install MSVC Build Tools, or run inside an x64 Native Tools command prompt.')
    installation = subprocess.check_output([str(vswhere), '-latest', '-products', '*', '-requires', 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64', '-property', 'installationPath'], text=True).strip()
    vcvars = Path(installation) / 'VC/Auxiliary/Build/vcvars64.bat'
    output = subprocess.check_output(f'cmd.exe /d /s /c ""{vcvars}" >nul && set"', env=ENV, text=True)
    for line in output.splitlines():
        key, sep, value = line.partition('=')
        if sep and key:
            ENV[key.upper()] = value


def build_foreign():
    cargo('rustc', '-p', 'ffier-test-foreign-lib', '--crate-type', 'cdylib')


def build():
    build_foreign()
    cargo('rustc', '-p', 'ffier-test-lib', '--crate-type', 'cdylib')


def header(schema='ffier-ft.json', output='ffier_test.h', guard='FFIER_FT_H', *options):
    with (TARGET / output).open('w', encoding='utf-8', newline='\n') as destination:
        cargo('run', '-p', 'ffier-gen-c-header', '--', TARGET / schema, guard, *options, stdout=destination)


def syntax(source):
    msvc_environment()
    if WINDOWS:
        run('cl', '/nologo', '/std:c11', '/W4', '/WX', '/TC', '/Zs', '/I' + str(TARGET), source)
    else:
        run('cc', '-std=c11', '-Wall', '-Wextra', '-Werror', '-I', TARGET, '-x', 'c', '-fsyntax-only', source)


def check_header():
    header()
    syntax(TARGET / 'ffier_test.h')


def c():
    check_header()
    msvc_environment()
    exe = TARGET / ('c_test_main.exe' if WINDOWS else 'c_test_main')
    if WINDOWS:
        run('cl', '/nologo', '/std:c11', '/W4', '/WX', '/Z7', '/Fe:' + str(exe), '/Fo:' + str(TARGET) + os.sep,
            '/I' + str(TARGET), TESTS / 'c/test_main.c', '/link', '/LIBPATH:' + str(TARGET / 'debug'),
            'ffier_test_lib.dll.lib', 'ffier_test_foreign_lib.dll.lib')
    else:
        run('cc', '-Wall', '-Wextra', '-Werror', '-g', '-o', exe, TESTS / 'c/test_main.c', '-I', TARGET,
            '-L', TARGET / 'debug', '-lffier_test_lib', '-lffier_test_foreign_lib',
            '-Wl,-rpath,' + str(TARGET / 'debug'), '-Wl,--disable-new-dtags')
    run(exe)
    if WINDOWS:
        # Rust's abort path uses fail-fast on Windows. Run each invalid access
        # in a child so the full native suite can finish and check the status.
        cases = [('mixed', 1, 8), ('mixed', 3, 8), ('mixed', ctypes.c_size_t(-1).value, 8),
                 ('mixed', 0, 0), ('unit', 0, 8), ('empty', 0, 8)]
        for kind, index, size in cases:
            result = subprocess.run([str(exe), '--payload-abort-case', kind, str(index), str(size)], cwd=ROOT,
                                    env=ENV, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            assert result.returncode & 0xffffffff == 0xc0000409, (kind, index, size, result.returncode)
        print('All 6 invalid C payload accesses aborted as expected.', flush=True)
    else:
        run(exe, '--payload-abort-tests')


def valgrind():
    run('valgrind', '--leak-check=full', '--show-leak-kinds=all', '--track-origins=yes', '--error-exitcode=1', TARGET / 'c_test_main')


def miri(model=None):
    for name in ([model] if model else ['stacked', 'tree']):
        previous = ENV.get('MIRIFLAGS')
        ENV['MIRIFLAGS'] = (previous or '') + (' -Zmiri-tree-borrows' if name == 'tree' else '')
        run('cargo', '+nightly', 'miri', 'test', '--manifest-path', ROOT / 'Cargo.toml', '-p', 'ffier-test-lib')
        if previous is None:
            ENV.pop('MIRIFLAGS', None)
        else:
            ENV['MIRIFLAGS'] = previous


def no_codegen():
    cargo('check', '-p', 'ffier-test-lib', '--no-default-features')
    cargo('test', '-p', 'ffier-test-lib', '--no-default-features', 'fieldless_error_client_tests')


def check_cfg():
    cargo('build', '-p', 'ffier-test-lib', '--no-default-features', '--features', 'ffi,fruit-label')
    python('check_schema.py', TARGET / 'ffier-ft.json')


def conditional(enabled):
    args = ['--manifest-path', TESTS / 'conditional-bridge/Cargo.toml', '--target-dir', TARGET / 'conditional-bridge', '-p', 'conditional-bridge']
    if enabled:
        run('cargo', 'test', *args, '--features', 'a_feature')
    else:
        run('cargo', 'build', *args)
        library = TARGET / 'conditional-bridge/debug' / dll('conditional_bridge')
        # Load in a short-lived child: Windows keeps a DLL locked while loaded.
        code = 'import ctypes,sys; lib=ctypes.CDLL(sys.argv[1]); assert all(not hasattr(lib,"conditional_widget_"+n) for n in ("new","get","destroy"))'
        run(sys.executable, '-c', code, library)


def platform_cfg():
    cargo('rustc', '-p', 'ffier-test-windows-handles', '--crate-type', 'cdylib')
    for args in [['--features', 'ffier-strong'], ['--features', 'ffier-dynlookup'], ['--features', 'ffier-dynlookup,ffier-strong']]:
        cargo('test', '-p', 'ffier-test-windows-handles-client', *args)
    schema_path = Path(ENV.get('FFIER_WINDOWS_HANDLES_SCHEMA', TARGET / 'ffier-windows-handles.json'))
    schema = json.loads(schema_path.read_text())
    logs = [f for f in schema['free_functions'] if f['name'] == 'init_log']
    assert len(logs) == 2
    assert {f['params'][0]['type'] for f in logs} == {'BorrowedFd', 'BorrowedHandle'}
    handle_api = next(t for t in schema['exported_types'] if t['name'] == 'HandleApi')
    assert len([m for m in handle_api['methods'] if m['name'] == 'raw']) == 2
    source = next(t for t in schema['traits'] if t['name'] == 'ReadySource')
    assert len(source['methods']) == source['num_slots'] == 4
    header(str(schema_path), 'platform.h', 'FFIER_PLATFORM_H', '--fn-typedefs')
    syntax(TARGET / 'platform.h')


def consumer(mode=None):
    for name in ([mode] if mode else ['native', 'via-cdylib']):
        cargo('test', '-p', 'ffier-test-consumer', '--no-default-features', '--features', name)


def rust_client():
    for args in [['--features', 'ffier-dynlookup,fruit-label'], ['--features', 'ffier-strong,fruit-label'], ['--features', 'ffier-dynlookup,ffier-strong,fruit-label']]:
        if not WINDOWS and 'ffier-strong' not in args[-1]:
            # Keep the provider in the global lookup namespace even when the
            # platform linker discards unreferenced libraries with --as-needed.
            runner = ['env',
                      'LD_PRELOAD=' + str(TARGET / 'debug/libffier_test_lib.so'),
                      'DYLD_INSERT_LIBRARIES=' + str(TARGET / 'debug/libffier_test_lib.dylib')]
            args += ['--config', 'target."cfg(unix)".runner = ' + json.dumps(runner)]
        cargo('test', '-p', 'ffier-test-lib-via-cdylib', *args)
    msvc_environment()


def clippy():
    build()
    cargo('clippy', '--workspace', '--features', WORKSPACE_CLIENT_FEATURES, '--', '-D', 'warnings')
    for features in [['--features', 'conditional-bridge/a_feature'], []]:
        run('cargo', 'clippy', '--manifest-path', TESTS / 'conditional-bridge/Cargo.toml', '--target-dir', TARGET / 'conditional-bridge', '--workspace', '--all-targets', *features, '--', '-D', 'warnings')
    msvc_environment()


def all_checks():
    build_foreign()
    no_codegen()
    check_cfg()
    build()
    check_header()
    platform_cfg()
    python('compile-fail/check.py')
    python('grouped-functions/check.py')
    conditional(True)
    conditional(False)
    c()
    if sys.platform == 'linux':
        valgrind()
        miri()
    else:
        print('Valgrind and Miri memory-model checks run in the Linux job.', flush=True)
    consumer()
    rust_client()
    cargo('test', '-p', 'ffier-test-compat', '--features', 'ffier-strong')
    cargo('test', '--workspace', '--features', WORKSPACE_CLIENT_FEATURES)
    print('\nAll checks passed!', flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', nargs='?', default='all')
    action = parser.parse_args().action
    actions = {
        'all': all_checks, 'clippy': clippy, 'build': build, 'build-foreign': build_foreign,
        'gen-header': lambda: (build(), header()), 'check-header': lambda: (build(), check_header()),
        'c': lambda: (build(), c()), 'valgrind': lambda: (build(), c(), valgrind()),
        'miri': miri, 'miri-stacked': lambda: miri('stacked'), 'miri-tree': lambda: miri('tree'),
        'consumer': lambda: (build(), consumer()), 'consumer-native': lambda: (build(), consumer('native')),
        'consumer-cdylib': lambda: (build(), consumer('via-cdylib')), 'rust-client': lambda: (build(), rust_client()),
        'compat': lambda: (build(), cargo('test', '-p', 'ffier-test-compat', '--features', 'ffier-strong')),
        'no-codegen': lambda: (build_foreign(), no_codegen()), 'check-cfg-library-entry': lambda: (build_foreign(), check_cfg()),
        'cargo-test': lambda: (build(), cargo('test', '--workspace', '--features', WORKSPACE_CLIENT_FEATURES)), 'compile-fail': lambda: python('compile-fail/check.py'),
        'conditional-bridge': lambda: (conditional(True), conditional(False)),
        'conditional-bridge-enabled': lambda: conditional(True), 'conditional-bridge-disabled': lambda: conditional(False),
        'platform-cfg': platform_cfg, 'grouped-functions': lambda: python('grouped-functions/check.py'),
    }
    if action not in actions:
        parser.error('unknown action: ' + action)
    actions[action]()


if __name__ == '__main__':
    try:
        main()
    except subprocess.CalledProcessError as error:
        print(f'Command failed with exit status {error.returncode}.', file=sys.stderr)
        sys.exit(1)
