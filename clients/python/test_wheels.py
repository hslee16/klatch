"""The platform wheels (#431): `pip install klatch` on a supported machine gives the klatch command and the Python
client. build_wheels.py makes them from the release binaries; these tests build wheels around a stand-in binary and
check them the way an installer would, then install one with pip and run the installed command."""
import base64
import hashlib
import os
import stat
import struct
import subprocess
import sys
import tempfile
import unittest
import zipfile

import build_wheels as bw

HERE = os.path.dirname(os.path.abspath(__file__))
FAKE = b"#!/bin/sh\necho 'klatch 0.1.0-dev.1 (protocol 1, store schema 22)'\n"


def make_wheel(out, tags=("any",), version="0.1.0.dev1"):
    work = tempfile.mkdtemp(prefix="klatch-wheel-src-")
    binary = os.path.join(work, "klatch")
    with open(binary, "wb") as f:
        f.write(FAKE)
    os.chmod(binary, 0o755)
    notices = os.path.join(work, "THIRD-PARTY-LICENSES.txt")
    with open(notices, "w") as f:
        f.write("notices\n")
    return bw.build_wheel(
        version=version,
        binary=binary,
        tags=list(tags),
        out_dir=out,
        license_file=os.path.join(HERE, "LICENSE"),
        notices_file=notices,
        client_source=os.path.join(HERE, "klatch_client.py"),
        init_source=os.path.join(HERE, "package", "wheel_init.py"),
        readme=os.path.join(HERE, "README.md"),
    )


def digest(data):
    return "sha256=" + base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=").decode()


class WheelContents(unittest.TestCase):
    def setUp(self):
        self.out = tempfile.mkdtemp(prefix="klatch-wheel-out-")

    def test_the_filename_carries_every_platform_tag(self):
        path = make_wheel(self.out, ("manylinux_2_17_x86_64", "musllinux_1_1_x86_64"))
        self.assertEqual(
            os.path.basename(path),
            "klatch-0.1.0.dev1-py3-none-manylinux_2_17_x86_64.musllinux_1_1_x86_64.whl",
        )

    def test_it_holds_the_client_the_command_and_the_licences(self):
        path = make_wheel(self.out)
        names = set(zipfile.ZipFile(path).namelist())
        for want in (
            "klatch/__init__.py",
            "klatch/client.py",
            "klatch-0.1.0.dev1.data/scripts/klatch",
            "klatch-0.1.0.dev1.dist-info/METADATA",
            "klatch-0.1.0.dev1.dist-info/WHEEL",
            "klatch-0.1.0.dev1.dist-info/RECORD",
            "klatch-0.1.0.dev1.dist-info/licenses/LICENSE",
            "klatch-0.1.0.dev1.dist-info/licenses/THIRD-PARTY-LICENSES.txt",
        ):
            self.assertIn(want, names)

    def test_the_command_is_executable_inside_the_wheel(self):
        zf = zipfile.ZipFile(make_wheel(self.out))
        info = zf.getinfo("klatch-0.1.0.dev1.data/scripts/klatch")
        mode = info.external_attr >> 16
        self.assertTrue(stat.S_ISREG(mode) and mode & 0o111, oct(mode))
        # the files that are not programs are not executable
        client = zf.getinfo("klatch/client.py").external_attr >> 16
        self.assertFalse(client & 0o111, oct(client))

    def test_wheel_file_says_it_is_platform_specific(self):
        zf = zipfile.ZipFile(make_wheel(self.out, ("macosx_11_0_arm64",)))
        text = zf.read("klatch-0.1.0.dev1.dist-info/WHEEL").decode()
        self.assertIn("Wheel-Version: 1.0", text)
        self.assertIn("Root-Is-Purelib: false", text)
        self.assertIn("Tag: py3-none-macosx_11_0_arm64", text)

    def test_every_tag_is_listed_in_the_wheel_file(self):
        zf = zipfile.ZipFile(make_wheel(self.out, ("manylinux_2_17_aarch64", "musllinux_1_1_aarch64")))
        text = zf.read("klatch-0.1.0.dev1.dist-info/WHEEL").decode()
        self.assertIn("Tag: py3-none-manylinux_2_17_aarch64", text)
        self.assertIn("Tag: py3-none-musllinux_1_1_aarch64", text)

    def test_record_lists_every_file_with_its_real_hash_and_size(self):
        zf = zipfile.ZipFile(make_wheel(self.out))
        record = zf.read("klatch-0.1.0.dev1.dist-info/RECORD").decode().splitlines()
        listed = {}
        for line in record:
            name, h, size = line.rsplit(",", 2)
            listed[name] = (h, size)
        self.assertEqual(set(listed), set(zf.namelist()))
        self.assertEqual(listed["klatch-0.1.0.dev1.dist-info/RECORD"], ("", ""))
        for name, (h, size) in listed.items():
            if name.endswith("/RECORD"):
                continue
            data = zf.read(name)
            self.assertEqual(h, digest(data), name)
            self.assertEqual(int(size), len(data), name)

    def test_metadata_matches_the_package(self):
        zf = zipfile.ZipFile(make_wheel(self.out))
        text = zf.read("klatch-0.1.0.dev1.dist-info/METADATA").decode()
        for want in (
            "Metadata-Version: 2.4",
            "Name: klatch",
            "Version: 0.1.0.dev1",
            "Requires-Python: >=3.9",
            "License-Expression: MIT",
            "License-File: LICENSE",
            "Project-URL: Source, https://github.com/hslee16/klatch",
            "Description-Content-Type: text/markdown",
            "early pre-release",  # the README is the description
        ):
            self.assertIn(want, text)

    def test_the_wheel_is_reproducible(self):
        a = make_wheel(tempfile.mkdtemp(), ("any",))
        b = make_wheel(tempfile.mkdtemp(), ("any",))
        self.assertEqual(open(a, "rb").read(), open(b, "rb").read())

    def test_a_missing_binary_is_an_error(self):
        with self.assertRaises(FileNotFoundError):
            bw.build_wheel(
                version="0.1.0.dev1", binary="/nonexistent/klatch", tags=["any"], out_dir=self.out,
                license_file=os.path.join(HERE, "LICENSE"), notices_file=os.path.join(HERE, "LICENSE"),
                client_source=os.path.join(HERE, "klatch_client.py"),
                init_source=os.path.join(HERE, "package", "wheel_init.py"),
                readme=os.path.join(HERE, "README.md"),
            )


