"""Public runtime package policy; validation metadata lives outside release assets."""
import hashlib
import gzip
import io
from pathlib import Path, PurePosixPath
import re
import stat
import tarfile
import zipfile

from validate_native import TARGETS, digest

MAX_BYTES = 128 * 1024 * 1024


def validation_directory(assets):
    return assets.with_name(assets.name + '-validation')


def package_names(version):
    return {target: f'scripta-{version}-{target}' + ('.zip' if target.endswith('windows-msvc') else '.tar.gz')
            for target in TARGETS}


def runtime_file(name, windows=False):
    binaries = ('scripta.exe', 'scr.exe') if windows else ('scripta', 'scr')
    if name in (*binaries, 'LICENSE', 'THIRD_PARTY_NOTICES.txt'):
        return True
    parts = PurePosixPath(name).parts
    if len(parts) < 3 or parts[0] != 'third-party':
        return False
    if any(part.lower() in ('docs', 'examples', 'playground', 'logs', 'validation') for part in parts):
        return False
    if name.lower().endswith(('.json', '.log')):
        return False
    leaf = parts[-1].lower()
    return (parts[1:3] == ('rust', 'licenses')
            or name == 'third-party/rust/COPYRIGHT-library.html'
            or (len(parts) == 3 and (leaf.startswith(('license', 'copying', 'notice', 'unlicense', 'declared-license'))
                or leaf.endswith(('-license', '-license.md', '-license.txt')))))


def _inspect_package(path, version, target):
    """Check bounded regular entries and allow only executables and legal notices."""
    windows = target.endswith('windows-msvc')
    prefix = f'scripta-{version}-{target}/'
    files = {}
    total = 0

    def add(name, size, stream, mode):
        nonlocal total
        relative = name.removeprefix(prefix)
        parts = PurePosixPath(relative)
        if (not name.startswith(prefix) or '\\' in name or parts.is_absolute()
                or '..' in parts.parts or parts.as_posix() != relative
                or not runtime_file(relative, windows) or relative in files):
            raise ValueError('Unexpected or unsafe public package entry: ' + name)
        if size < 0 or size > MAX_BYTES or total + size > MAX_BYTES or len(files) >= 4096:
            raise ValueError('Public package size or entry limit exceeded')
        data = stream.read(size + 1)
        if len(data) != size:
            raise ValueError('Public package entry size differs')
        total += size
        if not windows and relative in ('scripta', 'scr') and not mode & 0o111:
            raise ValueError('Packaged Unix executable lacks execution permission')
        files[relative] = data

    if windows:
        with zipfile.ZipFile(path) as archive:
            for entry in archive.infolist():
                mode = entry.external_attr >> 16
                if entry.is_dir() or stat.S_IFMT(mode) not in (0, stat.S_IFREG):
                    raise ValueError('Public package entries must be regular files')
                with archive.open(entry) as stream:
                    add(entry.filename, entry.file_size, stream, mode)
    else:
        with gzip.open(path, 'rb') as stream:
            raw = stream.read(MAX_BYTES + 4 * 1024 * 1024 + 1)
        if len(raw) > MAX_BYTES + 4 * 1024 * 1024:
            raise ValueError('Unpacked runtime archive exceeds its size limit')
        with tarfile.open(fileobj=io.BytesIO(raw), mode='r:') as archive:
            for entry in archive:
                if not entry.isfile():
                    raise ValueError('Public package entries must be regular files')
                with archive.extractfile(entry) as stream:
                    add(entry.name, entry.size, stream, entry.mode)
    primary, alias = ('scripta.exe', 'scr.exe') if windows else ('scripta', 'scr')
    if not {primary, alias, 'LICENSE', 'THIRD_PARTY_NOTICES.txt'}.issubset(files):
        raise ValueError('Public package is missing executables or license notices')
    if not files[primary] or files[primary] != files[alias]:
        raise ValueError('Public compatibility executable differs')
    return {'target': target, 'version': version, 'binary': primary, 'compatibility_binary': alias,
            'binary_sha256': hashlib.sha256(files[primary]).hexdigest(),
            'files': {name: hashlib.sha256(data).hexdigest() for name, data in files.items()}}


def inspect_package(path, version, target):
    try:
        if path.stat().st_size > MAX_BYTES * 2:
            raise ValueError('Runtime archive exceeds its size limit')
        return _inspect_package(path, version, target)
    except (zipfile.BadZipFile, tarfile.TarError, OSError) as error:
        raise ValueError('Invalid runtime package archive') from error


def public_assets(assets, version, *, inspect=True):
    if not assets.is_dir() or assets.is_symlink():
        raise ValueError('Public assets must be a regular directory')
    names = package_names(version)
    expected = set(names.values()) | {'SHA256SUMS'}
    if ({p.name for p in assets.iterdir()} != expected
            or any(not p.is_file() or p.is_symlink() for p in assets.iterdir())):
        raise ValueError('Release assets must contain only three platform packages and SHA256SUMS')
    hashes = {}
    if (assets / 'SHA256SUMS').stat().st_size > 65536:
        raise ValueError('Source checksums exceed their size limit')
    for line in (assets / 'SHA256SUMS').read_bytes().decode('ascii').splitlines():
        match = re.fullmatch(r'([0-9a-f]{64})  ([A-Za-z0-9._-]+)', line)
        if not match or match[2] in hashes:
            raise ValueError('Malformed or duplicate source checksums')
        hashes[match[2]] = match[1]
    if set(hashes) != set(names.values()):
        raise ValueError('Source checksums must cover exactly the three platform packages')
    for target, name in names.items():
        if (assets / name).stat().st_size > MAX_BYTES * 2:
            raise ValueError('Runtime archive exceeds its size limit')
        if digest(assets / name) != hashes[name]:
            raise ValueError('Source asset checksum mismatch: ' + name)
        if inspect:
            inspect_package(assets / name, version, target)
    return hashes
