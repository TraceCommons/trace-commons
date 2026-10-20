# Build the IronWire menu bar app.
#
# There is no Xcode project on purpose (see README.md). Everything here runs
# from a terminal, which is what lets the release job produce the same bundle a
# developer gets locally.
#
#   make app     build/IronWire.app          — universal, ad-hoc signed
#   make dist    build/IronWire-macos.zip    — the release artifact
#   make run     build and launch it
#   make test    the Swift suite
#   make clean

APP        := IronWire
BUNDLE     := build/$(APP).app
CONFIG     := release

# One version for the app and the daemon it reports on. Reading it from the
# workspace manifest means a release cannot ship an app claiming to be a
# different one.
VERSION    := $(shell sed -n 's/^version = "\(.*\)"/\1/p' ../Cargo.toml | head -n1)

# Universal by default, because the release ships one bundle for both Macs.
# Override for a faster local loop:  make app ARCHS=
ARCHS      ?= --arch arm64 --arch x86_64

.PHONY: all app dist run test install uninstall clean

all: app

test:
	swift test

$(BUNDLE): $(shell find Sources Resources -type f) Package.swift Makefile
	@swift build -c $(CONFIG) $(ARCHS)
	@rm -rf "$(BUNDLE)"
	@mkdir -p "$(BUNDLE)/Contents/MacOS" "$(BUNDLE)/Contents/Resources"
	@cp "$$(swift build -c $(CONFIG) $(ARCHS) --show-bin-path)/$(APP)" "$(BUNDLE)/Contents/MacOS/$(APP)"
	@sed 's/__VERSION__/$(VERSION)/g' Resources/Info.plist > "$(BUNDLE)/Contents/Info.plist"
	@printf 'APPL????' > "$(BUNDLE)/Contents/PkgInfo"
	# Ad-hoc, which is enough to execute on Apple Silicon and enough to run
	# locally. It is *not* enough for a bundle that arrives from the internet:
	# Gatekeeper wants notarisation, which needs a certificate this project does
	# not have yet (docs/PACKAGING.md).
	@codesign --force --sign - --timestamp=none "$(BUNDLE)" >/dev/null 2>&1 || \
		echo "warning: could not ad-hoc sign $(BUNDLE)"
	@echo "built $(BUNDLE) ($(VERSION))"

app: $(BUNDLE)

dist: app
	@rm -f "build/$(APP)-macos.zip"
	# ditto rather than zip: it is the only one that preserves the bundle's
	# symlinks and the code signature inside it.
	@ditto -c -k --sequesterRsrc --keepParent "$(BUNDLE)" "build/$(APP)-macos.zip"
	@echo "built build/$(APP)-macos.zip"

run: app
	@# Restart rather than stack: a second copy would put two identical icons
	@# in the menu bar and poll the daemon twice.
	@pkill -x $(APP) 2>/dev/null || true
	@open "$(BUNDLE)"

# Where an installed copy lives. `/Applications` needs write access the user may
# not have; `make install PREFIX=~/Applications` is the per-user answer and
# needs no privileges at all.
PREFIX ?= /Applications

install: app
	@if [ ! -w "$(PREFIX)" ] && [ -d "$(PREFIX)" ]; then \
		echo "error: $(PREFIX) is not writable."; \
		echo "  install for just this user:  make -C macos install PREFIX=~/Applications"; \
		echo "  or for everyone:             sudo make -C macos install"; \
		exit 1; \
	fi
	@mkdir -p "$(PREFIX)"
	@pkill -x $(APP) 2>/dev/null || true
	@# Replaced whole rather than merged: a bundle with files from two builds in
	@# it is a bundle whose signature no longer describes its contents.
	@rm -rf "$(PREFIX)/$(APP).app"
	@ditto "$(BUNDLE)" "$(PREFIX)/$(APP).app"
	@echo "installed $(PREFIX)/$(APP).app ($(VERSION))"
	@echo "  open it with:  open \"$(PREFIX)/$(APP).app\""
	@echo "  it can start itself at login from its own menu"

uninstall:
	@pkill -x $(APP) 2>/dev/null || true
	@rm -rf "$(PREFIX)/$(APP).app"
	@echo "removed $(PREFIX)/$(APP).app"
	@echo "note: 'Open at Login' is a per-user login item; turn it off in the app"
	@echo "      before removing it, or in System Settings > General > Login Items"

clean:
	@rm -rf build .build