class Installing(unittest.TestCase):
    def test_pip_installs_it_and_the_command_and_the_client_work(self):
        out = tempfile.mkdtemp(prefix="klatch-wheel-out-")
        wheel = make_wheel(out)
        venv = tempfile.mkdtemp(prefix="klatch-venv-")
        subprocess.run([sys.executable, "-m", "venv", venv], check=True)
        py = os.path.join(venv, "bin", "python")
        subprocess.run([py, "-m", "pip", "install", "--no-deps", "--no-index", "-q", wheel], check=True)
        command = os.path.join(venv, "bin", "klatch")
        self.assertTrue(os.access(command, os.X_OK), "no executable klatch in the venv's bin")
        ran = subprocess.run([command, "--version"], capture_output=True, text=True, check=True)
        self.assertIn("klatch 0.1.0-dev.1", ran.stdout)
        client = subprocess.run(
            [py, "-I", "-c", "import klatch; print(klatch.__version__, klatch.PROTOCOL_VERSION)"],
            capture_output=True, text=True, check=True, cwd=tempfile.gettempdir(),
        )
        self.assertEqual(client.stdout.split(), ["0.1.0.dev1", "1"])


class Tags(unittest.TestCase):
    def test_each_release_target_has_its_platform_tags(self):
        self.assertEqual(bw.tags_for("aarch64-apple-darwin"), ["macosx_11_0_arm64"])
        self.assertEqual(bw.tags_for("x86_64-apple-darwin"), ["macosx_10_12_x86_64"])
        self.assertEqual(
            bw.tags_for("x86_64-unknown-linux-musl"), ["manylinux_2_17_x86_64", "musllinux_1_1_x86_64"]
        )
        self.assertEqual(
            bw.tags_for("aarch64-unknown-linux-musl"), ["manylinux_2_17_aarch64", "musllinux_1_1_aarch64"]
        )

    def test_the_dynamic_linux_builds_get_no_wheel(self):
        # the static build is the Linux wheel: it needs no glibc, so one wheel serves glibc and musl systems
        for target in ("x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu", "x86_64-pc-windows-msvc"):
            with self.assertRaises(ValueError):
                bw.tags_for(target)


def macho(cmds):
    """A minimal 64-bit Mach-O header followed by the given load commands (bytes)."""
    body = b"".join(cmds)
    header = struct.pack("<IiiIIIII", 0xFEEDFACF, 0x0100000C, 0, 2, len(cmds), len(body), 0, 0)
    return header + body


class MachO(unittest.TestCase):
    def write(self, data):
        f = tempfile.NamedTemporaryFile(delete=False)
        f.write(data)
        f.close()
        self.addCleanup(os.unlink, f.name)
        return f.name

    def test_build_version_command(self):
        # LC_BUILD_VERSION (0x32): platform, minos, sdk, ntools; 11.0 is 0x000B0000
        cmd = struct.pack("<IIIIII", 0x32, 24, 1, 0x000B0000, 0x001A0500, 0)
        self.assertEqual(bw.macho_minos(self.write(macho([cmd]))), (11, 0))

    def test_version_min_command(self):
        # LC_VERSION_MIN_MACOSX (0x24): version, sdk; 10.12 is 0x000A0C00
        cmd = struct.pack("<IIII", 0x24, 16, 0x000A0C00, 0x000F0500)
        self.assertEqual(bw.macho_minos(self.write(macho([cmd]))), (10, 12))

    def test_a_binary_that_needs_a_newer_macos_than_its_tag_is_refused(self):
        cmd = struct.pack("<IIIIII", 0x32, 24, 1, 0x000E0000, 0x001A0500, 0)  # needs macOS 14
        with self.assertRaises(ValueError):
            bw.check_macos_minimum(self.write(macho([cmd])), "macosx_11_0_arm64")

    def test_a_binary_within_its_tag_is_accepted(self):
        cmd = struct.pack("<IIIIII", 0x32, 24, 1, 0x000B0000, 0x001A0500, 0)
        bw.check_macos_minimum(self.write(macho([cmd])), "macosx_11_0_arm64")

    def test_not_a_macho_file_is_an_error(self):
        with self.assertRaises(ValueError):
            bw.macho_minos(self.write(b"#!/bin/sh\n"))


if __name__ == "__main__":
    unittest.main()
