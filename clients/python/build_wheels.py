#!/usr/bin/env python3
"""Build the klatch platform wheels (#431) from the release archives, with the standard library only.

A platform wheel installs the `klatch` and `klatch-server` commands (as scripts in the environment's bin directory, the
way ruff and uv ship theirs) and the Python client (`import klatch`). So `pipx install klatch` and `uvx klatch` give a working klatch
on macOS and Linux. The pure client wheel stays for every other platform.

    build_wheels.py --version 0.1.0.dev1 --target aarch64-apple-darwin --archive klatch-0.1.0-dev.1-aarch64-apple-darwin.tar.gz --out-dir dist

The wheel is reproducible: files sorted, timestamps fixed (or SOURCE_DATE_EPOCH), nothing from the build machine.
"""
import argparse
import base64
import hashlib
import io
import os
import struct
import sys
import tarfile
import zipfile

NAME = "klatch"
SERVER_NAME = "klatch-server"
SUMMARY = "A substrate for agent rooms (pre-release; the klatch and klatch-server commands and the Python client)"
HOMEPAGE = "https://github.com/hslee16/klatch"
REQUIRES_PYTHON = ">=3.9"
# 1980-01-02, the earliest date a zip can hold, unless SOURCE_DATE_EPOCH says otherwise
DEFAULT_EPOCH = 315619200

# Which wheel each release target becomes. The Linux wheels are the static musl builds: they need no glibc, so one
# wheel serves both glibc (manylinux 2.17, the oldest tag pip still reads everywhere) and musl (musllinux 1.1) systems.
# The macOS minimums are what the release binaries declare (checked against the binary when the wheel is built).
TARGET_TAGS = {
    "aarch64-apple-darwin": ["macosx_11_0_arm64"],
    "x86_64-apple-darwin": ["macosx_10_12_x86_64"],
    "x86_64-unknown-linux-musl": ["manylinux_2_17_x86_64", "musllinux_1_1_x86_64"],
    "aarch64-unknown-linux-musl": ["manylinux_2_17_aarch64", "musllinux_1_1_aarch64"],
}


def tags_for(target):
    """The platform tags of the wheel built from a release target; an error for a target that gets no wheel."""
    try:
        return list(TARGET_TAGS[target])
    except KeyError:
        raise ValueError(f"no wheel for target {target}: it is one of {sorted(TARGET_TAGS)}") from None


def macho_minos(path):
    """The (major, minor) macOS version a Mach-O binary asks for, from LC_BUILD_VERSION or LC_VERSION_MIN_MACOSX."""
    with open(path, "rb") as f:
        data = f.read(1 << 16)
    if len(data) < 32:
        raise ValueError(f"{path} is not a Mach-O binary")
    magic = struct.unpack_from("<I", data, 0)[0]
    if magic == 0xFEEDFACF:
        header = 32
    elif magic == 0xFEEDFACE:
        header = 28
    else:
        raise ValueError(f"{path} is not a thin little-endian Mach-O binary")
    ncmds = struct.unpack_from("<I", data, 16)[0]
    offset = header
    for _ in range(ncmds):
        if offset + 8 > len(data):
            break
        cmd, size = struct.unpack_from("<II", data, offset)
        if cmd == 0x32 and offset + 16 <= len(data):  # LC_BUILD_VERSION: cmd, size, platform, minos
            version = struct.unpack_from("<I", data, offset + 12)[0]
            return (version >> 16, (version >> 8) & 0xFF)
        if cmd == 0x24 and offset + 12 <= len(data):  # LC_VERSION_MIN_MACOSX: cmd, size, version
            version = struct.unpack_from("<I", data, offset + 8)[0]
            return (version >> 16, (version >> 8) & 0xFF)
        if size < 8:
            break
        offset += size
    raise ValueError(f"{path} does not say which macOS it needs")


def check_macos_minimum(binary, tag):
    """Refuse a wheel whose macOS tag promises older systems than the binary supports."""
    parts = tag.split("_")  # macosx_<major>_<minor>_<arch>
    claimed = (int(parts[1]), int(parts[2]))
    needs = macho_minos(binary)
    if needs > claimed:
        raise ValueError(f"{binary} needs macOS {needs[0]}.{needs[1]} but the wheel tag {tag} promises {claimed[0]}.{claimed[1]}")


def _record_hash(data):
    return "sha256=" + base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=").decode()


def _metadata(version, readme_text, license_names):
    lines = [
        "Metadata-Version: 2.4",
        f"Name: {NAME}",
        f"Version: {version}",
        f"Summary: {SUMMARY}",
        f"Project-URL: Homepage, {HOMEPAGE}",
        f"Project-URL: Source, {HOMEPAGE}",
        "Classifier: Development Status :: 2 - Pre-Alpha",
        "License-Expression: MIT",
    ]
    lines += [f"License-File: {n}" for n in license_names]
    lines += [f"Requires-Python: {REQUIRES_PYTHON}", "Description-Content-Type: text/markdown", "", readme_text]
    return "\n".join(lines) + "\n"


