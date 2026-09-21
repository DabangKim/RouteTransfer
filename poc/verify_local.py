"""Isolated loopback SSH/SFTP experiment, not the product implementation."""
import asyncio
import hashlib
import json
import platform
import secrets
import tempfile
import time
from contextlib import asynccontextmanager
from datetime import datetime, timezone
from pathlib import Path

import asyncssh

CHUNK = 64 * 1024
RESULT = Path(__file__).parent / 'results' / 'local.json'


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def snapshot(root):
    return {str(p.relative_to(root)): digest(p) for p in root.rglob('*') if p.is_file()}


class PasswordServer(asyncssh.SSHServer):
    def __init__(self, node):
        self.node = node

    def begin_auth(self, username):
        return True

    def password_auth_supported(self):
        return True

    def validate_password(self, username, password):
        return username == 'poc' and secrets.compare_digest(password, self.node['password'])

    def connection_requested(self, dest_host, dest_port, orig_host, orig_port):
        # A listener may forward only to the next test listener, never arbitrary hosts.
        allowed = (self.node['forward'] and dest_host == '127.0.0.1'
                   and dest_port == self.node.get('next_port'))
        if allowed:
            self.node['forward_count'] += 1
        return allowed


class PinnedClient(asyncssh.SSHClient):
    def __init__(self, fingerprint):
        self.fingerprint = fingerprint

    def validate_host_public_key(self, host, addr, port, key):
        return key.get_fingerprint() == self.fingerprint


@asynccontextmanager
async def lab(base, count, *, block_forward=False, sftp=True):
    nodes = []
    servers = []
    try:
        for index in range(count):
            root = base / f'server-{index + 1}'
            root.mkdir(parents=True)
            key = asyncssh.generate_private_key('ssh-ed25519')
            node = dict(root=root, fingerprint=key.get_fingerprint(),
                        password=secrets.token_urlsafe(24), forward=True, forward_count=0)
            nodes.append(node)
            kwargs = {}
            if index == count - 1 and sftp:
                kwargs['sftp_factory'] = lambda chan, root=root: asyncssh.SFTPServer(
                    chan, chroot=str(root).encode())
            server = await asyncssh.listen(
                '127.0.0.1', 0, server_factory=lambda node=node: PasswordServer(node),
                server_host_keys=[key], password_auth=True, public_key_auth=False,
                kbdint_auth=False, **kwargs)
            node['port'] = server.get_port()
            servers.append(server)
        for index in range(count - 1):
            nodes[index]['next_port'] = nodes[index + 1]['port']
        if block_forward:
            nodes[0]['forward'] = False
        yield nodes
    finally:
        for server in servers:
            server.close()
        await asyncio.gather(*(server.wait_closed() for server in servers))


@asynccontextmanager
async def route(nodes, *, bad_password=None, bad_key=None):
    conns = []
    try:
        for index, node in enumerate(nodes):
            fingerprint = 'SHA256:wrong' if index == bad_key else node['fingerprint']
            conn = await asyncssh.connect(
                node.get('host', '127.0.0.1'), node['port'], username='poc',
                password='wrong' if index == bad_password else node['password'],
                client_keys=[], agent_path=None, config=None,
                known_hosts=([], [], []),
                client_factory=lambda fp=fingerprint: PinnedClient(fp),
                preferred_auth=['password'], tunnel=conns[-1] if conns else None,
                login_timeout=10)
            conns.append(conn)
        yield conns
    finally:
        for conn in reversed(conns):
            conn.close()
        await asyncio.gather(*(conn.wait_closed() for conn in conns), return_exceptions=True)


async def upload(sftp, source, dest, *, interrupt=None):
    temp = dest + '.rt-poc-part'
    await sftp.makedirs(str(Path(dest).parent), exist_ok=True)
    transferred = 0
    async with sftp.open(temp, 'wb') as output:
        with source.open('rb') as stream:
            while chunk := stream.read(CHUNK):
                await output.write(chunk)
                transferred += len(chunk)
                if interrupt and transferred >= CHUNK:
                    interrupt()
                    await asyncio.sleep(0.02)
                    raise ConnectionError('Injected route interruption')
    assert (await sftp.stat(temp)).size == source.stat().st_size
    await sftp.posix_rename(temp, dest)
    return transferred


async def roundtrip(base, count):
    source = base / 'photos'
    (source / '2026' / '한글 폴더').mkdir(parents=True)
    (source / 'empty').mkdir()
    for index in range(24):
        # Synthetic binary fixtures; not actual JPEG photos or a throughput benchmark.
        (source / '2026' / '한글 폴더' / f'photo {index:03}.bin').write_bytes(
            bytes([index]) * (CHUNK + index * 31))
    async with lab(base / 'lab', count) as nodes:
        async with route(nodes) as conns:
            async with conns[-1].start_sftp_client() as sftp:
                for folder in sorted(p for p in source.rglob('*') if p.is_dir()):
                    await sftp.makedirs('/photos/' + folder.relative_to(source).as_posix(), exist_ok=True)
                for file in sorted(p for p in source.rglob('*') if p.is_file()):
                    await upload(sftp, file, '/photos/' + file.relative_to(source).as_posix())
                output = base / 'download'
                output.mkdir()
                await sftp.get('/photos', str(output), recurse=True)
        target = output / 'photos'
        assert snapshot(source) == snapshot(target)
        assert (target / 'empty').is_dir()
        assert all(not list(n['root'].iterdir()) for n in nodes[:-1])
        assert all(n['forward_count'] >= 1 for n in nodes[:-1])
        return dict(servers=count, files=24,
                    bytes=sum(p.stat().st_size for p in source.rglob('*') if p.is_file()),
                    sha256_match=True, empty_folder_preserved=True,
                    intermediate_files=0,
                    forwarding_channels=[n['forward_count'] for n in nodes[:-1]])


