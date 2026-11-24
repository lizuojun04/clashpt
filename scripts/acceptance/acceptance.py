"""Black-box acceptance. Run with run.sh [CLI executable] [Mihomo executable]."""
import ctypes
import gzip
import hashlib
import http.server
import io
import json
import os
import platform
import secrets
import shutil
import signal
import socket
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

NAMES = [
    '用户控制 API 正常连接', '用户控制 API 的错误服务端拒绝',
    'HTTP 默认认证', 'SOCKS 默认认证', 'Bash 代理', 'Zsh 代理',
    '卸载保留无关文件', '安装保留无关程序', '并发安装卸载',
    '系统控制 API 的服务端身份', '系统部署仅信任管理员程序',
    '系统内核升级', '系统部署失败回退',
]


class Skip(Exception):
    pass


def run(*cmd, env=None):
    return subprocess.run(cmd, env=env, stdin=subprocess.DEVNULL, capture_output=True, timeout=50)


def require(condition):
    if not condition:
        raise AssertionError()


def check(name, function):
    try:
        function()
        result[name] = '通过'
    except Skip:
        result[name] = '跳过'
    except Exception:
        result[name] = '失败'


def report():
    counts = [sum(value == status for value in result.values()) for status in ('通过', '失败', '跳过')]
    print('通过 %d，失败 %d，跳过 %d' % tuple(counts))
    for name in NAMES:
        if result[name] == '失败':
            print('失败需求 ' + name)
    return int(counts[1] > 0)


def compatible_syscalls():
    # Older packaged PRoot versions predate these Linux filesystem interfaces.
    # Let libc and the CLI use the kernel's established ENOSYS fallback paths.
    sec = ctypes.CDLL('libseccomp.so.2')
    class Instruction(ctypes.Structure):
        _fields_ = [('op', ctypes.c_ushort), ('jt', ctypes.c_ubyte), ('jf', ctypes.c_ubyte),
                    ('argument', ctypes.c_uint32)]
    class Program(ctypes.Structure):
        _fields_ = [('length', ctypes.c_ushort), ('filter', ctypes.POINTER(Instruction))]
    rules = [Instruction(0x20, 0, 0, 0)]
    for name in (b'statx', b'faccessat2', b'openat2'):
        nr = sec.seccomp_syscall_resolve_name(name)
        if nr >= 0:
            rules.extend([Instruction(0x15, 0, 1, nr), Instruction(0x06, 0, 0, 0x00050026)])
    rules.append(Instruction(0x06, 0, 0, 0x7fff0000))
    code = (Instruction * len(rules))(*rules)
    policy = Program(len(rules), code)
    libc = ctypes.CDLL(None)
    if libc.prctl(38, 1, 0, 0, 0) or libc.prctl(22, 2, ctypes.byref(policy), 0, 0):
        raise Skip()



def contain(base):
    # Landlock grants applications only the disposable tree and read-only OS resources.
    libc = ctypes.CDLL(None, use_errno=True)
    abi = libc.syscall(444, 0, 0, 1)
    if abi < 1 or platform.machine() not in ('x86_64', 'aarch64'):
        raise Skip()
    ro = 1 | 4 | 8
    allowed = (1 << 13) - 1
    if abi >= 2:
        allowed |= 1 << 13
    if abi >= 3:
        allowed |= 1 << 14
    policy = libc.syscall(444, ctypes.byref(ctypes.c_uint64(allowed)), 8, 0)
    if policy < 0:
        raise Skip()
    class Rule(ctypes.Structure):
        _pack_ = 1
        _fields_ = [('access', ctypes.c_uint64), ('fd', ctypes.c_int32)]
    for path, access in [(base, allowed), ('/tmp', allowed), ('/dev/shm', allowed), ('/proot-loaders', 5), ('/usr', ro), ('/lib', ro), ('/lib64', ro),
                         ('/bin', ro), ('/sbin', ro), ('/etc', ro), ('/proc', ro),
                         ('/sys', ro), ('/dev/null', 6), ('/dev/urandom', 4),
                         ('/dev/random', 4)]:
        if os.path.exists(path):
            fd = os.open(path, os.O_PATH)
            rc = libc.syscall(445, policy, 1, ctypes.byref(Rule(access, fd)), 0)
            os.close(fd)
            if rc:
                raise Skip()
    if libc.prctl(38, 1, 0, 0, 0) or libc.syscall(446, policy, 0):
        raise Skip()
    os.close(policy)