def build_wheel(version, binary, server_binary, tags, out_dir, license_file, notices_file, client_source, init_source, readme):
    """Write one wheel and return its path. Raises FileNotFoundError if an input file is missing."""
    def read(path):
        with open(path, "rb") as f:
            return f.read()

    command = read(binary)
    server = read(server_binary)
    dist_info = f"{NAME}-{version}.dist-info"
    data_dir = f"{NAME}-{version}.data"
    files = {  # name -> (bytes, mode)
        f"{NAME}/__init__.py": (read(init_source), 0o644),
        f"{NAME}/client.py": (read(client_source), 0o644),
        f"{data_dir}/scripts/{NAME}": (command, 0o755),
        f"{data_dir}/scripts/{SERVER_NAME}": (server, 0o755),
        f"{dist_info}/licenses/LICENSE": (read(license_file), 0o644),
        f"{dist_info}/licenses/THIRD-PARTY-LICENSES.txt": (read(notices_file), 0o644),
    }
    readme_text = read(readme).decode("utf-8").rstrip("\n")
    files[f"{dist_info}/METADATA"] = (_metadata(version, readme_text, ["LICENSE", "THIRD-PARTY-LICENSES.txt"]).encode(), 0o644)
    wheel_lines = ["Wheel-Version: 1.0", "Generator: klatch build_wheels.py", "Root-Is-Purelib: false"]
    wheel_lines += [f"Tag: py3-none-{t}" for t in tags]
    files[f"{dist_info}/WHEEL"] = (("\n".join(wheel_lines) + "\n").encode(), 0o644)

    record = [f"{n},{_record_hash(d)},{len(d)}" for n, (d, _) in sorted(files.items())]
    record.append(f"{dist_info}/RECORD,,")
    files[f"{dist_info}/RECORD"] = (("\n".join(record) + "\n").encode(), 0o644)

    epoch = int(os.environ.get("SOURCE_DATE_EPOCH", DEFAULT_EPOCH))
    stamp = max(epoch, DEFAULT_EPOCH)
    import time
    date_time = time.gmtime(stamp)[:6]

    os.makedirs(out_dir, exist_ok=True)
    path = os.path.join(out_dir, f"{NAME}-{version}-py3-none-{'.'.join(tags)}.whl")
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as zf:
        for name in sorted(files, key=lambda n: (n.endswith("/RECORD"), n)):
            data, mode = files[name]
            info = zipfile.ZipInfo(name, date_time)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.create_system = 3
            info.external_attr = (0o100000 | mode) << 16
            zf.writestr(info, data)
    return path


def from_archive(version, target, archive, out_dir, repo_root):
    """Build the wheel for a release target from its release archive (the binary, LICENSE and the notices in it)."""
    tags = tags_for(target)
    with tarfile.open(archive) as tar, _scratch() as work:
        members = {os.path.basename(m.name): m for m in tar.getmembers() if m.isfile()}
        for need in ("klatch", "klatch-server", "LICENSE", "THIRD-PARTY-LICENSES.txt"):
            if need not in members:
                raise ValueError(f"{archive} does not contain {need}")
            with open(os.path.join(work, need), "wb") as f:
                f.write(tar.extractfile(members[need]).read())
        binary = os.path.join(work, "klatch")
        server_binary = os.path.join(work, "klatch-server")
        if target.endswith("apple-darwin"):
            check_macos_minimum(binary, tags[0])
            check_macos_minimum(server_binary, tags[0])
        return build_wheel(
            version=version, binary=binary, server_binary=server_binary, tags=tags, out_dir=out_dir,
            license_file=os.path.join(work, "LICENSE"), notices_file=os.path.join(work, "THIRD-PARTY-LICENSES.txt"),
            client_source=os.path.join(repo_root, "clients", "python", "klatch_client.py"),
            init_source=os.path.join(repo_root, "clients", "python", "package", "wheel_init.py"),
            readme=os.path.join(repo_root, "clients", "python", "README.md"),
        )


class _scratch:
    def __enter__(self):
        import tempfile
        self._dir = tempfile.TemporaryDirectory(prefix="klatch-wheel-")
        return self._dir.__enter__()

    def __exit__(self, *exc):
        return self._dir.__exit__(*exc)


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("--version", required=True, help="the PyPI version, for example 0.1.0.dev1")
    p.add_argument("--target", required=True, choices=sorted(TARGET_TAGS))
    p.add_argument("--archive", required=True, help="the release archive of that target")
    p.add_argument("--out-dir", required=True)
    args = p.parse_args(argv)
    repo_root = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
    print(from_archive(args.version, args.target, args.archive, args.out_dir, repo_root))
    return 0


if __name__ == "__main__":
    sys.exit(main())
