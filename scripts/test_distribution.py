"""Distribution architecture guards, without downloads or release builds.

Run: uv run --no-project python -m unittest discover -s scripts -p test_distribution.py
"""

import hashlib
import os
import shutil
import tarfile
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent


class DistributionTests(unittest.TestCase):
    def setUp(self):
        self.work = tempfile.TemporaryDirectory(prefix="h0x-ade-distribution-")
        self.addCleanup(self.work.cleanup)
        self.bin = Path(self.work.name)
        self.stub("uname", 'case "$1" in -s) echo "$TEST_OS";; -m) echo "$TEST_ARCH";; esac')
        self.stub("curl", 'echo "unexpected download" >&2; exit 97')
        self.stub("cargo", 'echo "unexpected build" >&2; exit 97')
        self.stub("sysctl", 'exit 1')

    def stub(self, name, body):
        path = self.bin / name
        path.write_text("#!/bin/sh\n" + body + "\n")
        path.chmod(0o755)

    def run_script(self, script, os_name, arch, *args, **extra):
        env = dict(os.environ, PATH=f"{self.bin}:{os.environ['PATH']}", TEST_OS=os_name, TEST_ARCH=arch, **extra)
        return subprocess.run(
            [str(ROOT / script), *args], cwd=ROOT, env=env,
            capture_output=True, text=True, check=False,
        )

    def test_intel_mac_installer_stops_before_download(self):
        for arch in ("x86_64", "amd64"):
            with self.subTest(arch=arch):
                result = self.run_script("distribution/install.sh", "Darwin", arch)
                self.assertEqual(result.returncode, 1)
                self.assertIn("Intel Macs aren't supported; build from source", result.stderr)
                self.assertNotIn("unexpected download", result.stderr)

    def test_windows_shells_are_pointed_at_the_powershell_installer(self):
        for os_name in ("MINGW64_NT-10.0-26300", "MSYS_NT-10.0-26300", "CYGWIN_NT-10.0-26300"):
            with self.subTest(os_name=os_name):
                result = self.run_script("distribution/install.sh", os_name, "x86_64")
                self.assertEqual(result.returncode, 1)
                self.assertIn("/latest/download/install.ps1 | iex", result.stderr)
                self.assertNotIn("unexpected download", result.stderr)

    def test_unknown_os_installer_names_it(self):
        result = self.run_script("distribution/install.sh", "FreeBSD", "x86_64")
        self.assertEqual(result.returncode, 1)
        self.assertIn("no build for FreeBSD", result.stderr)
        self.assertNotIn("unexpected download", result.stderr)

    def test_rosetta_mac_installer_uses_arm64(self):
        for key in ("hw.optional.arm64", "sysctl.proc_translated"):
            with self.subTest(key=key):
                self.stub("sysctl", f'[ "$2" = "{key}" ] && echo 1')
                self.stub("curl", '''while [ "$#" -gt 0 ]; do
  if [ "$1" = -o ]; then
    printf '%s' '{"version":"test","macos-aarch64":{"url":"test","sha256":"test"}}' > "$2"
    exit 0
  fi
  shift
done
exit 97''')
                result = self.run_script("distribution/install.sh", "Darwin", "x86_64", H0X_ADE_RELEASES_URL="https://releases.example")
                self.assertEqual(result.returncode, 1)
                self.assertIn("Installing HyperOx test for macos-aarch64", result.stdout)
                self.assertIn("download is damaged", result.stderr)
                self.assertNotIn("Intel Macs", result.stderr)

    def test_installer_refuses_a_download_off_github_and_needs_https(self):
        for url in ("https://evil.example/h0x-ade-linux-x86_64.tar.gz", "http://github.com/x/h0x-ade-linux-x86_64.tar.gz"):
            with self.subTest(url=url):
                manifest = Path(self.work.name) / "latest.json"
                manifest.write_text('{"version":"1.0.0","core_min":"0.1.13","targets":{"linux-x86_64":{"url":"%s","sha256":"x"}}}' % url)
                log = Path(self.work.name) / "urls"
                log.unlink(missing_ok=True)
                self.stub("curl", f'''url=; out=; prev=
for a; do
  if [ "$prev" = -o ]; then out=$a; else case $a in -*) ;; *) url=$a ;; esac; fi
  prev=$a
done
echo "$url" >> "{log}"
case $url in */latest.json) cp "{manifest}" "$out"; exit 0 ;; esac
exit 22''')
                result = self.run_script("distribution/install.sh", "Linux", "x86_64")
                self.assertEqual(result.returncode, 1)
                self.assertIn("refusing to download", result.stderr)
                self.assertEqual(len(log.read_text().splitlines()), 1, "only the manifest was fetched")

    APP_URL = "https://github.com/vjk7989/h0x-ADE-app/releases/download/v1.0.0/h0x-ade-linux-x86_64.tar.gz"

    def release_install(self, url=APP_URL, core_min="0.1.13", core=None, core_status=1, **extra):
        """Runs install.sh on Linux against the real release sites (stubbed curl).

        `core`: None (no h0x-core yet; the core installer writes one), "present"
        (an h0x-core in ~/.local/bin already) or "not-executable". The h0x-core
        answers `update --from … --check` with `core_status`. Returns the result,
        the fetched URLs, every curl command line, the core installer's
        `H0X_CORE_VERSION|H0X_CORE_INSTALL_DIR|H0X_CORE_RELEASES_URL` (or None if
        it never ran), the h0x-core calls, whether tar ran, and the home."""
        if shutil.which("h0x-core"):
            self.skipTest("an h0x-core on this machine's PATH")
        work = Path(self.work.name)
        home = work / "home"
        shutil.rmtree(home, ignore_errors=True)
        bin_dir = home / ".local/bin"
        source = work / "source/h0x-ade"
        source.mkdir(parents=True, exist_ok=True)
        (source / "h0x-ade").write_text("app")
        archive = work / "release.tar.gz"
        with tarfile.open(archive, "w:gz") as tar:
            tar.add(source, arcname="h0x-ade")
        sha = hashlib.sha256(archive.read_bytes()).hexdigest()
        pin = "" if core_min is None else ',"core_min":"%s"' % core_min
        manifest = work / "latest.json"
        manifest.write_text('{"version":"1.0.0"%s,"targets":{"linux-x86_64":{"url":"%s","sha256":"%s"}}}' % (pin, url, sha), encoding="utf-8")
        calls = work / "core-calls"
        core_stub = f'#!/bin/sh\necho "$*" >> "{calls}"\nexit {core_status}\n'
        core_env = work / "core-env"
        core_installer = work / "core-installer.sh"
        core_installer.write_text(f'''printf '%s|%s|%s\\n' "${{H0X_CORE_VERSION-unset}}" "${{H0X_CORE_INSTALL_DIR-unset}}" "${{H0X_CORE_RELEASES_URL-unset}}" > "{core_env}"
mkdir -p "$H0X_CORE_INSTALL_DIR"
printf '%s' '{core_stub}' > "$H0X_CORE_INSTALL_DIR/h0x-core"
chmod 755 "$H0X_CORE_INSTALL_DIR/h0x-core"
''')
        if core is not None:
            bin_dir.mkdir(parents=True)
            (bin_dir / "h0x-core").write_text(core_stub)
            (bin_dir / "h0x-core").chmod(0o644 if core == "not-executable" else 0o755)
        for name in ("urls", "args", "tar-calls"):
            (work / name).unlink(missing_ok=True)
        for name in (calls, core_env):
            name.unlink(missing_ok=True)
        url_log, arg_log, tar_log = work / "urls", work / "args", work / "tar-calls"
        self.stub("curl", f'''url=; out=; prev=
for a; do
  if [ "$prev" = -o ]; then out=$a; else case $a in -*) ;; *) url=$a ;; esac; fi
  prev=$a
done
echo "$url" >> "{url_log}"
echo "$*" >> "{arg_log}"
case $url in
  */latest/download/latest.json) cp "{manifest}" "$out" ;;
  *.minisig) echo signature > "$out" ;;
  */install.sh) cp "{core_installer}" "$out" ;;
  *h0x-ade-linux-x86_64.tar.gz*) cp "{archive}" "$out" ;;
  *) exit 22 ;;
esac''')
        self.stub("tar", f'echo "$*" >> "{tar_log}"; echo "unexpected unpack" >&2; exit 97')
        env = {k: v for k, v in os.environ.items() if k not in ("H0X_ADE_RELEASES_URL", "H0X_CORE_RELEASES_URL", "H0X_CORE_VERSION", "H0X_CORE_INSTALL_DIR")}
        env.update(PATH=f"{self.bin}:{os.environ['PATH']}", TEST_OS="Linux", TEST_ARCH="x86_64",
                   HOME=str(home), XDG_DATA_HOME=str(home / ".local/share"), **extra)
        result = subprocess.run([str(ROOT / "distribution/install.sh")], cwd=ROOT, env=env,
                                capture_output=True, text=True, check=False)
        read = lambda p: p.read_text().splitlines() if p.exists() else []
        ran_core_installer = core_env.read_text().strip() if core_env.exists() else None
        return result, read(url_log), read(arg_log), ran_core_installer, read(calls), tar_log.exists(), home

    def assert_nothing_installed(self, home):
        self.assertFalse((home / ".local/share/h0x-ade").exists(), "the app was unpacked")
        self.assertFalse((home / ".local/bin/h0x-ade").exists(), "the app was linked")

    def test_installer_refuses_look_alike_github_hosts(self):
        for host in ("github.com.evil.com", "evil.com/github.com", "user@github.com@evil.com",
                     "github.com@evil.com", "GITHUB.COM", "xn--github-9ya.com", "gіthub.com",
                     "github.com:8443", "objects.githubusercontent.com.evil.com",
                     "evil-githubusercontent.com"):
            for scheme in ("https://", "http://"):
                url = f"{scheme}{host}/vjk7989/h0x-ADE-app/releases/download/v1.0.0/h0x-ade-linux-x86_64.tar.gz"
                with self.subTest(url=url):
                    result, urls, _, core_env, _, unpacked, home = self.release_install(url=url)
                    self.assertEqual(result.returncode, 1, result.stderr)
                    self.assertIn("refusing to download", result.stderr)
                    self.assertEqual(len(urls), 1, f"only the manifest was fetched: {urls}")
                    self.assertIsNone(core_env, "no core installer ran")
                    self.assertFalse(unpacked)
                    self.assertNotIn("h0x-core", os.listdir(home / ".local/bin") if (home / ".local/bin").exists() else [])
        # Plain http to a real GitHub host is refused too.
        for url in (self.APP_URL.replace("https://", "http://"),
                    "http://objects.githubusercontent.com/x/h0x-ade-linux-x86_64.tar.gz",
                    "github.com/vjk7989/h0x-ADE-app/releases/download/v1.0.0/h0x-ade-linux-x86_64.tar.gz",
                    "//github.com/vjk7989/h0x-ADE-app/releases/download/v1.0.0/h0x-ade-linux-x86_64.tar.gz"):
            with self.subTest(url=url):
                result, urls, *_ = self.release_install(url=url)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertIn("refusing to download", result.stderr)
                self.assertEqual(len(urls), 1, urls)

    def test_installer_checks_the_app_signature_with_the_pinned_core_before_unpacking(self):
        # A query (a signed mirror URL) is not part of the file name or the signature's URL.
        url = self.APP_URL + "?x=1"
        result, urls, args, core_env, calls, unpacked, home = self.release_install(url=url, core_status=1)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("not signed by the HyperOx release key", result.stderr)
        # The pinned core, never "latest", from a downloaded file (not piped).
        self.assertIn("https://github.com/vjk7989/h0x-ADE-core/releases/download/v0.1.13/install.sh", urls)
        self.assertFalse([u for u in urls if "/latest/" in u and not u.endswith("/latest/download/latest.json")], urls)
        # It runs with the version and the folder, and without a releases site: one set
        # here would switch off the core installer's own signature check.
        bin_dir = home / ".local/bin"
        self.assertEqual(core_env, f"0.1.13|{bin_dir}|unset")
        self.assertIn(self.APP_URL + ".minisig", urls)
        self.assertEqual(len(calls), 1, calls)
        self.assertRegex(calls[0], r"^update --from .*/h0x-ade-linux-x86_64\.tar\.gz --check$")
        # https only, on redirects too.
        for line in args:
            with self.subTest(curl=line):
                self.assertIn("--proto =https ", line)
                self.assertIn("--proto-redir =https ", line)
        self.assertFalse(unpacked, "unpacked before the signature was checked")
        self.assert_nothing_installed(home)

    def test_installer_stops_before_unpacking_when_the_core_cannot_check(self):
        # A core older than 0.1.9 has no `update --check` (exit 2); one that is there but
        # won't run; either way nothing from the download is unpacked or installed.
        for core, status in (("present", 2), ("present", 1), ("not-executable", 0)):
            with self.subTest(core=core, status=status):
                result, urls, _, core_env, _, unpacked, home = self.release_install(core=core, core_status=status)
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                self.assertIn("h0x-core", result.stderr, "the failure says why")
                self.assertIsNone(core_env, "a core that is there is left alone")
                self.assertFalse([u for u in urls if "h0x-ADE-core" in u], urls)
                self.assertFalse(unpacked)
                self.assert_nothing_installed(home)

    def test_installer_refuses_a_missing_or_unsafe_core_pin(self):
        for pin in (None, "", "latest", "../x", "0.1.13/../../x", "v0.1.13", "0.1.13?x=1", "0.1.13#x"):
            with self.subTest(core_min=pin):
                result, urls, _, core_env, calls, unpacked, home = self.release_install(core_min=pin)
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                self.assertIn("needs h0x-core", result.stderr)
                self.assertFalse([u for u in urls if "h0x-ADE-core" in u], f"fetched a core installer: {urls}")
                self.assertIsNone(core_env)
                self.assertEqual(calls, [])
                self.assertFalse(unpacked)
                self.assert_nothing_installed(home)

    def test_installer_warns_loudly_when_a_releases_site_switches_off_signature_checks(self):
        # Set in a shell profile and forgotten, either one turns a check off: the installer must say so.
        for var, value in (("H0X_ADE_RELEASES_URL", "https://releases.example"),
                           ("H0X_CORE_RELEASES_URL", "http://127.0.0.1:9/core")):
            with self.subTest(var=var):
                url = (value + "/download/v1.0.0/h0x-ade-linux-x86_64.tar.gz") if var == "H0X_ADE_RELEASES_URL" else self.APP_URL
                result, *_ = self.release_install(url=url, core_status=0, **{var: value})
                self.assertIn(var, result.stderr, "the warning names the variable")
                self.assertIn("signature", result.stderr.lower(), "the warning says what is not checked")

    def download_url(self, url, releases="https://releases.example", installed=False):
        """The URL install.sh downloads the app from, for a manifest giving `url`."""
        home = Path(self.work.name) / "home"
        data = home / ".local/share"
        shutil.rmtree(data / "h0x-ade", ignore_errors=True)
        (data / "h0x-ade" if installed else data).mkdir(parents=True, exist_ok=True)
        log = Path(self.work.name) / "urls"
        log.unlink(missing_ok=True)
        manifest = Path(self.work.name) / "latest.json"
        manifest.write_text('{"version":"1.0.0","targets":{"linux-x86_64":{"url":"%s","sha256":"x"}}}' % url)
        self.stub("curl", f'''out=; url=; prev=
for a; do
  if [ "$prev" = -o ]; then out=$a; else case $a in -*) ;; *) url=$a ;; esac; fi
  prev=$a
done
echo "$url" >> "{log}"
case $url in */latest.json) cp "{manifest}" "$out"; exit 0 ;; esac
exit 22''')
        env = dict(os.environ, PATH=f"{self.bin}:{os.environ['PATH']}", TEST_OS="Linux", TEST_ARCH="x86_64",
                   HOME=str(home), XDG_DATA_HOME=str(data), H0X_ADE_RELEASES_URL=releases)
        result = subprocess.run([str(ROOT / "distribution/install.sh")], cwd=ROOT, env=env,
                                capture_output=True, text=True, check=False)
        self.assertIn("download failed", result.stderr)
        return log.read_text().splitlines()[-1]

    def test_installer_downloads_the_manifest_url_as_given(self):
        r = "https://releases.example"
        tarball = f"{r}/download/v1.0.0/h0x-ade-linux-x86_64.tar.gz"
        self.assertEqual(self.download_url(tarball), tarball)
        self.assertEqual(self.download_url(tarball, installed=True), tarball)
        # A mirror's query (a signed URL) or fragment, or another host: exactly as given.
        for url in (f"{tarball}?sig=a%2Fb&exp=1", f"{tarball}#x",
                    "https://mirror.example/download/v1.0.0/h0x-ade-linux-x86_64.tar.gz",
                    "https://releases.example.evil/download/v1.0.0/h0x-ade-linux-x86_64.tar.gz"):
            with self.subTest(url=url):
                self.assertEqual(self.download_url(url), url)
        # A test server elsewhere: its URLs too.
        local = "http://127.0.0.1:8000/e2e"
        self.assertEqual(self.download_url(f"{local}/download/v1.0.0/a.tar.gz", releases=local), f"{local}/download/v1.0.0/a.tar.gz")

    def mac_installer(self, signature, status, verify_status=0):
        home = self.bin / "home"
        (home / ".local/bin").mkdir(parents=True)
        (home / ".local/bin/h0x-core").touch()
        source = self.bin / "source/HyperOx.app/Contents"
        source.mkdir(parents=True)
        plist = b"signed release plist must stay unchanged"
        (source / "Info.plist").write_bytes(plist)
        archive = self.bin / "release.tar.gz"
        with tarfile.open(archive, "w:gz") as tar:
            tar.add(source.parent, arcname="HyperOx.app")
        sha = hashlib.sha256(archive.read_bytes()).hexdigest()
        manifest = self.bin / "manifest.json"
        manifest.write_text('{"version":"1.0.0","targets":{"macos-aarch64":{"url":"release","sha256":"%s"}}}' % sha)
        self.stub("curl", f'''url=; out=; prev=
for a; do
  if [ "$prev" = -o ]; then out=$a; else case $a in -*) ;; *) url=$a ;; esac; fi
  prev=$a
done
case $url in */latest.json) cp "{manifest}" "$out" ;; release) cp "{archive}" "$out" ;; *) exit 97 ;; esac''')
        installed = home / "Applications/HyperOx.app/Contents/Info.plist"
        installed.parent.mkdir(parents=True)
        installed.write_bytes(b"previous installed app")
        fixture = self.bin / "signature.txt"
        fixture.write_text(signature)
        log = self.bin / "codesign.log"
        self.stub("codesign", f'''echo "$*" >> "{log}"
case "$1" in
  -dvv) cat "{fixture}" >&2; exit {status} ;;
  --verify) exit {verify_status} ;;
  --force) exit 0 ;;
  *) exit 97 ;;
esac''')
        plist_log = self.bin / "plist.log"
        self.stub("PlistBuddy", f'''echo "$*" >> "{plist_log}"
printf 'dev PATH added' >> "$3"''')
        # Remap only the system tools in a scratch copy; production uses absolute paths.
        script = (ROOT / "distribution/install.sh").read_text()
        self.assertNotIn("\n    codesign ", script)
        script = script.replace("/usr/bin/codesign", str(self.bin / "codesign"))
        script = script.replace("/usr/libexec/PlistBuddy", str(self.bin / "PlistBuddy"))
        installer = self.bin / "install.sh"
        installer.write_text(script)
        env = dict(os.environ, PATH=f"{self.bin}:{os.environ['PATH']}", TEST_OS="Darwin",
                   TEST_ARCH="arm64", HOME=str(home), H0X_ADE_RELEASES_URL="https://releases.example")
        result = subprocess.run(["sh", str(installer)], cwd=ROOT, env=env,
                                capture_output=True, text=True, check=False)
        return result, installed.read_bytes(), log.read_text().splitlines(), plist_log.exists(), plist

    def test_mac_installer_preserves_certificate_signed_bundle(self):
        fixture = (ROOT / "scripts/fixtures/codesign/developer-id.txt").read_text()
        result, installed, calls, edited, plist = self.mac_installer(fixture, 0)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(installed, plist)
        self.assertFalse(edited)
        self.assertEqual(len(calls), 2)
        self.assertTrue(calls[0].startswith("-dvv "))
        self.assertTrue(calls[1].startswith("--verify --deep --strict "))

    def test_mac_installer_prepares_adhoc_bundle(self):
        self.check_dev_installer("adhoc", 0)

    def test_mac_installer_prepares_unsigned_bundle(self):
        self.check_dev_installer("unsigned", 1)

    def check_dev_installer(self, fixture_name, status):
        fixture = (ROOT / f"scripts/fixtures/codesign/{fixture_name}.txt").read_text()
        result, installed, calls, edited, plist = self.mac_installer(fixture, status)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(installed, plist + b"dev PATH added")
        self.assertTrue(edited)
        self.assertEqual(len(calls), 2)
        self.assertTrue(calls[1].startswith("--force --sign - "))

    def test_mac_installer_unknown_signature_leaves_installed_app_untouched(self):
        result, installed, calls, edited, _ = self.mac_installer("codesign: internal error", 1)
        self.assertEqual(result.returncode, 1)
        self.assertIn("cannot determine app signature: codesign: internal error", result.stderr)
        self.assertEqual(installed, b"previous installed app")
        self.assertFalse(edited)
        self.assertEqual(len(calls), 1)
        self.assertTrue(calls[0].startswith("-dvv "))

    def test_mac_installer_invalid_release_leaves_installed_app_untouched(self):
        fixture = (ROOT / "scripts/fixtures/codesign/developer-id.txt").read_text()
        result, installed, calls, edited, _ = self.mac_installer(fixture, 0, verify_status=1)
        self.assertEqual(result.returncode, 1)
        self.assertIn("invalid app signature", result.stderr)
        self.assertEqual(installed, b"previous installed app")
        self.assertFalse(edited)
        self.assertEqual(len(calls), 2)
        self.assertTrue(calls[1].startswith("--verify --deep --strict "))

    def test_intel_mac_package_stops_before_build(self):
        for args in ((), ("x86_64-apple-darwin",)):
            with self.subTest(args=args):
                result = self.run_script("scripts/package", "Darwin", "x86_64", *args)
                self.assertEqual(result.returncode, 1)
                self.assertIn("Intel Macs aren't supported; build from source", result.stderr)
                self.assertNotIn("unexpected build", result.stderr)

    def test_supported_installers_still_try_download(self):
        for os_name, arch in (("Darwin", "arm64"), ("Linux", "x86_64"), ("Linux", "aarch64")):
            with self.subTest(os=os_name, arch=arch):
                result = self.run_script("distribution/install.sh", os_name, arch)
                self.assertEqual(result.returncode, 1)
                self.assertIn("unexpected download", result.stderr)
                self.assertNotIn("Intel Macs", result.stderr)

    def test_packages_keep_full_notices_on_every_platform(self):
        checkout = self.bin / "checkout"
        for path in [
            "Cargo.toml", "LICENSE", "THIRD-PARTY-NOTICES", "scripts/package",
            "assets/icons/h0x-ade-icon.svg", "assets/icons/HyperOx.icns",
        ]:
            destination = checkout / path
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / path, destination)
        sign = checkout / "scripts/mac-sign"
        sign.write_text("#!/bin/sh\nexit 0\n")
        sign.chmod(0o755)
        release = checkout / "target/release"
        release.mkdir(parents=True)
        for name in ("h0x-ade", "h0x-ade.exe"):
            (release / name).write_bytes(b"test binary")
        self.stub("cargo", "exit 0")
        for os_name, arch, target, prefix in (
            ("Linux", "x86_64", "linux-x86_64", "h0x-ade"),
            ("Darwin", "arm64", "macos-aarch64", "HyperOx.app/Contents/Resources"),
            ("MINGW64_NT", "x86_64", "windows-x86_64", "h0x-ade"),
        ):
            with self.subTest(target=target):
                result = self.run_script(checkout / "scripts/package", os_name, arch)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                with tarfile.open(checkout / f"dist/h0x-ade-{target}.tar.gz") as archive:
                    for name in ("LICENSE", "THIRD-PARTY-NOTICES"):
                        self.assertEqual(
                            archive.extractfile(f"{prefix}/{name}").read(),
                            (ROOT / name).read_bytes(),
                        )
        self.stub("cargo", 'echo "unexpected build" >&2; exit 97')
        for name in ("LICENSE", "THIRD-PARTY-NOTICES"):
            notice = checkout / name
            original = notice.read_bytes()
            for state in ("empty", "missing"):
                with self.subTest(notice=name, state=state):
                    if state == "empty":
                        notice.write_bytes(b"")
                    else:
                        notice.unlink()
                    result = self.run_script(checkout / "scripts/package", "Linux", "x86_64")
                    self.assertEqual(result.returncode, 1)
                    self.assertIn(f"{name} is missing or empty", result.stderr)
                    self.assertNotIn("unexpected build", result.stderr)
            notice.write_bytes(original)

    def test_supported_packages_still_try_build(self):
        for os_name, arch in (("Darwin", "arm64"), ("Linux", "x86_64"), ("Linux", "aarch64")):
            with self.subTest(os=os_name, arch=arch):
                result = self.run_script("scripts/package", os_name, arch)
                self.assertEqual(result.returncode, 97)
                self.assertIn("unexpected build", result.stderr)
                self.assertNotIn("Intel Macs", result.stderr)


if __name__ == "__main__":
    unittest.main()