async def retry_after_disconnect(base):
    source = base / 'source'
    source.mkdir()
    first, second = source / 'complete.bin', source / 'pending.bin'
    first.write_bytes(b'A' * CHUNK)
    second.write_bytes(b'B' * CHUNK * 8)
    async with lab(base / 'lab', 4) as nodes:
        root = nodes[-1]['root']
        (root / 'pending.bin').write_bytes(b'original target must survive')
        completed = set()
        attempts = {'complete.bin': 0, 'pending.bin': 0}
        async with route(nodes) as conns:
            async with conns[-1].start_sftp_client() as sftp:
                attempts[first.name] += 1
                await upload(sftp, first, '/' + first.name)
                completed.add(first.name)
                original_stat = (root / first.name).stat()
                attempts[second.name] += 1
                try:
                    await upload(sftp, second, '/' + second.name, interrupt=conns[0].abort)
                except (ConnectionError, asyncssh.Error, OSError):
                    pass
                else:
                    raise AssertionError('Expected connection interruption')
        assert (root / 'pending.bin').read_bytes() == b'original target must survive'
        partial_size = (root / 'pending.bin.rt-poc-part').stat().st_size
        assert 0 < partial_size < second.stat().st_size
        async with route(nodes) as conns:
            async with conns[-1].start_sftp_client() as sftp:
                for file in (first, second):
                    if file.name in completed:
                        continue
                    attempts[file.name] += 1
                    sent = await upload(sftp, file, '/' + file.name)
                    assert sent == file.stat().st_size  # Starts from zero, not resume.
                    completed.add(file.name)
        assert (root / first.name).stat().st_mtime_ns == original_stat.st_mtime_ns
        assert snapshot(source) == snapshot(root)
        assert attempts == {'complete.bin': 1, 'pending.bin': 2}
        return dict(attempts=attempts, interrupted_partial_bytes=partial_size,
                    original_preserved_during_failure=True, retry_from_zero=True,
                    sha256_match=True)


async def expected_failure(base, kind):
    async with lab(base, 2, block_forward=kind == 'forward_denied',
                   sftp=kind != 'sftp_unavailable') as nodes:
        expected = {
            'wrong_password': asyncssh.PermissionDenied,
            'changed_host_key': asyncssh.HostKeyNotVerifiable,
            'forward_denied': asyncssh.ChannelOpenError,
            'sftp_unavailable': asyncssh.ChannelOpenError,
        }[kind]
        try:
            async with route(nodes, bad_password=1 if kind == 'wrong_password' else None,
                             bad_key=1 if kind == 'changed_host_key' else None) as conns:
                async with conns[-1].start_sftp_client():
                    pass
        except expected as exc:
            return dict(rejected=True, exception=type(exc).__name__)
        raise AssertionError(f'{kind}: expected {expected.__name__}')


async def main():
    report = dict(timestamp=datetime.now(timezone.utc).isoformat(),
                  platform=platform.platform(), python=platform.python_version(),
                  asyncssh=asyncssh.__version__,
                  scope='Loopback AsyncSSH client/server; synthetic fixtures; not Ubuntu/OpenSSH or production stack',
                  tests=[])
    with tempfile.TemporaryDirectory(prefix='routetransfer-poc-') as tmp:
        base = Path(tmp)
        cases = [
            ('route_2_roundtrip', lambda p: roundtrip(p, 2)),
            ('route_4_roundtrip', lambda p: roundtrip(p, 4)),
            ('file_retry_disconnect', retry_after_disconnect),
        ] + [(kind, lambda p, kind=kind: expected_failure(p, kind)) for kind in
             ('wrong_password', 'changed_host_key', 'forward_denied', 'sftp_unavailable')]
        for name, test in cases:
            folder = base / name
            folder.mkdir()
            start = time.monotonic()
            try:
                details = await asyncio.wait_for(test(folder), timeout=60)
                result = dict(name=name, status='pass', details=details)
            except Exception as exc:
                result = dict(name=name, status='fail', error=f'{type(exc).__name__}: {exc}')
            result['seconds'] = round(time.monotonic() - start, 3)
            report['tests'].append(result)
            print(json.dumps(result, ensure_ascii=False), flush=True)
    report['passed'] = all(t['status'] == 'pass' for t in report['tests'])
    RESULT.parent.mkdir(exist_ok=True)
    RESULT.write_text(json.dumps(report, indent=2, ensure_ascii=False) + '\n')
    if not report['passed']:
        raise SystemExit(1)


if __name__ == '__main__':
    asyncio.run(main())
