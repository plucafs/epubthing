PREFIX ?= /usr/local
BINDIR ?= $(PREFIX)/bin
DATADIR ?= $(PREFIX)/share
DESTDIR ?=

APP_NAME = epubthing
DESKTOP_FILE = assets/$(APP_NAME).desktop
MIME_FILE = assets/$(APP_NAME)-mime.xml
ICON_FILE = assets/icons/$(APP_NAME).svg

.PHONY: all build install uninstall clean

all: build

build:
	cargo build --release

install:
	@test -f target/release/$(APP_NAME) || { echo "Binary not found. Run 'make build' first."; exit 1; }
	install -Dm755 target/release/$(APP_NAME) $(DESTDIR)$(BINDIR)/$(APP_NAME)
	install -Dm644 $(DESKTOP_FILE) $(DESTDIR)$(DATADIR)/applications/$(APP_NAME).desktop
	install -Dm644 $(MIME_FILE) $(DESTDIR)$(DATADIR)/mime/packages/$(APP_NAME)-mime.xml
	install -Dm644 $(ICON_FILE) $(DESTDIR)$(DATADIR)/icons/hicolor/scalable/apps/$(APP_NAME).svg
	-update-desktop-database $(DESTDIR)$(DATADIR)/applications 2>/dev/null || true
	-update-mime-database $(DESTDIR)$(DATADIR)/mime 2>/dev/null || true
	@echo ""
	@echo "Installed $(APP_NAME). Log out and back in for full desktop integration."

uninstall:
	rm -f $(DESTDIR)$(BINDIR)/$(APP_NAME)
	rm -f $(DESTDIR)$(DATADIR)/applications/$(APP_NAME).desktop
	rm -f $(DESTDIR)$(DATADIR)/mime/packages/$(APP_NAME)-mime.xml
	rm -f $(DESTDIR)$(DATADIR)/icons/hicolor/scalable/apps/$(APP_NAME).svg
	-update-desktop-database $(DESTDIR)$(DATADIR)/applications 2>/dev/null || true
	-update-mime-database $(DESTDIR)$(DATADIR)/mime 2>/dev/null || true
	@echo ""
	@echo "Uninstalled $(APP_NAME)."

clean:
	cargo clean