def environment(base):
    base.mkdir()
    tmp = base / 'tmp'
    tmp.mkdir()
    return {'HOME': str(base), 'XDG_CONFIG_HOME': str(base / 'cfg'),
            'TMPDIR': str(tmp), 'PATH': str(root / 'bin') + ':/usr/bin:/bin',
            'LANG': 'C', 'NO_COLOR': '1', 'CLASHPT_MIHOMO': str(root / 'bin/mihomo')}


def proxy_cases():
    if not (root / 'bin/mihomo').exists():
        return
    env = environment(root / 'u')
    cli = lambda *args: run('clashpt', *args, env=env)
    profile = root / 'input.yaml'
    profile.write_text('mixed-port: 18571\nmode: rule\nproxy-groups:\n'
                       '  - {name: Probe, type: select, proxies: [DIRECT]}\nrules: [MATCH,DIRECT]\n'
                       .replace('[MATCH,DIRECT]', '["MATCH,DIRECT"]'))
    try:
        started = cli('profile', 'import', profile).returncode == 0 and cli('start').returncode == 0
        def api():
            require(started)
            answer = cli('nodes', 'list', '--json')
            require(answer.returncode == 0 and 'Probe' in str(json.loads(answer.stdout)))
        check(NAMES[0], api)
        if not started:
            return
        def identity():
            tracer = shutil.which('strace', path='/usr/bin:/bin')
            if not tracer:
                raise Skip()
            args = [tracer, '-f', '-qq', '-e', 'trace=getsockopt', 'clashpt', 'nodes', 'list', '--json']
            normal = run(*args, env=env)
            if normal.returncode or b'SO_PEERCRED' not in normal.stderr:
                raise Skip()
            # Simulate the OS being unable to attest to the genuine service's identity.
            # The only running controller is the real service for this profile.
            denied = run(tracer, '-f', '-qq', '-e', 'trace=getsockopt', '-e',
                         'inject=getsockopt:error=EACCES', 'clashpt', 'nodes', 'list', '--json', env=env)
            if b'SO_PEERCRED' not in denied.stderr or b'INJECTED' not in denied.stderr:
                raise Skip()
            require(denied.returncode != 0)
        check(NAMES[1], identity)
        class Origin(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                self.send_response(200)
                self.send_header('Content-Length', str(len(b'acceptance origin')))
                self.send_header('Connection', 'close')
                self.end_headers()
                self.wfile.write(b'acceptance origin')
            def log_message(self, *args):
                pass
        server = http.server.HTTPServer(('127.0.0.1', 0), Origin)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        dest = 'http://127.0.0.1:%d/probe' % server.server_port
        try:
            def http_auth():
                probe = run('/usr/bin/curl', '-qsS', '--noproxy', '', '--max-time', '4',
                            '--proxy', 'http://127.0.0.1:18571', '-o', '/dev/null',
                            '-w', '%{http_code}', dest, env=env)
                require(probe.stdout == b'407')
            def socks():
                with socket.create_connection(('127.0.0.1', 18571), timeout=4) as conn:
                    conn.sendall(bytes((5, 1, 0)))
                    with conn.makefile('rb') as stream:
                        require(stream.read(2) in (bytes((5, 2)), bytes((5, 255))))
            check(NAMES[2], http_auth)
            check(NAMES[3], socks)
            for shell, name in [('bash', NAMES[4]), ('zsh', NAMES[5])]:
                def integration():
                    exe = shutil.which(shell, path='/usr/bin:/bin')
                    if not exe:
                        raise Skip()
                    script = '''
                    source "$1" && clashpt proxy on >/dev/null || exit 1
                    for address in "$http_proxy" "$https_proxy" "$all_proxy"; do
                        [ -n "$address" ] || exit 1
                        answer=$(/usr/bin/curl -qfsS --max-time 4 --noproxy '' --proxy "$address" "$2") || exit 1
                        [ "$answer" = 'acceptance origin' ] || exit 1
                    done
                    clashpt proxy off >/dev/null && [ -z "${http_proxy-}${https_proxy-}${all_proxy-}" ]
                    '''
                    require(run(exe, '-c', script, shell, root / ('scripts/clashpt.' + shell),
                                dest, env=env).returncode == 0)
                check(name, integration)
        finally:
            server.shutdown()
            server.server_close()
    finally:
        cli('stop')


def install_cases():
    def sentinel(path):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(b'independent content\n')
    def preserved(paths):
        return all(p.is_file() and p.read_bytes() == b'independent content\n' for p in paths)
    def uninstall():
        base = root / 'uninstalled'
        env = environment(base)
        prefix = base / 'prefix'
        paths = [prefix / 'share/clashpt/shell/notes', base / 'cfg/clashpt/keep',
                 base / '.local/state/clashpt/keep', prefix / 'bin/mihomo']
        for p in paths:
            sentinel(p)
        run('sh', root / 'uninstall.sh', '-y', '--prefix', prefix, env=env)
        require(preserved(paths))
    check(NAMES[6], uninstall)
    if not (root / 'bin/mihomo').exists():
        return
    # Offline distributor with the documented release format and real local binaries.
    dist = root / 'dist'
    dist.mkdir()
    arch = platform.machine()
    asset = 'mihomo-linux-acceptance.gz'
    core = gzip.compress((root / 'bin/mihomo').read_bytes(), compresslevel=1)
    (dist / asset).write_bytes(core)
    manifest = ('%s v1.0 %s %s\n' % (arch, asset, hashlib.sha256(core).hexdigest())).encode()
    archive = dist / ('clashpt-linux-%s.tar.gz' % arch)
    with tarfile.open(archive, 'w:gz', compresslevel=1) as bundle:
        for member in ('clashpt', 'clashpt.bash', 'clashpt.zsh'):
            path = root / ('bin' if member == 'clashpt' else 'scripts') / member
            bundle.add(path, arcname=member, recursive=False)
        info = tarfile.TarInfo('mihomo-release.tsv')
        info.size = len(manifest)
        bundle.addfile(info, io.BytesIO(manifest))
    (dist / 'SHA256SUMS').write_text('%s  %s\n' % (hashlib.sha256(archive.read_bytes()).hexdigest(), archive.name))
    fetch = root / 'bin/curl'
    fetch.write_text('''#!/bin/sh
set -eu
while [ "$#" -gt 0 ]; do
    case "$1" in
        --output|-o) shift; out=$1 ;;
        http*://*) asset=${1##*/} ;;
    esac
    shift
done
cp -- "$DIST/$asset" "$out"
''')
    fetch.chmod(0o755)
    def command(base):
        env = environment(base)
        env['DIST'] = str(dist)
        env['CLASHPT_RELEASE_URL'] = 'https://distribution.invalid'
        prefix = base / 'prefix'
        def install():
            return run('sh', root / 'install.sh', '--prefix', prefix, env=env).returncode
        def remove():
            return run('sh', root / 'uninstall.sh', '-y', '--prefix', prefix, env=env).returncode
        return prefix, install, remove
    def foreign():
        prefix, install, _ = command(root / 'foreign')
        p = prefix / 'bin/mihomo'
        sentinel(p)
        install()
        require(preserved([p]))
    # Validate distribution before relying on any early refusal.
    prefix, install, remove = command(root / 'overlap')
    if install() != 0:
        result[NAMES[8]] = '失败'
        return
    check(NAMES[7], foreign)
    def concurrency():
        p = prefix / 'share/clashpt/notes'
        sentinel(p)
        members = [prefix / 'bin/clashpt', prefix / 'bin/mihomo',
                   prefix / 'share/clashpt/shell/clashpt.bash', prefix / 'share/clashpt/shell/clashpt.zsh']
        with ThreadPoolExecutor(max_workers=2) as pool:
            for _ in range(3):
                gate = threading.Barrier(2)
                def worker(operation):
                    gate.wait(timeout=5)
                    return operation()
                outcomes = list(pool.map(worker, (install, remove)))
                require(0 in outcomes and len({p.exists() for p in members}) == 1 and preserved([p]))
        require(install() == 0 and all(p.is_file() for p in members) and remove() == 0)
        require(all(not p.exists() for p in members) and preserved([p]))
    check(NAMES[8], concurrency)



def system_cases(repo, binary, core):
    # Optional bootable x86_64 Linux cloud image with cloud-init, Python and systemd.
    # The guest gets a disposable disk overlay, a read-only ISO, and no NIC.
    disk = os.environ.get('CLASHPT_ACCEPTANCE_QCOW2')
    qemu = shutil.which('qemu-system-x86_64')
    iso_tool = shutil.which('genisoimage') or shutil.which('mkisofs')
    if not disk or not qemu or not iso_tool or not core.is_file() or platform.machine() != 'x86_64':
        return
    with tempfile.TemporaryDirectory(prefix='cpa-vm-') as tmp:
        staging = Path(tmp)
        media = staging / 'seed'
        media.mkdir()
        for name, path in [('clashpt', binary), ('mihomo', core),
                           ('guest.py', Path(__file__).with_name('guest.py'))]:
            shutil.copy2(path, media / name)
        next_core = os.environ.get('CLASHPT_ACCEPTANCE_NEXT_MIHOMO')
        if next_core:
            shutil.copy2(next_core, media / 'mihomo-next')
        token = secrets.token_hex(24)
        (media / 'guest-token').write_text(token)
        (media / 'meta-data').write_text('instance-id: acceptance-' + token)
        (media / 'user-data').write_text('''#cloud-config
runcmd:
  - [mkdir, -p, /mnt/acceptance]
  - [mount, -o, ro, /dev/sr0, /mnt/acceptance]
  - [python3, /mnt/acceptance/guest.py]
  - [poweroff]
''')
        iso = staging / 'seed.iso'
        if run(iso_tool, '-quiet', '-R', '-V', 'cidata', '-o', iso, media).returncode:
            return
        output = staging / 'results'
        # input paths with commas need QEMU's literal comma escaping
        image = str(Path(disk).resolve()).replace(',', ',,')
        args = [qemu, '-m', '2048', '-smp', '2', '-accel', 'kvm:tcg', '-no-reboot',
                '-display', 'none', '-serial', 'none', '-monitor', 'none', '-nic', 'none',
                '-drive', 'file=' + image + ',format=qcow2,if=virtio,snapshot=on',
                '-cdrom', str(iso), '-device', 'virtio-serial',
                '-chardev', 'file,id=results,path=' + str(output),
                '-device', 'virtserialport,chardev=results,name=clashpt.acceptance']
        try:
            subprocess.run(args, timeout=450, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            nonce, statuses = json.loads(output.read_text())
            if nonce == token:
                for index, value in statuses.items():
                    if 9 <= int(index) <= 12 and value in ('通过', '失败', '跳过'):
                        result[NAMES[int(index)]] = value
        except (FileNotFoundError, ValueError, subprocess.TimeoutExpired):
            pass


def launch(repo, binary, core_path):
    if not binary.is_file():
        raise SystemExit('未找到验收用 CLI 二进制文件')
    system_cases(repo, binary, core_path)
    ns = shutil.which('unshare')
    ip = shutil.which('ip')
    if not ns or not ip or os.getuid() == 0:
        return report()
    with tempfile.TemporaryDirectory(prefix='cpa-', dir='/tmp') as temp:
        base = Path(temp)
        for d in ('bin', 'scripts'):
            (base / d).mkdir()
        shutil.copy2(binary, base / 'bin/clashpt')
        if core_path.is_file():
            shutil.copy2(core_path, base / 'bin/mihomo')
        for path in ['install.sh', 'uninstall.sh', 'scripts/clashpt.bash', 'scripts/clashpt.zsh']:
            shutil.copy2(repo / path, base / path)
        shutil.copy2(__file__, base / 'runner.py')
        (base / 'system-results').write_text(json.dumps(result))
        synthetic_uid = 1000000000 + secrets.randbelow(1000000000)
        inner = [ns, '--map-user=' + str(synthetic_uid), '--map-group=' + str(synthetic_uid)]
        wrapper = [ns, '-Urn', 'sh', '-c', '"$1" link set lo up && shift && exec "$@"', 'net', ip, *inner]
        if run(*wrapper, 'true').returncode:
            return report()
        proot = os.environ.get('CLASHPT_ACCEPTANCE_PROOT') or shutil.which('proot')
        if not proot:
            return report()
        virtual = base / 'vroot'
        virtual.mkdir()
        for d in ('tmp', 'dev/shm', 'suite', 'run', 'home', 'var/tmp', 'proot-loaders'):
            (virtual / d).mkdir(parents=True, exist_ok=True)
        (virtual / 'dev/shm/.acceptance').touch()
        bindings = []
        for source in ('/usr', '/lib', '/lib64', '/bin', '/sbin', '/etc', '/proc', '/sys',
                       '/dev/null', '/dev/urandom', '/dev/random'):
            if Path(source).exists():
                bindings.extend(['-b', source])
        procenv = {'PATH': '/usr/bin:/bin:/usr/sbin', 'LANG': 'C', 'PROOT_NO_SECCOMP': '1',
                   'TMPDIR': str(base / 'vroot/tmp')}
        libs = os.environ.get('CLASHPT_ACCEPTANCE_PROOT_LIB')
        if libs:
            procenv['LD_LIBRARY_PATH'] = libs
        command = [*wrapper, proot, '-r', str(virtual), *bindings, '-b', '/tmp:/proot-loaders', '-b', str(base) + ':/suite',
                   '-w', '/suite', '/usr/bin/python3', '-B', '/suite/runner.py', '--inside', '/suite']
        return subprocess.call(command, cwd=base, env=procenv, preexec_fn=compatible_syscalls)



os.umask(0o077)
result = dict.fromkeys(NAMES, '跳过')
if sys.argv[1] != '--inside':
    sys.exit(launch(*(Path(s).resolve() for s in sys.argv[1:])))
root = Path(sys.argv[2])
result.update(json.loads((root / 'system-results').read_text()))
if not (root / 'vroot/dev/shm/.acceptance').samefile('/dev/shm/.acceptance'):
    sys.exit(report())
try:
    contain(root)
except Skip:
    sys.exit(report())
# Adopt detached cores so stopping a core observes its real exit in a container.
libc = ctypes.CDLL(None)
if libc.prctl(36, 1, 0, 0, 0):
    sys.exit(report())
worker_pid = os.fork()
if worker_pid == 0:
    proxy_cases()
    install_cases()
    sys.exit(report())
while True:
    pid, status = os.wait()
    if pid == worker_pid:
        worker_status = os.waitstatus_to_exitcode(status)
        break
# Only reap or terminate descendants of this isolated invocation.
children = Path('/proc/self/task/%d/children' % os.getpid())
for child in children.read_text().split():
    try:
        os.kill(int(child), signal.SIGKILL)
    except ProcessLookupError:
        pass
sys.exit(worker_status)
