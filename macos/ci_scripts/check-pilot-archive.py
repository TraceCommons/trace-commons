#!/usr/bin/python3
"""Validate an archive before Cloud's separate distribution/notary action."""
import ctypes
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import subprocess
import sys


def require(condition, reason):
    if not condition:
        raise ValueError(reason)


def check_runtime(binary):
    # CodeSignOnCopy must carry the seeded hardened-runtime option into the
    # archive. Check every slice, since codesign defaults to the host slice.
    arches = subprocess.check_output(['lipo', '-archs', str(binary)], text=True).split()
    require(arches, 'no signed binary architectures')
    for arch in arches:
        signature = subprocess.run(['codesign', '-d', '--arch', arch, '--verbose=4', str(binary)],
                                   text=True, capture_output=True, check=True)
        flags = re.search(r'flags=0x([0-9a-fA-F]+)', signature.stderr)
        require(flags and int(flags.group(1), 16) & 0x10000, 'missing hardened runtime: ' + Path(binary).name + ' (' + arch + ')')


def check(archive, repo):
    archive, repo = Path(archive), Path(repo)
    generated = repo / 'macos/.xcode-cloud/generated'
    metadata = json.loads((generated / 'pilot-build.json').read_text())
    require(metadata['schemaVersion'] == 1, 'metadata schema')
    require(metadata['mode'] == 'release-universal', 'archive requires release-universal inputs')
    require(metadata['channel'] == 'production', 'metadata channel')
    commit = subprocess.check_output(['git', '-C', str(repo), 'rev-parse', 'HEAD'], text=True).strip()
    require(metadata['sourceCommit'] == commit, 'archive source checkout changed')
    if os.environ.get('CI_COMMIT'):
        require(metadata['sourceCommit'] == os.environ['CI_COMMIT'], 'archive CI source mismatch')
    if os.environ.get('CI_BUILD_NUMBER'):
        require(metadata['buildVersion'] == os.environ['CI_BUILD_NUMBER'], 'archive CI build mismatch')
    info = plistlib.loads((archive / 'Info.plist').read_bytes())
    require(info['ApplicationProperties']['ApplicationPath'] == 'Applications/Near.ai.app', 'archive product path')
    app = archive / 'Products/Applications/Near.ai.app'
    resources = app / 'Contents/Resources'
    require(json.loads((resources / 'pilot-build.json').read_text()) == metadata, 'embedded source metadata')
    plist = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
    expected = {'CFBundleIdentifier': 'ai.tracecommons.shell', 'CFBundleExecutable': 'TraceCommonsApp',
                'CFBundleShortVersionString': metadata['shortVersion'], 'CFBundleVersion': metadata['buildVersion'],
                'TCChannel': 'production', 'CFBundleIconName': 'AppIcon', 'CFBundleIconFile': 'AppIcon'}
    for key, value in expected.items():
        require(plist.get(key) == value, 'archive plist mismatch: ' + key)
    require(hashlib.sha256(plist['SUPublicEDKey'].encode()).hexdigest() == metadata['sparklePublicKeySHA256'], 'archive Sparkle public key')
    require(plist['SUFeedURL'] == 'https://storage.googleapis.com/tracecommons-flatpak/updates/appcast.xml', 'archive Sparkle feed')
    require((resources / 'en.lproj/InfoPlist.strings').read_bytes() == (generated / 'en.lproj/InfoPlist.strings').read_bytes(), 'localized display name')
    for resource in ('AppIcon.icns', 'Assets.car', 'TraceCommons_TCShellCore.bundle'):
        require((resources / resource).exists(), 'missing archive resource: ' + resource)
    framework = app / 'Contents/Frameworks/Sparkle.framework'
    for nested in ('Sparkle', 'Versions/B/Autoupdate', 'Versions/B/Updater.app',
                   'Versions/B/XPCServices/Downloader.xpc', 'Versions/B/XPCServices/Installer.xpc'):
        require((framework / nested).exists(), 'missing Sparkle component: ' + nested)
    dylib = app / 'Contents/Frameworks/libtrace_commons_contributor_ffi.dylib'
    binaries = (app / 'Contents/MacOS/TraceCommonsApp', app / 'Contents/MacOS/near-ai', dylib, framework / 'Sparkle')
    for binary in binaries:
        arches = subprocess.check_output(['lipo', '-archs', str(binary)], text=True).split()
        require(set(arches) == {'arm64', 'x86_64'} and len(arches) == 2, 'archive binary architectures')
    check_runtime(binaries[1])
    check_runtime(dylib)
    identifier = subprocess.check_output(['otool', '-D', str(dylib)], text=True)
    require('@rpath/libtrace_commons_contributor_ffi.dylib' in identifier.splitlines()[1:], 'archive dylib install name')
    linked = subprocess.check_output(['otool', '-L', str(binaries[0])], text=True)
    require('@rpath/libtrace_commons_contributor_ffi.dylib' in linked, 'app FFI linkage')
    cli_version = subprocess.check_output([str(binaries[1]), '--version'], text=True).strip()
    require(re.fullmatch(r'trace-commons-contributor ' + re.escape(metadata['shortVersion']) + r' \(commit ' + re.escape(metadata['sourceCommit']) + r', built [0-9T:Z-]+\) channel production', cli_version), 'archive CLI version, source or channel')
    # This function reads compile-time channel constants only; it starts no
    # daemon, opens no keychain, and performs no network operations.
    library = ctypes.CDLL(str(dylib))
    library.tc_build_channel_json.restype = ctypes.c_void_p
    library.tc_string_free.argtypes = [ctypes.c_void_p]
    pointer = library.tc_build_channel_json()
    require(pointer, 'archive FFI channel unavailable')
    try:
        channel = json.loads(ctypes.string_at(pointer))
    finally:
        library.tc_string_free(pointer)
    require(channel['channel'] == 'production', 'archive FFI channel')
    print('Validated private universal archive source metadata, version, channel, linkage and resources; distribution/notarization is a later action.')


if __name__ == '__main__':
    try:
        if len(sys.argv) >= 3 and sys.argv[1] == '--check-runtime':
            for binary in sys.argv[2:]:
                check_runtime(binary)
        else:
            require(len(sys.argv) == 3, 'usage: check-pilot-archive.py <archive> <repository>')
            check(sys.argv[1], sys.argv[2])
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        print('refusing: ' + str(error), file=sys.stderr)
        sys.exit(1)
