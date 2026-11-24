"""Entry point inside the acceptance runner's ephemeral QEMU guest only."""
import hashlib
import json
import os
import shutil
import subprocess
from pathlib import Path

channel = Path('/dev/virtio-ports/clashpt.acceptance')
media = Path('/mnt/acceptance')
if not channel.exists() or not (media / 'guest-token').exists():
    raise SystemExit(2)
token = (media / 'guest-token').read_text()
root = Path('/opt/clashpt-acceptance')
root.mkdir(mode=0o755)
binary = root / 'clashpt'
old = root / 'mihomo'
new = root / 'mihomo-next'
outcomes = {}


def run(*args):
    return subprocess.run(args, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                          stdin=subprocess.DEVNULL, timeout=70)


def apply(core, profile):
    return run(binary, '--mihomo', core, 'system', 'apply', profile, '--uid', str(uid)).returncode


def nodes():
    reply = run('runuser', '-u', 'clashpt-check', '--', binary, '--system', 'nodes', 'list', '--json')
    return reply.returncode == 0 and 'Probe' in str(json.loads(reply.stdout))


def fingerprint(path):
    return hashlib.sha256(path.read_bytes()).digest()


def check(index, case):
    try:
        outcomes[index] = '通过' if case() else '失败'
    except Exception:
        outcomes[index] = '失败'


try:
    for name in ('clashpt', 'mihomo', 'mihomo-next'):
        source = media / name
        if source.exists():
            shutil.copyfile(source, root / name)
            (root / name).chmod(0o755)
    run('modprobe', 'tun')
    if not Path('/dev/net/tun').exists() or not Path('/run/systemd/system').is_dir():
        raise SystemExit()
    if run('useradd', '-m', 'clashpt-check').returncode:
        raise SystemExit()
    uid = int(run('id', '-u', 'clashpt-check').stdout)
    cfg = root / 'profile.yaml'
    cfg.write_text('mixed-port: 18571\nproxy-groups:\n  - {name: Probe, type: select, proxies: [DIRECT]}\nrules: ["MATCH,DIRECT"]\n')
    saved_core = Path('/etc/clashpt/mihomo')
    if apply(old, cfg) or not nodes():
        outcomes[10] = '失败'
        raise SystemExit()
    # Observe a genuine gateway's transport without issuing requests to any substitute service.
    def identity():
        observed = run(binary, '--system', 'status', '--json')
        controller = json.loads(observed.stdout).get('controller')
        if not isinstance(controller, str) or not controller.startswith(('unix:', '/')):
            return None
        path = Path(controller.removeprefix('unix:').lstrip('/')).as_posix()
        sock = Path('/') / path
        if not sock.is_socket():
            return None
        return all(p.stat().st_uid == 0 and not p.stat().st_mode & 0o022 for p in sock.parents)
    secure = identity()
    if secure is not None:
        outcomes[9] = '通过' if secure else '跳过'
    def trust():
        sample = root / 'delegate-core'
        shutil.copy2(old, sample)
        os.chown(sample, uid, -1)
        return apply(sample, cfg) != 0 and nodes() and fingerprint(saved_core) == fingerprint(old)
    check(10, trust)
    if new.exists() and fingerprint(new) != fingerprint(old):
        check(11, lambda: apply(new, cfg) == 0 and nodes() and fingerprint(saved_core) == fingerprint(new))
        if outcomes[11] == '通过':
            def rollback():
                # A trusted test core supports the documented version/config probes and
                # experiences an ordinary startup failure. It has no control listener.
                failing = root / 'unavailable-core'
                failing.write_text('#!/bin/sh\ncase " $* " in *" -v "*|*" -t "*) exec /opt/clashpt-acceptance/mihomo "$@";; esac\nexit 1\n')
                failing.chmod(0o755)
                return apply(failing, cfg) != 0 and nodes() and fingerprint(saved_core) == fingerprint(new)
            check(12, rollback)
finally:
    channel.write_text(json.dumps([token, outcomes]) + '\n')
